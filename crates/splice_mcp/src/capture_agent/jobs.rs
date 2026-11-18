use super::{capture_source_error, permissions, sources};
use crate::{
    Json, object,
    record_options::Options,
    text,
    wire::{Args, bad, display, failure},
};
use apple_sys::{
    capture::{Recording, RecordingStats},
    screenshot,
};
use splice_capture::EventLog;
use splice_core::history::StoreError;
use splice_model::JobId;
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) const POLL: Duration = Duration::from_millis(12);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct Jobs {
    jobs: Vec<Job>,
    last_id: u64,
}

struct Job {
    id: JobId,
    options: Options,
    state: State,
    stage: Option<Stage>,
    stats: RecordingStats,
}

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

    pub(super) fn begin(&mut self, package: &Path, options: Options) -> Result<Json, StoreError> {
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
            options,
            state: State::Pending(ready),
            stage: Some(Stage::create(package, id)?),
            stats: RecordingStats::default(),
        };
        job.tick(Instant::now());
        let receipt = job.status()?;
        self.jobs.push(job);
        Ok(receipt)
    }

    pub(super) fn tick(&mut self, now: Instant) {
        if let Some(job) = self.jobs.last_mut() {
            job.tick(now);
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
            }
            "take.cancel" => (),
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
        let target = sources::target(&self.options.source)?;
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
            let recording = Recording::start(
                &stage.0,
                target,
                self.options.mic.then_some(""),
                self.options.system_audio,
                self.options.camera,
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
            poll_events(stage, recording, log)?;
        }
        if self
            .options
            .duration
            .is_some_and(|duration| self.stats.elapsed_seconds >= duration)
        {
            self.finish();
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
                poll_events(stage, &mut recording, &mut log)?;
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
    }

    fn status(&self) -> Result<Json, StoreError> {
        let state = match &self.state {
            State::Pending(_) => "pending",
            State::Running { .. } => "recording",
            State::Ready => "ready",
            State::Cancelled => "cancelled",
            State::Failed(error) => return Err(failure(error.code, &error.message, &error.fix)),
        };
        let progress = if state == "ready" {
            1.0
        } else {
            self.options.duration.map_or(0.0, |duration| {
                (self.stats.elapsed_seconds / duration).clamp(0.0, 1.0)
            })
        };
        let bytes = self.stage.as_ref().map_or(Ok(0), Stage::bytes)?;
        let mut fields = vec![
            ("jobId", display(&self.id)),
            ("kind", text("record")),
            ("state", text(state)),
            ("progress", Json::Float(progress)),
            ("elapsed", Json::Float(self.stats.elapsed_seconds)),
            ("bytes", Json::Unsigned(bytes)),
            ("droppedFrames", Json::Unsigned(self.stats.dropped_frames)),
            (
                "settings",
                object(vec![
                    ("mic", Json::Bool(self.options.mic)),
                    ("systemAudio", Json::Bool(self.options.system_audio)),
                    ("camera", Json::Bool(self.options.camera)),
                    ("events", Json::Bool(self.options.events)),
                    ("still", Json::Bool(self.options.still)),
                ]),
            ),
        ];
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

fn poll_events(
    stage: &Stage,
    recording: &mut Recording,
    log: &mut Option<EventLog>,
) -> Result<(), StoreError> {
    if log.is_none()
        && let Some(state) = recording.input_state()
    {
        *log = Some(EventLog::open(&stage.0, state).map_err(storage)?);
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
            .begin(&package, Options::parse(&args).unwrap())
            .unwrap();
        let id = first.string("jobId").unwrap();
        let staged = package.join("takes").join(format!(".{id}"));
        assert!(jobs.active() && staged.is_dir());
        assert_eq!(first.string("state").unwrap(), "pending");
        assert_eq!(
            jobs.begin(&package, Options::parse(&args).unwrap())
                .unwrap_err()
                .code,
            "BUSY"
        );
        let query = object(vec![("jobId", text(id))]);
        assert_eq!(jobs.command("take.stop", &query).unwrap_err().code, "BUSY");
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
            .begin(&package, Options::parse(&args).unwrap())
            .unwrap();
        assert_ne!(second.string("jobId").unwrap(), id);
        drop(jobs);
        assert_eq!(fs::read_dir(package.join("takes")).unwrap().count(), 0);
    }
}
