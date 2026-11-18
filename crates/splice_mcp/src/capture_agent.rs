use crate::{
    Json,
    ipc::{receive, send, socket_path},
    object, paths, text,
    wire::{Args, bad, error_payload},
};
use apple_sys::{
    app, capture,
    ipc::{user, verify_peer},
};
use splice_core::history::StoreError;
use std::{
    env,
    ffi::OsString,
    fs::{self, OpenOptions, TryLockError},
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{self, Command},
    thread,
    time::{Duration, Instant},
};

const SOCK: &str = "agent.sock";
const IDLE: Duration = Duration::from_secs(60);
const STARTUP: Duration = Duration::from_secs(3);
const IO_TIMEOUT: Duration = Duration::from_secs(1);
const REPLY_TIMEOUT: Duration = Duration::from_secs(4);
const REQUEST_LIMIT: u64 = 64 * 1024;
const REPLY_LIMIT: u64 = 16 * 1024 * 1024;
const REQUIREMENT: &str = include_str!("../../../packaging/designated-requirement.txt");

mod permissions;
mod sources;

struct Identity {
    bundle: PathBuf,
    team: String,
}

pub(super) use permissions::ask as permission_report;

/// Hosts capture requests on the application thread of the installed app.
pub fn run() -> Result<(), StoreError> {
    let bundle = installed_bundle()?;
    if paths::app_bundle()
        .map_err(|error| identity(&error))?
        .as_ref()
        != Some(&bundle)
        || !app::has_launchd_parent()
    {
        return Err(identity(
            &"Capture needs the installed Splice.app launched by Launch Services.",
        ));
    }
    let team = verify_signature(&bundle)?;
    serve(
        &paths::state_dir().map_err(|error| transport(&error))?,
        &Identity { bundle, team },
    )
    .map_err(|error| transport(&error))
}

fn installed_bundle() -> Result<PathBuf, StoreError> {
    let bundle = app::user_home()
        .and_then(|home| home.canonicalize())
        .map_err(|error| identity(&error))?
        .join("Applications/Splice.app");
    if bundle.canonicalize().map_err(|error| identity(&error))? != bundle {
        return Err(identity(
            &"The installed Splice.app must resolve to its default location.",
        ));
    }
    Ok(bundle)
}

fn verify_signature(bundle: &Path) -> Result<String, StoreError> {
    let requirement = REQUIREMENT.trim();
    let display = Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=4", "-r-"])
        .arg(bundle)
        .output()
        .map_err(|error| identity(&error))?;
    let pinned = format!("designated => {requirement}");
    let stdout = String::from_utf8_lossy(&display.stdout);
    let stderr = String::from_utf8_lossy(&display.stderr);
    let lines = || stdout.lines().chain(stderr.lines());
    if !display.status.success() || !lines().any(|line| line == pinned) {
        return Err(identity(
            &"The installed app has a different designated requirement.",
        ));
    }
    let checked = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--test-requirement"])
        .arg(format!("={requirement}"))
        .arg(bundle)
        .output()
        .map_err(|error| identity(&error))?;
    if !checked.status.success() {
        return Err(identity(&String::from_utf8_lossy(&checked.stderr).trim()));
    }
    lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .filter(|team| !team.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| identity(&"The installed app did not report a signing team."))
}

fn connect(directory: &Path) -> io::Result<Option<UnixStream>> {
    match socket_path(directory, SOCK, false).and_then(UnixStream::connect) {
        Ok(stream) => {
            verify_peer(&stream)?;
            Ok(Some(stream))
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn launch() -> Result<(), StoreError> {
    let bundle = installed_bundle()?;
    verify_signature(&bundle)?;
    let mut home = OsString::from("HOME=");
    home.push(env::var_os("HOME").ok_or_else(|| transport(&"HOME is missing"))?);
    // ponytail: rely on Launch Services until SSH sessions without a GUI need a LaunchAgent.
    let result = Command::new("/usr/bin/open")
        .args(["-g", "-j", "-n", "-a"])
        .arg(bundle)
        .arg("--env")
        .arg(home)
        .args(["--args", "--capture-agent"])
        .output()
        .map_err(|error| transport(&error))?;
    if !result.status.success() {
        return Err(transport(&String::from_utf8_lossy(&result.stderr).trim()));
    }
    Ok(())
}

pub(super) fn request(method: &str, args: &Json) -> Result<Json, StoreError> {
    let directory = paths::state_dir()
        .map_err(|error| transport(&error))?
        .join("capture");
    let mut stream = if let Some(stream) = connect(&directory).map_err(|error| transport(&error))? {
        stream
    } else {
        launch()?;
        let deadline = Instant::now() + STARTUP;
        loop {
            if let Some(stream) = connect(&directory).map_err(|error| transport(&error))? {
                break stream;
            }
            if Instant::now() >= deadline {
                return Err(transport(
                    &"The installed capture agent did not open its socket.",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    };
    stream
        .set_read_timeout(Some(REPLY_TIMEOUT))
        .map_err(|error| transport(&error))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| transport(&error))?;
    send(
        &mut stream,
        &object(vec![("method", text(method)), ("args", args.clone())]),
    )
    .map_err(|error| transport(&error))?;
    let reply = receive(&stream, REPLY_LIMIT).map_err(|error| transport(&error))?;
    if let Some(error) = reply.field("error") {
        let code = match error.string("code")? {
            "CAPTURE_NOT_READY" => "CAPTURE_NOT_READY",
            "BAD_ARGS" => "BAD_ARGS",
            "TCC_DENIED" => "TCC_DENIED",
            "CAPTURE_SOURCE" => "CAPTURE_SOURCE",
            _ => return Err(transport(&"The capture agent returned an unknown error.")),
        };
        return Err(StoreError::new(
            code,
            error.string("message")?,
            error.string("fix")?,
        ));
    }
    reply
        .field("result")
        .cloned()
        .ok_or_else(|| transport(&"The capture agent returned an invalid reply."))
}

fn serve(state: &Path, identity: &Identity) -> io::Result<()> {
    fs::create_dir_all(state)?;
    let directory = state.join("capture");
    let path = socket_path(&directory, SOCK, true)?;
    let lock_path = directory.join("agent.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)?;
    let meta = fs::symlink_metadata(&lock_path)?;
    if !meta.is_file() || meta.uid() != user() || meta.nlink() != 1 {
        return Err(io::Error::other("The capture lock path is occupied."));
    }
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(()),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == user() => fs::remove_file(&path)?,
        Ok(_) => return Err(io::Error::other("The capture socket path is occupied.")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(error),
    }
    if !app::start_accessory() {
        return Err(io::Error::other("Could not start AppKit as an accessory."));
    }
    let listener = UnixListener::bind(&path)?;
    let result = listen(&listener, IDLE, app::wait_accessory, identity);
    let removed = fs::remove_file(path);
    result.and(removed)
}

fn listen(
    listener: &UnixListener,
    idle: Duration,
    wait: fn(Duration) -> bool,
    identity: &Identity,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let mut activity = Instant::now();
    while let Some(remaining) = idle.checked_sub(activity.elapsed()) {
        let delay = match listener.accept() {
            Ok((mut stream, _)) => {
                if serve_peer(&mut stream, identity).is_ok() {
                    activity = Instant::now();
                }
                Duration::ZERO
            }
            // ponytail: use a short AppKit wait until measured idle wakeups warrant a run-loop source.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                remaining.min(Duration::from_millis(50))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if !wait(delay) {
            break;
        }
    }
    Ok(())
}

fn serve_peer(stream: &mut UnixStream, identity: &Identity) -> io::Result<()> {
    verify_peer(stream)?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let request = receive(stream, REQUEST_LIMIT)?;
    let reply = match dispatch(&request, identity) {
        Ok(value) => object(vec![("result", value)]),
        Err(error) => object(vec![("error", error_payload(error))]),
    };
    send(stream, &reply)
}

fn dispatch(request: &Json, identity: &Identity) -> Result<Json, StoreError> {
    let args = request
        .field("args")
        .filter(|args| args.as_object().is_some())
        .ok_or_else(|| bad("args", "an object"))?;
    match request.string("method")? {
        "ping" => Ok(object(vec![(
            "pid",
            Json::Unsigned(u64::from(process::id())),
        )])),
        "permissions" => permissions::command(args, identity),
        "record.start" => {
            let snapshot = permissions::snapshot();
            if args.flag("dryRun")? {
                let mut result = permissions::report(identity, &snapshot);
                let sources = capture::sources(permissions::screen_granted(&snapshot))
                    .map_err(capture_source_error)?;
                result.push(("sources", sources::payload(&sources)));
                return Ok(object(result));
            }
            permissions::required(args, &snapshot)?;
            Err(capture_not_ready())
        }
        "record.stop" => Err(capture_not_ready()),
        _ => Err(bad("method", "permissions, record.start or record.stop")),
    }
}

fn capture_not_ready() -> StoreError {
    StoreError::new(
        "CAPTURE_NOT_READY",
        "The signed capture agent is reachable. Recording is under development.",
        "Use a build with the native recorder.",
    )
}

fn capture_source_error(error: capture::CaptureError) -> StoreError {
    if error.permission_denied {
        return permissions::screen_denied();
    }
    StoreError::new(
        "CAPTURE_SOURCE",
        &error.to_string(),
        "Check that the requested display or window is available and retry.",
    )
}

fn identity(reason: &dyn std::fmt::Display) -> StoreError {
    StoreError::new(
        "CAPTURE_IDENTITY",
        &reason.to_string(),
        "Run make install and use splice record to start the installed agent.",
    )
}

fn transport(reason: &dyn std::fmt::Display) -> StoreError {
    StoreError::new(
        "CAPTURE_AGENT_UNAVAILABLE",
        &reason.to_string(),
        "Use a logged-in macOS session and retry the capture request.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_agent_round_trips_and_exits_when_idle() {
        let path = PathBuf::from(format!("/tmp/splice-agent-test-{}.sock", process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let mut peer = UnixStream::connect(&path).unwrap();
        peer.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        send(
            &mut peer,
            &object(vec![("method", text("ping")), ("args", object(Vec::new()))]),
        )
        .unwrap();
        let timeout = Duration::from_millis(25);
        let started = Instant::now();
        listen(
            &listener,
            timeout,
            |delay| {
                thread::sleep(delay);
                true
            },
            &Identity {
                bundle: path.clone(),
                team: "test".to_owned(),
            },
        )
        .unwrap();
        assert!(started.elapsed() >= timeout);
        let reply = receive(&peer, REPLY_LIMIT).unwrap();
        assert_eq!(
            reply
                .field("result")
                .and_then(|reply| reply.field("pid"))
                .and_then(Json::as_u64),
            Some(u64::from(process::id())),
        );
        fs::remove_file(path).unwrap();
    }
}
