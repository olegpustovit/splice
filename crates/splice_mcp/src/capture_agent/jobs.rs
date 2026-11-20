use super::{capture_source_error, permissions};
use crate::{
    Json, object,
    record_options::{self, DeviceInfo, Options, SystemAudio},
    text,
    wire::{Args, bad, display, failure},
};
use apple_sys::{
    capture::{Geometry, Recording, RecordingStats, Target},
    screenshot,
};
use splice_capture::EventLog;
use splice_core::history::StoreError;
use splice_model::JobId;
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const POLL: Duration = Duration::from_millis(12);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
/// The most screen frames per second a take records (ScreenCaptureKit delivers on change).
pub(super) const CAPTURE_FPS: u32 = 60;

/// What `take.begin` resolved before the countdown: the exact source and devices the take
/// records, echoed back so an agent sees what it will get.
pub(super) struct Plan {
    pub target: Target,
    /// The other windows of a multi-window take (T46.6), in file order (`window-1.mov`, …),
    /// with their capture geometry.
    pub windows: Vec<(Target, Geometry)>,
    pub mic: Option<DeviceInfo>,
    pub camera: Option<DeviceInfo>,
    /// Apps whose sound `systemAudio:{apps}` records: bundle ID and name.
    pub audio_apps: Vec<(String, String)>,
    pub geometry: Option<Geometry>,
    /// `size` asked for and the window size Accessibility then reported, in points.
    pub resized: Option<([f64; 2], [f64; 2])>,
}

#[derive(Default)]
pub(super) struct Jobs {
    jobs: Vec<Job>,
    last_id: u64,
}

struct Job {
    id: JobId,
    options: Options,
    plan: Plan,
    /// Recorded seconds at which the take stops, or pauses with `then_pause`.
    limit: Option<f64>,
    then_pause: bool,
    state: State,
    stage: Option<Stage>,
    stats: RecordingStats,
    peer: Option<UnixStream>,
    /// Writing is paused; the take's clock excludes the pause.
    paused: bool,
    /// Markers dropped during the take: take seconds and names.
    markers: Vec<(f64, String)>,
    /// The stage's size on disk and when it was measured: status requests come every 40 ms
    /// and each measurement lists the stage directories.
    bytes: std::cell::Cell<(u64, Option<Instant>)>,
}

/// How long a measured stage size answers status requests.
const BYTES_FRESH: Duration = Duration::from_millis(500);

enum State {
    Pending(Instant),
    Running {
        since: Instant,
        recording: Recording,
        log: Option<EventLog>,
    },
    Ready,
    Cancelled,
    Failed(StoreError),
}

struct Stage(PathBuf);

impl Jobs {
    pub(super) fn active(&self) -> bool {
        self.jobs.last().is_some_and(Job::active)
    }

    pub(super) fn has_sealed_take(&self) -> bool {
        self.jobs.iter().any(|job| {
            matches!(job.state, State::Ready)
                && job.stage.as_ref().is_some_and(|stage| stage.0.exists())
        })
    }

    pub(super) fn begin(
        &mut self,
        package: &Path,
        options: Options,
        plan: Plan,
    ) -> Result<Json, StoreError> {
        if self.active() {
            return Err(failure(
                "BUSY",
                "A screen capture is already running.",
                "Stop it with record.stop or discard it with jobs.cancel.",
            ));
        }
        let ready = Instant::now()
            .checked_add(options.countdown)
            .ok_or_else(|| bad("countdown", "seconds in the supported range"))?;
        let clock = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |age| age.as_nanos() as u64);
        self.last_id = clock.max(self.last_id.saturating_add(1));
        let id = JobId::from_serial(self.last_id);
        let mut job = Job {
            id,
            limit: options.duration,
            then_pause: options.then_pause,
            options,
            plan,
            state: State::Pending(ready),
            stage: Some(Stage::create(package, id)?),
            stats: RecordingStats::default(),
            peer: None,
            paused: false,
            markers: Vec::new(),
            bytes: Default::default(),
        };
        job.tick(Instant::now());
        let receipt = job.status()?;
        self.jobs.push(job);
        Ok(receipt)
    }

    pub(super) fn tick(&mut self, now: Instant) {
        for job in &mut self.jobs {
            if let Some(peer) = &mut job.peer
                && !matches!(peer.read(&mut [0]), Err(error) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted))
            {
                job.discard(State::Cancelled);
            }
        }
        if let Some(job) = self.jobs.last_mut() {
            job.tick(now);
        }
    }

    pub(super) fn attach_peer(&mut self, peer: Option<UnixStream>) {
        let job = self.jobs.last_mut().expect("begin succeeded");
        if peer.is_some() {
            job.peer = peer;
        } else {
            job.discard(State::Cancelled);
        }
    }

    pub(super) fn command(&mut self, method: &str, args: &Json) -> Result<Json, StoreError> {
        let id = args
            .string("jobId")?
            .parse::<JobId>()
            .map_err(|_| bad("jobId", "a recording job ID"))?;
        let job = self
            .jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or_else(|| {
                failure(
                    "NOT_FOUND",
                    &format!("No recording {id} is running or retained."),
                    "Pass the jobId from record.start.",
                )
            })?;
        match method {
            "take.status" => (),
            "take.stop" => {
                if matches!(job.state, State::Pending(_)) {
                    return Err(failure(
                        "BUSY",
                        "This capture is counting down.",
                        "Wait for capture to start or use jobs.cancel to discard it.",
                    ));
                }
                if matches!(job.state, State::Running { .. }) {
                    job.finish();
                }
            }
            "take.cancel" if let Some(stage) = &job.stage => {
                job.state = State::Cancelled;
                stage.remove()?;
                job.stage = None;
                job.peer = None;
            }
            "take.cancel" => (),
            "take.pause" | "take.resume" => {
                let State::Running { recording, .. } = &mut job.state else {
                    return Err(failure(
                        "NOT_RECORDING",
                        "Only a running take pauses or resumes.",
                        "Check jobs.status; start a new take with record.start.",
                    ));
                };
                job.paused = method == "take.pause";
                recording.set_paused(job.paused);
                // record.start {resume, duration}: exactly this many more recorded seconds.
                if !job.paused
                    && let Some(more) = args.field("duration")
                {
                    let more = more
                        .as_f64()
                        .filter(|seconds| *seconds > 0.0 && seconds.is_finite())
                        .ok_or_else(|| bad("duration", "positive seconds of recorded time"))?;
                    job.limit = Some(job.stats.elapsed_seconds + more);
                    job.then_pause = record_options::then(args, true)?;
                }
            }
            "take.mark" => {
                if !matches!(job.state, State::Running { .. }) {
                    return Err(failure(
                        "NOT_RECORDING",
                        "Markers go on a running take.",
                        "Add markers after recording with marker.add.",
                    ));
                }
                let name = args
                    .optional_string("marker")?
                    .unwrap_or("Marker")
                    .to_owned();
                job.markers.push((job.stats.elapsed_seconds, name));
            }
            _ => unreachable!("validated capture method"),
        }
        job.status()
    }
}

impl Job {
    fn active(&self) -> bool {
        matches!(self.state, State::Pending(_) | State::Running { .. })
    }

    fn tick(&mut self, now: Instant) {
        let result = match &self.state {
            State::Pending(ready) if *ready <= now => self.start(),
            State::Running { .. } => self.poll(now),
            _ => return,
        };
        if let Err(error) = result {
            self.discard(State::Failed(error));
        }
    }

    fn start(&mut self) -> Result<(), StoreError> {
        permissions::require_options(&self.options, &permissions::snapshot())?;
        let target = self.plan.target;
        let stage = self.stage.as_ref().expect("pending job has a stage");
        if self.options.still {
            let png = screenshot::png(target).map_err(capture_source_error)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(stage.0.join("screen.png"))
                .map_err(storage)?;
            file.write_all(&png).map_err(storage)?;
            file.sync_all().map_err(storage)?;
            stage.sync()?;
            self.state = State::Ready;
        } else {
            apple_sys::capture::exclude(&self.options.exclude, self.options.hide_desktop);
            let apps: Vec<String> = self
                .plan
                .audio_apps
                .iter()
                .map(|(bundle, _)| bundle.clone())
                .collect();
            let mut targets = vec![target];
            targets.extend(self.plan.windows.iter().map(|(window, _)| *window));
            let recording = Recording::start(
                &stage.0,
                &targets,
                self.plan.mic.as_ref().map(|device| device.id.as_str()),
                self.options.system_audio.on(),
                &apps,
                self.plan.camera.as_ref().map(|device| device.id.as_str()),
                self.options.events,
            )
            .map_err(native)?;
            self.state = State::Running {
                since: Instant::now(),
                recording,
                log: None,
            };
        }
        Ok(())
    }

    fn poll(&mut self, now: Instant) -> Result<(), StoreError> {
        let State::Running {
            since,
            recording,
            log,
        } = &mut self.state
        else {
            unreachable!("running job")
        };
        self.stats = recording.status().map_err(native)?;
        if self.stats.video_frames == 0 && now.duration_since(*since) >= FIRST_FRAME_TIMEOUT {
            return Err(recording_error("No complete screen frame arrived."));
        }
        if self.options.events {
            let stage = self.stage.as_ref().expect("running job has a stage");
            poll_events(stage, recording, log, &self.plan.windows)?;
        }
        // Within one poll (12 ms) of the limit: under a frame at 60 fps.
        match record_options::at_limit(
            self.stats.elapsed_seconds,
            self.limit,
            self.paused,
            self.then_pause,
        ) {
            Some(true) => {
                self.limit = None;
                recording.set_paused(true);
                self.paused = true;
            }
            Some(false) => {
                self.limit = None;
                self.finish();
            }
            None => {}
        }
        Ok(())
    }

    fn finish(&mut self) {
        let State::Running {
            mut recording,
            mut log,
            ..
        } = std::mem::replace(&mut self.state, State::Ready)
        else {
            unreachable!("running job")
        };
        let stage = self.stage.as_ref().expect("running job has a stage");
        let result = recording.finish().map_err(native).and_then(|stats| {
            self.stats = stats;
            if self.options.events {
                poll_events(stage, &mut recording, &mut log, &self.plan.windows)?;
                let log = log
                    .take()
                    .ok_or_else(|| recording_error("The input clock never started."))?;
                log.finish(recording.dropped_input()).map_err(storage)?;
            }
            stage.sync()
        });
        drop(recording);
        if let Err(error) = result {
            self.discard(State::Failed(error));
        }
    }

    fn discard(&mut self, state: State) {
        self.state = state;
        self.stage = None;
        self.peer = None;
    }

    fn status(&self) -> Result<Json, StoreError> {
        let state = match &self.state {
            State::Pending(_) => "pending",
            State::Running { .. } if self.paused => "paused",
            State::Running { .. } => "recording",
            State::Ready => "ready",
            State::Cancelled => "cancelled",
            State::Failed(error) => return Err(failure(error.code, &error.message, &error.fix)),
        };
        let progress = if state == "ready" {
            1.0
        } else {
            self.limit.map_or(0.0, |limit| {
                (self.stats.elapsed_seconds / limit).clamp(0.0, 1.0)
            })
        };
        let bytes = match self.bytes.get() {
            (bytes, Some(at)) if at.elapsed() < BYTES_FRESH && state == "recording" => bytes,
            _ => {
                let bytes = self.stage.as_ref().map_or(Ok(0), Stage::bytes)?;
                self.bytes.set((bytes, Some(Instant::now())));
                bytes
            }
        };
        let mut fields = vec![
            ("jobId", display(&self.id)),
            ("kind", text("record")),
            ("state", text(state)),
            ("progress", Json::Float(progress)),
            ("elapsed", Json::Float(self.stats.elapsed_seconds)),
            ("bytes", Json::Unsigned(bytes)),
            ("videoFrames", Json::Unsigned(self.stats.video_frames)),
            ("droppedFrames", Json::Unsigned(self.stats.dropped_frames)),
            ("settings", settings(&self.options, &self.plan)),
        ];
        if let Some(limit) = self.limit {
            fields.push(("stopsAt", Json::Float(limit)));
            fields.push(("then", text(if self.then_pause { "pause" } else { "stop" })));
        }
        if !self.markers.is_empty() {
            fields.push((
                "markers",
                Json::Array(
                    self.markers
                        .iter()
                        .map(|(t, name)| {
                            object(vec![("seconds", Json::Float(*t)), ("name", text(name))])
                        })
                        .collect(),
                ),
            ));
        }
        if let State::Ready = self.state {
            let path = &self.stage.as_ref().expect("ready job has a stage").0;
            fields.push(("staged", display(&path.display())));
        }
        Ok(object(fields))
    }
}

impl Stage {
    fn create(package: &Path, id: JobId) -> Result<Self, StoreError> {
        if !package.is_absolute()
            || package
                .extension()
                .is_none_or(|extension| extension != "splice")
            || package.canonicalize().map_err(storage)? != package
            || !package.is_dir()
        {
            return Err(bad(
                "package",
                "the canonical path of an open .splice directory",
            ));
        }
        let takes = package.join("takes");
        match fs::create_dir(&takes) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(storage(error)),
        }
        if !fs::symlink_metadata(&takes).map_err(storage)?.is_dir() {
            return Err(bad("takes", "a real directory within the project package"));
        }
        let stage = crate::record_publish::stage_path(package, id);
        DirBuilder::new()
            .mode(0o700)
            .create(&stage)
            .map_err(storage)?;
        Ok(Self(stage))
    }

    fn sync(&self) -> Result<(), StoreError> {
        for entry in fs::read_dir(&self.0).map_err(storage)? {
            let entry = entry.map_err(storage)?;
            File::open(entry.path())
                .and_then(|file| file.sync_all())
                .map_err(storage)?;
        }
        File::open(&self.0)
            .and_then(|dir| dir.sync_all())
            .map_err(storage)
    }

    fn bytes(&self) -> Result<u64, StoreError> {
        bytes_in(&self.0, true)
    }

    fn remove(&self) -> Result<(), StoreError> {
        match fs::remove_dir_all(&self.0) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage(error)),
        }
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}

fn bytes_in(path: &Path, include_cursors: bool) -> Result<u64, StoreError> {
    let mut bytes = 0u64;
    for entry in fs::read_dir(path).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(storage(error)),
        };
        if meta.is_file() {
            bytes = bytes.saturating_add(meta.len());
        } else if include_cursors && meta.is_dir() && entry.file_name() == "cursors" {
            bytes = bytes.saturating_add(bytes_in(&entry.path(), false)?);
        }
    }
    Ok(bytes)
}

/// Every setting the take records with, as resolved (T46.1): source bounds and pixel size,
/// frame rate, devices, app audio, countdown and timing.
pub(super) fn settings(options: &Options, plan: &Plan) -> Json {
    let pair = |[a, b]: [f64; 2]| Json::Array(vec![Json::Float(a), Json::Float(b)]);
    let (kind, id) = match plan.target {
        Target::Window(id) if !plan.windows.is_empty() => ("windows", text(&format!("win_{id}"))),
        Target::Display(0) => ("display", text("main")),
        Target::Display(id) => ("display", display(&id)),
        Target::Window(id) => ("window", text(&format!("win_{id}"))),
        Target::Area { display: 0, .. } => ("area", text("main")),
        Target::Area { display: id, .. } => ("area", display(&id)),
    };
    let mut source = vec![("kind", text(kind)), ("id", id)];
    let bounds = |geometry: Geometry| {
        let rect = geometry.rect;
        [
            (
                "rect",
                object(vec![
                    ("x", Json::Float(rect.x)),
                    ("y", Json::Float(rect.y)),
                    ("width", Json::Float(rect.width)),
                    ("height", Json::Float(rect.height)),
                ]),
            ),
            ("scale", Json::Float(geometry.scale)),
            (
                "size",
                Json::Array(vec![
                    Json::Unsigned(u64::from(geometry.width)),
                    Json::Unsigned(u64::from(geometry.height)),
                ]),
            ),
        ]
    };
    if let Some(geometry) = plan.geometry {
        source.extend(bounds(geometry));
    }
    // Every window of a multi-window take, in file order: screen.mov, window-1.mov, …
    if let (Target::Window(first), Some(geometry)) = (plan.target, plan.geometry)
        && !plan.windows.is_empty()
    {
        let window = |id: u32, file: String, geometry: Geometry| {
            let mut fields = vec![("id", text(&format!("win_{id}"))), ("file", text(&file))];
            fields.extend(bounds(geometry));
            object(fields)
        };
        let mut windows = vec![window(first, "screen.mov".into(), geometry)];
        for (n, (target, geometry)) in plan.windows.iter().enumerate() {
            if let Target::Window(id) = target {
                windows.push(window(*id, format!("window-{}.mov", n + 1), *geometry));
            }
        }
        source.push(("windows", Json::Array(windows)));
    }
    if let Some((requested, actual)) = plan.resized {
        source.push((
            "resized",
            object(vec![
                ("requested", pair(requested)),
                ("points", pair(actual)),
            ]),
        ));
    }
    let device = |device: &Option<DeviceInfo>| {
        device.as_ref().map_or(Json::Bool(false), |device| {
            object(vec![
                ("id", text(&device.id)),
                ("name", text(&device.name)),
                ("default", Json::Bool(device.default)),
            ])
        })
    };
    let system_audio = match &options.system_audio {
        SystemAudio::Off => Json::Bool(false),
        SystemAudio::All => text("all"),
        SystemAudio::Apps(_) => object(vec![(
            "apps",
            Json::Array(
                plan.audio_apps
                    .iter()
                    .map(|(bundle, name)| {
                        object(vec![("bundleId", text(bundle)), ("name", text(name))])
                    })
                    .collect(),
            ),
        )]),
    };
    let mut exclude: Vec<Json> = options
        .exclude
        .iter()
        .filter(|app| *app != "*")
        .map(|app| text(app))
        .collect();
    if options.hide_desktop {
        exclude.push(text("desktop"));
    }
    object(vec![
        ("source", object(source)),
        ("fps", Json::Unsigned(u64::from(CAPTURE_FPS))),
        ("mic", device(&plan.mic)),
        ("camera", device(&plan.camera)),
        ("systemAudio", system_audio),
        ("events", Json::Bool(options.events)),
        ("still", Json::Bool(options.still)),
        ("countdown", Json::Float(options.countdown.as_secs_f64())),
        ("duration", options.duration.map_or(Json::Null, Json::Float)),
        (
            "then",
            text(if options.then_pause { "pause" } else { "stop" }),
        ),
        ("exclude", Json::Array(exclude)),
    ])
}

fn poll_events(
    stage: &Stage,
    recording: &mut Recording,
    log: &mut Option<EventLog>,
    windows: &[(Target, Geometry)],
) -> Result<(), StoreError> {
    if log.is_none()
        && let Some(state) = recording.input_state()
    {
        // A multi-window take lists every window's bounds; events stay relative to the first.
        let mut rects = Vec::new();
        if !windows.is_empty() {
            rects.push(state.rect);
            rects.extend(windows.iter().map(|(_, geometry)| geometry.rect));
        }
        *log = Some(EventLog::open(&stage.0, state, &rects).map_err(storage)?);
    }
    if let Some(log) = log {
        log.poll(recording).map_err(storage)?;
    }
    Ok(())
}

fn recording_error(message: &str) -> StoreError {
    failure(
        "CAPTURE_FAILED",
        message,
        "Check the capture source and requested devices, then retry.",
    )
}

fn native(error: apple_sys::capture::CaptureError) -> StoreError {
    if error.permission_denied {
        return permissions::screen_denied();
    }
    recording_error(&error.to_string())
}

fn storage(error: io::Error) -> StoreError {
    failure(
        "IO",
        &error.to_string(),
        "Check that the project is writable and has free space, then retry.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display_plan() -> Plan {
        Plan {
            target: Target::Display(0),
            windows: Vec::new(),
            mic: None,
            camera: None,
            audio_apps: Vec::new(),
            geometry: None,
            resized: None,
        }
    }

    #[test]
    fn capture_agent_echoes_resolved_settings() {
        let args = splice_json::parse(
            r#"{"source":{"window":"win_7"},"size":[1280,720],"countdown":1.5,"duration":3,"then":"pause","mic":"usb","systemAudio":{"apps":["Music"]}}"#,
        )
        .unwrap();
        let options = Options::parse(&args).unwrap();
        let plan = Plan {
            target: Target::Window(7),
            windows: Vec::new(),
            mic: Some(DeviceInfo {
                kind: "mic".into(),
                id: "usb".into(),
                name: "Podcast Mic".into(),
                default: false,
            }),
            camera: None,
            audio_apps: vec![("com.apple.Music".into(), "Music".into())],
            geometry: Some(Geometry {
                rect: apple_sys::capture::Rect {
                    x: 10.0,
                    y: 20.0,
                    width: 1280.0,
                    height: 720.0,
                },
                scale: 2.0,
                width: 2560,
                height: 1440,
            }),
            resized: Some(([1280.0, 720.0], [1280.0, 720.0])),
        };
        let mut echo = String::new();
        splice_json::write(&settings(&options, &plan), &mut echo).unwrap();
        for part in [
            r#""source":{"kind":"window","id":"win_7","rect":{"x":10,"y":20,"width":1280,"height":720},"scale":2,"size":[2560,1440],"resized":{"requested":[1280,720],"points":[1280,720]}}"#,
            r#""fps":60"#,
            r#""mic":{"id":"usb","name":"Podcast Mic","default":false}"#,
            r#""camera":false"#,
            r#""systemAudio":{"apps":[{"bundleId":"com.apple.Music","name":"Music"}]}"#,
            r#""countdown":1.5"#,
            r#""duration":3"#,
            r#""then":"pause""#,
        ] {
            assert!(echo.contains(part), "{part} in {echo}");
        }
    }

    #[test]
    fn capture_agent_echoes_every_window_of_a_multi_window_take() {
        let args = splice_json::parse(r#"{"source":{"windows":["win_7","win_9"]}}"#).unwrap();
        let options = Options::parse(&args).unwrap();
        let geometry = |x: f64, width: f64| Geometry {
            rect: apple_sys::capture::Rect {
                x,
                y: 0.0,
                width,
                height: 600.0,
            },
            scale: 2.0,
            width: (width * 2.0) as u32,
            height: 1200,
        };
        let plan = Plan {
            target: Target::Window(7),
            windows: vec![(Target::Window(9), geometry(800.0, 400.0))],
            mic: None,
            camera: None,
            audio_apps: Vec::new(),
            geometry: Some(geometry(0.0, 800.0)),
            resized: None,
        };
        let mut echo = String::new();
        splice_json::write(&settings(&options, &plan), &mut echo).unwrap();
        assert!(
            echo.starts_with(
                r#"{"source":{"kind":"windows","id":"win_7","rect":{"x":0,"y":0,"width":800,"height":600},"scale":2,"size":[1600,1200],"windows":[{"id":"win_7","file":"screen.mov","rect":{"x":0,"y":0,"width":800,"height":600},"scale":2,"size":[1600,1200]},{"id":"win_9","file":"window-1.mov","rect":{"x":800,"y":0,"width":400,"height":600},"scale":2,"size":[800,1200]}]},"#
            ),
            "{echo}"
        );
        assert!(echo.contains(r#""systemAudio":"all""#), "{echo}");
    }

    #[test]
    fn capture_agent_pending_job_cancels_without_opening_hardware() {
        let temp = crate::tests::TestDirectory::new("capture-jobs");
        let package = temp.0.canonicalize().unwrap().join("recording.splice");
        fs::create_dir(&package).unwrap();
        let mut jobs = Jobs::default();
        let args = object(vec![
            ("countdown", Json::Unsigned(60)),
            ("events", Json::Bool(false)),
        ]);
        let first = jobs
            .begin(&package, Options::parse(&args).unwrap(), display_plan())
            .unwrap();
        let id = first.string("jobId").unwrap();
        let staged = package.join("takes").join(format!(".{id}"));
        assert!(jobs.active() && staged.is_dir());
        assert_eq!(first.string("state").unwrap(), "pending");
        assert_eq!(
            jobs.begin(&package, Options::parse(&args).unwrap(), display_plan())
                .unwrap_err()
                .code,
            "BUSY"
        );
        let query = object(vec![("jobId", text(id))]);
        assert_eq!(jobs.command("take.stop", &query).unwrap_err().code, "BUSY");
        // record.start {resume, duration} needs a running take.
        let resume = object(vec![("jobId", text(id)), ("duration", Json::Unsigned(3))]);
        assert_eq!(
            jobs.command("take.resume", &resume).unwrap_err().code,
            "NOT_RECORDING"
        );
        assert_eq!(
            jobs.command("take.cancel", &query)
                .unwrap()
                .string("state")
                .unwrap(),
            "cancelled"
        );
        assert!(!jobs.active() && !staged.exists());
        assert_eq!(
            jobs.command("take.status", &query)
                .unwrap()
                .string("state")
                .unwrap(),
            "cancelled"
        );
        let second = jobs
            .begin(&package, Options::parse(&args).unwrap(), display_plan())
            .unwrap();
        assert_ne!(second.string("jobId").unwrap(), id);
        drop(jobs);
        assert_eq!(fs::read_dir(package.join("takes")).unwrap().count(), 0);
    }

    #[test]
    fn capture_agent_discards_pending_take_when_writer_disconnects() {
        let temp = crate::tests::TestDirectory::new("capture-writer-disconnect");
        let package = temp.0.canonicalize().unwrap().join("recording.splice");
        fs::create_dir(&package).unwrap();
        let mut jobs = Jobs::default();
        let args = object(vec![("countdown", Json::Unsigned(60))]);
        let started = jobs
            .begin(&package, Options::parse(&args).unwrap(), display_plan())
            .unwrap();
        let (writer, agent) = UnixStream::pair().unwrap();
        agent.set_nonblocking(true).unwrap();
        jobs.attach_peer(Some(agent));
        jobs.tick(Instant::now());
        assert!(jobs.active());
        drop(writer);
        jobs.tick(Instant::now());
        let query = object(vec![("jobId", text(started.string("jobId").unwrap()))]);
        assert_eq!(
            jobs.command("take.status", &query)
                .unwrap()
                .string("state")
                .unwrap(),
            "cancelled"
        );
        assert_eq!(fs::read_dir(package.join("takes")).unwrap().count(), 0);
    }
}
