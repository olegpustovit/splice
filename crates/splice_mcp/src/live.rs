use crate::{
    Json,
    ipc::{self, receive, send},
    paths,
};
use apple_sys::ipc::{user, verify_peer};
use splice_core::history::{Store, StoreError};
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
const VIEWER: &str = "v";
const HOLDER: &str = "h";
const HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) mod proxy;
mod viewer;
pub use viewer::{ViewCommand, ViewSelection, ViewTab, Viewer, ViewerZoom};

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
    project: PathBuf,
    incoming: Option<Receiver<Request>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    /// Publishes the project's socket and wakes the viewer when a request arrives.
    pub fn start(store: &Store, state: &Path, wake: fn() -> bool) -> io::Result<Self> {
        let (send, recv) = mpsc::sync_channel(1);
        let mut server = Self::listen(
            store.path(),
            state,
            VIEWER,
            Box::new(move |request| {
                send.send(request).map_err(io::Error::other)?;
                wake();
                Ok(())
            }),
        )?;
        server.incoming = Some(recv);
        Ok(server)
    }

    /// Publishes a headless writer's handoff socket. `sink` receives each handoff request.
    pub(crate) fn hold(
        project: &Path,
        state: &Path,
        sink: Box<dyn FnMut(Request) -> io::Result<()> + Send>,
    ) -> io::Result<Self> {
        Self::listen(project, state, HOLDER, sink)
    }

    /// The package this socket serves.
    pub(crate) fn project(&self) -> &Path {
        &self.project
    }

    fn listen(
        project: &Path,
        state: &Path,
        kind: &str,
        mut sink: Box<dyn FnMut(Request) -> io::Result<()> + Send>,
    ) -> io::Result<Self> {
        fs::create_dir_all(state)?;
        let path = socket_path(state, project, kind, true)?;
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
                    let _ = serve_peer(stream, &mut *sink);
                }
            })
            .inspect_err(|_| {
                let _ = fs::remove_file(&path);
            })?;
        Ok(Self {
            path,
            project: project.to_path_buf(),
            incoming: None,
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
    let path = match socket_path(&state, project, VIEWER, false) {
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

/// Asks a headless writer (an agent's `splice mcp`) to hand this package over and waits until
/// it has released the writer lock. Returns false when no headless writer holds the package.
pub fn handoff(state: &Path, project: &Path) -> Result<bool, StoreError> {
    let locked = |message: &dyn std::fmt::Display| {
        StoreError::new(
            "LOCKED",
            &message.to_string(),
            "Close the other writer, then open the project again.",
        )
    };
    let path = match socket_path(state, project, HOLDER, false) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(locked(&error)),
    };
    let mut stream = match UnixStream::connect(path) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(locked(&error)),
    };
    let reply = verify_peer(&stream)
        .and_then(|()| stream.set_write_timeout(Some(HANDOFF_TIMEOUT)))
        .and_then(|()| stream.set_read_timeout(Some(HANDOFF_TIMEOUT)))
        .and_then(|()| {
            send(
                &mut stream,
                &crate::object(vec![("command", crate::text("handoff"))]),
            )
        })
        .and_then(|()| receive(&stream, REQUEST_LIMIT))
        .map_err(|error| locked(&error))?;
    match reply.field("error") {
        None => Ok(true),
        Some(error) => Err(StoreError::new(
            "LOCKED",
            error
                .field("message")
                .and_then(Json::as_str)
                .unwrap_or("The agent holding this project refused to hand it over."),
            error
                .field("fix")
                .and_then(Json::as_str)
                .unwrap_or("Close the other writer, then open the project again."),
        )),
    }
}

fn socket_path(state: &Path, project: &Path, kind: &str, create: bool) -> io::Result<PathBuf> {
    let project = fs::metadata(project)?;
    let name = format!("{kind}-{:x}-{:x}", project.dev(), project.ino());
    ipc::socket_path(&state.join("live"), &name, create)
}
