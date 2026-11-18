use super::{
    Json,
    export_file::{Output, path_error},
    object, platform, text, timeline,
    wire::*,
};
use splice_av::{Composition, ExportState};
use splice_core::history::{Store, StoreError};
use splice_model::{JobId, Project, SessionId};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

enum State {
    Running(f32),
    Cancelling(f32),
    Done,
    Cancelled,
    Failed(StoreError),
}

pub(super) struct Job {
    pub(super) id: JobId,
    pub(super) owner: Option<(String, SessionId)>,
    revision: u64,
    settings: Json,
    path: PathBuf,
    state: Arc<Mutex<State>>,
    worker: Option<JoinHandle<()>>,
}

/// Exports a snapshot of the current project and waits until the MP4 is published.
pub fn export_project(
    store: Store,
    preset: Option<&str>,
    path: Option<&str>,
    overwrite: bool,
) -> Result<Json, StoreError> {
    let hevc = platform::export_options(preset, path)?;
    let mut job = Job::start(&store, JobId::from_serial(1), None, hevc, path, overwrite)?;
    drop(store);
    job.wait();
    job.status()
}

impl Job {
    pub(super) fn start(
        store: &Store,
        id: JobId,
        owner: Option<(String, SessionId)>,
        hevc: bool,
        path: Option<&str>,
        overwrite: bool,
    ) -> Result<Self, StoreError> {
        let project = store.project();
        let revision = project.rev;
        let end = timeline::duration(project)?;
        if end == 0 {
            return Err(failure(
                "OUT_OF_RANGE",
                "The timeline has no frames to export.",
                "Add a clip or overlay before exporting.",
            ));
        }
        let output = Output::prepare(destination(store, path)?, overwrite)?;
        let path = output.file().to_path_buf();
        let settings = object(vec![
            ("width", Json::Unsigned(u64::from(project.size[0]))),
            ("height", Json::Unsigned(u64::from(project.size[1]))),
            ("fps", Json::Unsigned(u64::from(project.fps))),
            ("codec", text(if hevc { "HEVC" } else { "H.264" })),
            ("container", text("MP4")),
            ("encoder", text("AVFoundation")),
            ("path", Json::String(path.to_string_lossy().into_owned())),
            ("durationFrames", frame(end)),
        ]);
        let project = project.clone();
        let package = store.path().to_path_buf();
        let state = Arc::new(Mutex::new(State::Running(0.0)));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name(format!("splice-{id}"))
            .spawn(move || {
                let result = encode(&project, &package, &output, hevc, &shared);
                let mut state = shared.lock().expect("export state lock");
                let next = if matches!(*state, State::Cancelling(_)) {
                    State::Cancelled
                } else {
                    match result {
                        Ok(true) => match output.publish() {
                            Ok(()) => State::Done,
                            Err(error) => State::Failed(error),
                        },
                        Ok(false) => State::Cancelled,
                        Err(error) => State::Failed(error),
                    }
                };
                drop(output);
                *state = next;
            })
            .map_err(|error| export_failure(&error))?;
        Ok(Self {
            id,
            owner,
            revision,
            settings,
            path,
            state,
            worker: Some(worker),
        })
    }

    pub(super) fn started(&self) -> Json {
        object(vec![
            ("jobId", display(&self.id)),
            ("state", text("running")),
            ("settings", self.settings.clone()),
        ])
    }

    pub(super) fn active(&self) -> bool {
        matches!(
            *self.state.lock().expect("export state lock"),
            State::Running(_) | State::Cancelling(_)
        ) && self.worker_running()
    }

    fn worker_running(&self) -> bool {
        self.worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
    }

    pub(super) fn cancel(&self) {
        let mut state = self.state.lock().expect("export state lock");
        if let State::Running(progress) = *state {
            *state = State::Cancelling(progress);
        }
    }

    pub(super) fn status(&self) -> Result<Json, StoreError> {
        let state = self.state.lock().expect("export state lock");
        let (name, progress) = match &*state {
            State::Running(progress) if self.worker_running() => ("running", *progress),
            State::Cancelling(progress) if self.worker_running() => ("cancelling", *progress),
            State::Done => ("done", 1.0),
            State::Cancelled => ("cancelled", 0.0),
            State::Failed(error) => return Err(failure(error.code, &error.message, &error.fix)),
            _ => {
                return Err(export_failure(
                    &"The encoder thread ended without a result.",
                ));
            }
        };
        let mut fields = vec![
            ("jobId", display(&self.id)),
            ("kind", text("export")),
            ("state", text(name)),
            ("progress", Json::Float(f64::from(progress))),
            ("revision", rev(self.revision)),
            ("settings", self.settings.clone()),
        ];
        if name == "done" {
            fields.push((
                "output",
                Json::String(self.path.to_string_lossy().into_owned()),
            ));
        }
        Ok(object(fields))
    }

    pub(super) fn wait(&mut self) {
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            *self.state.lock().expect("export state lock") =
                State::Failed(export_failure(&"The encoder thread could not finish."));
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.worker.is_some() {
            self.cancel();
            self.wait();
        }
    }
}

fn encode(
    project: &Project,
    package: &Path,
    output: &Output,
    hevc: bool,
    state: &Mutex<State>,
) -> Result<bool, StoreError> {
    if cancelling(state) {
        return Ok(false);
    }
    let composition =
        Composition::prepare(project, package).map_err(|error| export_failure(&error))?;
    if cancelling(state) {
        return Ok(false);
    }
    let mut job = composition
        .start_export(output.staged(), hevc)
        .map_err(|error| export_failure(&error))?;
    let mut cancelled = false;
    loop {
        if !cancelled && cancelling(state) {
            job.cancel();
            cancelled = true;
        }
        match job.status().map_err(|error| export_failure(&error))? {
            ExportState::Complete => return Ok(true),
            ExportState::Cancelled => return Ok(false),
            ExportState::Running(progress) => {
                if let State::Running(last) = &mut *state.lock().expect("export state lock") {
                    *last = last.max(progress);
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn cancelling(state: &Mutex<State>) -> bool {
    matches!(
        *state.lock().expect("export state lock"),
        State::Cancelling(_)
    )
}

fn destination(store: &Store, path: Option<&str>) -> Result<PathBuf, StoreError> {
    if let Some(path) = path {
        return Ok(store.path().join(path));
    }
    let dir = store.path().join("exports");
    match fs::create_dir(&dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(path_error(&dir, &error)),
    }
    let kind = fs::symlink_metadata(&dir).map_err(|error| path_error(&dir, &error))?;
    if !kind.is_dir() {
        return Err(failure(
            "BAD_ARGS",
            "The package's exports path must be a directory.",
            "Pass a different output path or create the exports directory.",
        ));
    }
    let name = store
        .path()
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    Ok(dir.join(format!("{name}-r{}.mp4", store.project().rev)))
}

fn export_failure(error: &dyn std::fmt::Display) -> StoreError {
    failure(
        "EXPORT_FAILED",
        &error.to_string(),
        "Check the source media and export destination, then try the job again.",
    )
}
