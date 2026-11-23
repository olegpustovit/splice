use crate::record_options::{self, DeviceInfo, Options, SystemAudio};
use crate::{
    Json,
    ipc::{receive, send, socket_path},
    object, paths, text,
    wire::{Args, bad, error_payload},
};
use apple_sys::{
    app,
    capture::{self, Target},
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
const TAKE_REPLY_TIMEOUT: Duration = Duration::from_secs(20);
const REQUEST_LIMIT: u64 = 64 * 1024;
const REPLY_LIMIT: u64 = 16 * 1024 * 1024;
const REQUIREMENT: &str = include_str!("../../../packaging/designated-requirement.txt");

mod jobs;
pub(crate) use jobs::CAPTURE_FPS;
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
    exchange(&mut dial()?, method, args)
}

/// The running capture agent's state for take `id`, without launching an agent (M16): `None`
/// when no agent runs, `Err(code)` when it cannot answer or failed the take.
pub(super) fn take_state(id: splice_model::JobId) -> Option<Result<String, &'static str>> {
    let unavailable = Some(Err("CAPTURE_AGENT_UNAVAILABLE"));
    let Ok(state) = paths::state_dir() else {
        return unavailable;
    };
    let mut stream = match connect(&state.join("capture")) {
        Ok(Some(stream)) => stream,
        Ok(None) => return None,
        Err(_) => return unavailable,
    };
    let args = object(vec![("jobId", text(&id.to_string()))]);
    Some(
        exchange(&mut stream, "take.status", &args)
            .and_then(|status| status.string("state").map(str::to_owned))
            .map_err(|error| error.code),
    )
}

pub(super) fn begin(args: &Json) -> Result<(Json, UnixStream), StoreError> {
    let mut stream = dial()?;
    let result = exchange(&mut stream, "take.begin", args)?;
    Ok((result, stream))
}

fn dial() -> Result<UnixStream, StoreError> {
    let directory = paths::state_dir()
        .map_err(|error| transport(&error))?
        .join("capture");
    if let Some(stream) = connect(&directory).map_err(|error| transport(&error))? {
        return Ok(stream);
    }
    launch()?;
    let deadline = Instant::now() + STARTUP;
    loop {
        if let Some(stream) = connect(&directory).map_err(|error| transport(&error))? {
            return Ok(stream);
        }
        if Instant::now() >= deadline {
            return Err(transport(
                &"The installed capture agent did not open its socket.",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn exchange(stream: &mut UnixStream, method: &str, args: &Json) -> Result<Json, StoreError> {
    stream
        .set_read_timeout(Some(if method.starts_with("take.") {
            TAKE_REPLY_TIMEOUT
        } else {
            REPLY_TIMEOUT
        }))
        .map_err(|error| transport(&error))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| transport(&error))?;
    send(
        stream,
        &object(vec![("method", text(method)), ("args", args.clone())]),
    )
    .map_err(|error| transport(&error))?;
    let reply = receive(stream, REPLY_LIMIT).map_err(|error| transport(&error))?;
    if let Some(error) = reply.field("error") {
        let code = match error.string("code")? {
            "BAD_ARGS" => "BAD_ARGS",
            "NO_DEVICE" => "NO_DEVICE",
            "NOT_RECORDING" => "NOT_RECORDING",
            "TCC_DENIED" => "TCC_DENIED",
            "CAPTURE_SOURCE" => "CAPTURE_SOURCE",
            "CAPTURE_FAILED" => "CAPTURE_FAILED",
            "BUSY" => "BUSY",
            "NOT_FOUND" => "NOT_FOUND",
            "IO" => "IO",
            _ => return Err(transport(&"The capture agent returned an unknown error.")),
        };
        let mut failure = StoreError::new(code, error.string("message")?, error.string("fix")?);
        // Keep the broken rule (param, got, expected) the agent computed.
        if let (Some(param), Some(expected)) = (
            error.field("param").and_then(Json::as_str),
            error.field("expected").and_then(Json::as_str),
        ) {
            let rule = splice_core::commands::CommandError::rule(
                code,
                param,
                error.field("got").cloned(),
                expected,
                &failure.fix,
            );
            failure.rule = StoreError::from(rule).rule;
        }
        return Err(failure);
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
    let mut jobs = jobs::Jobs::default();
    let mut tick = activity;
    loop {
        let now = Instant::now();
        if now >= tick {
            let was_active = jobs.active();
            jobs.tick(now);
            if was_active {
                activity = Instant::now();
            }
            tick = now + jobs::POLL;
        }
        let remaining = match idle.checked_sub(activity.elapsed()) {
            Some(remaining) => remaining,
            None if jobs.has_sealed_take() => {
                activity = Instant::now();
                idle
            }
            None => break,
        };
        let delay = match listener.accept() {
            Ok((stream, _)) => {
                if serve_peer(stream, identity, &mut jobs).is_ok() {
                    activity = Instant::now();
                }
                Duration::ZERO
            }
            // ponytail: use a short AppKit wait until measured idle wakeups warrant a run-loop source.
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if jobs.active() {
                    tick.saturating_duration_since(Instant::now())
                } else {
                    remaining.min(Duration::from_millis(50))
                }
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

fn serve_peer(
    mut stream: UnixStream,
    identity: &Identity,
    jobs: &mut jobs::Jobs,
) -> io::Result<()> {
    verify_peer(&stream)?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let request = receive(&stream, REQUEST_LIMIT)?;
    let result = dispatch(&request, identity, jobs);
    let owns_take = result.is_ok() && request.string("method").ok() == Some("take.begin");
    let reply = match result {
        Ok(value) => object(vec![("result", value)]),
        Err(error) => object(vec![("error", error_payload(error))]),
    };
    let written = send(&mut stream, &reply);
    if owns_take {
        // The open socket bounds capture to the lifetime of the project writer.
        let ready = written.and_then(|()| stream.set_nonblocking(true));
        jobs.attach_peer(ready.is_ok().then_some(stream));
        return ready;
    }
    written
}

fn dispatch(
    request: &Json,
    identity: &Identity,
    jobs: &mut jobs::Jobs,
) -> Result<Json, StoreError> {
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
            let mut result = permissions::report(identity, &snapshot);
            let granted = permissions::screen_granted(&snapshot);
            let sources = capture::sources(granted).map_err(capture_source_error)?;
            // A dry run with a window source checks every window now (T46.6), so a typo fails
            // here with the open windows instead of after the countdown.
            if granted
                && (args.field("window").is_some()
                    || args.field("source").is_some_and(|source| {
                        source.field("window").is_some() || source.field("windows").is_some()
                    }))
            {
                sources::resolve(&record_options::source(args)?, &sources)?;
            }
            result.push(("sources", sources::payload(&sources)));
            let devices: Vec<Json> = apple_sys::capture::devices()
                .into_iter()
                .map(|device| {
                    object(vec![
                        ("kind", text(&device.kind)),
                        ("id", text(&device.id)),
                        ("name", text(&device.name)),
                        ("default", Json::Bool(device.default)),
                    ])
                })
                .collect();
            result.push(("devices", Json::Array(devices)));
            Ok(object(result))
        }
        "take.check" => {
            permissions::required(args, &permissions::snapshot())?;
            Ok(object(Vec::new()))
        }
        "take.begin" => {
            let options = Options::parse(args)?;
            permissions::require_options(&options, &permissions::snapshot())?;
            let plan = plan(&options)?;
            jobs.begin(Path::new(args.string("package")?), options, plan)
        }
        method @ ("take.status" | "take.stop" | "take.cancel" | "take.pause" | "take.resume"
        | "take.mark") => jobs.command(method, args),
        _ => Err(bad("method", "a capture agent command")),
    }
}

/// Resolves devices, app audio, the window (resized when asked) and the capture geometry
/// before the countdown, so the start receipt states what the take records. A track with no
/// device would otherwise start and fail seconds later with no samples.
fn plan(options: &Options) -> Result<jobs::Plan, StoreError> {
    let devices: Vec<DeviceInfo> = capture::devices()
        .into_iter()
        .map(|device| DeviceInfo {
            kind: device.kind,
            id: device.id,
            name: device.name,
            default: device.default,
        })
        .collect();
    let mic = record_options::resolve_device("mic", &options.mic, &devices)?;
    let camera = record_options::resolve_device("camera", &options.camera, &devices)?;
    let listed = capture::sources(true).map_err(capture_source_error)?;
    let audio_apps = match &options.system_audio {
        SystemAudio::Apps(names) => {
            let running: Vec<(String, String)> = listed
                .iter()
                .filter_map(|source| match source {
                    capture::Source::Application { bundle, name, .. } => {
                        Some((bundle.clone(), name.clone()))
                    }
                    _ => None,
                })
                .collect();
            record_options::resolve_apps(names, &running)?
        }
        _ => Vec::new(),
    };
    let targets = sources::targets(&options.source)?;
    let target = targets[0];
    let resized = match (options.size, target) {
        (Some(size), Target::Window(id)) => Some((size, resize(&listed, id, size)?)),
        _ => None,
    };
    let mut geometry = capture::geometry(target).map_err(capture_source_error)?;
    // The window server can lag the Accessibility resize by a frame or two.
    if let Some((_, points)) = resized {
        for _ in 0..20 {
            if (geometry.rect.width - points[0]).abs() < 2.0
                && (geometry.rect.height - points[1]).abs() < 2.0
            {
                break;
            }
            thread::sleep(Duration::from_millis(25));
            geometry = capture::geometry(target).map_err(capture_source_error)?;
        }
    }
    let windows = targets[1..]
        .iter()
        .map(|&window| {
            Ok((
                window,
                capture::geometry(window).map_err(capture_source_error)?,
            ))
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(jobs::Plan {
        target,
        windows,
        mic,
        camera,
        audio_apps,
        geometry: Some(geometry),
        resized,
    })
}

/// Resizes window `id` to `size` points; the size the window then has (apps clamp it).
fn resize(listed: &[capture::Source], id: u32, size: [f64; 2]) -> Result<[f64; 2], StoreError> {
    let (pid, rect) = listed
        .iter()
        .find_map(|source| match source {
            capture::Source::Window {
                id: window,
                pid,
                rect,
                ..
            } if *window == id => Some((*pid, *rect)),
            _ => None,
        })
        .ok_or_else(|| {
            StoreError::new(
                "CAPTURE_SOURCE",
                "The window to resize is no longer shareable.",
                "Bring the window on screen and retry, or pick another with record.start {dryRun:true, titles:true}.",
            )
        })?;
    match capture::resize_window(pid, rect, size) {
        Ok(frame) => Ok([frame.width, frame.height]),
        Err(capture::ResizeError::Untrusted) => Err(permissions::accessibility_denied()),
        Err(capture::ResizeError::NotFound) => Err(StoreError::new(
            "CAPTURE_SOURCE",
            "Accessibility does not list the window to resize.",
            "Retry once the window is on screen, or record without size and resize it yourself.",
        )),
        Err(capture::ResizeError::Refused) => Err(StoreError::new(
            "CAPTURE_SOURCE",
            "The app refused to resize its window.",
            "Record without size (this app's windows cannot be resized), or pick another window.",
        )),
    }
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
