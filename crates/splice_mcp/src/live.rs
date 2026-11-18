use crate::{
    Json,
    ipc::{self, receive, send},
    paths,
};
use apple_sys::ipc::{user, verify_peer};
use splice_core::history::Store;
use std::{
    fs, io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const REQUEST_LIMIT: u64 = 17 * 1024 * 1024;
const REPLY_LIMIT: u64 = 128 * 1024 * 1024;

pub(super) mod proxy;
mod viewer;
pub use viewer::{ViewCommand, Viewer};

pub(super) fn disconnected(reason: &dyn std::fmt::Display) -> splice_core::history::StoreError {
    splice_core::history::StoreError::new(
        "VIEWER_DISCONNECTED",
        &reason.to_string(),
        "Call project.info to reopen the project. Read its revision before retrying an edit.",
    )
}

/// A request awaiting dispatch by the viewer's event loop.
pub struct Request {
    /// Internal viewer command.
    pub body: Json,
    reply: Sender<Json>,
}

impl Request {
    /// Releases the socket worker with the result.
    pub fn reply(self, result: Json) {
        let _ = self.reply.send(result);
    }
}

/// A listener tied to one writer-locked package.
pub struct Server {
    path: PathBuf,
    incoming: Option<Receiver<Request>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    /// Publishes the project's socket and wakes the viewer when a request arrives.
    pub fn start(store: &Store, state: &Path, wake: fn() -> bool) -> io::Result<Self> {
        fs::create_dir_all(state)?;
        let path = socket_path(state, store.path(), true)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() && meta.uid() == user() => {
                fs::remove_file(&path)?
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Viewer socket path is occupied",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let (send, recv) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        // ponytail: serialize requests with one worker. Move long native reads off the
        // window thread when measured tool latency disrupts interaction.
        let worker = thread::Builder::new()
            .name("splice-viewer-ipc".into())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let Ok((stream, _)) = listener.accept() else {
                        break;
                    };
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = serve_peer(stream, &mut |request| {
                        send.send(request).map_err(io::Error::other)?;
                        wake();
                        Ok(())
                    });
                }
            })
            .inspect_err(|_| {
                let _ = fs::remove_file(&path);
            })?;
        Ok(Self {
            path,
            incoming: Some(recv),
            stop,
            worker: Some(worker),
        })
    }

    /// Takes an already received request without waiting for a client.
    pub fn next_request(&self) -> Option<Request> {
        self.incoming.as_ref()?.try_recv().ok()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        drop(self.incoming.take());
        let woke = UnixStream::connect(&self.path).is_ok();
        if let Some(worker) = self.worker.take()
            && woke
        {
            let _ = worker.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Calls the viewer that holds this package, or returns `None` before sending if none is listening.
pub fn request(project: &Path, body: &Json) -> io::Result<Option<Json>> {
    let Ok(state) = paths::state_dir() else {
        return Ok(None);
    };
    let path = match socket_path(&state, project, false) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    verify_peer(&stream)?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    send(&mut stream, body)?;
    receive(&stream, REPLY_LIMIT).map(Some)
}

fn serve_peer(
    mut stream: UnixStream,
    dispatch: &mut dyn FnMut(Request) -> io::Result<()>,
) -> io::Result<()> {
    verify_peer(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let body = receive(&stream, REQUEST_LIMIT)?;
    let (reply, recv) = mpsc::channel();
    dispatch(Request { body, reply })?;
    let result = recv.recv().map_err(io::Error::other)?;
    send(&mut stream, &result)
}

fn socket_path(state: &Path, project: &Path, create: bool) -> io::Result<PathBuf> {
    let project = fs::metadata(project)?;
    let name = format!("v-{:x}-{:x}", project.dev(), project.ino());
    ipc::socket_path(&state.join("live"), &name, create)
}
