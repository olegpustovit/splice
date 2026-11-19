use super::{
    Json,
    export_file::{Output, path_error},
    object, platform, text, timeline,
    wire::*,
};
use splice_av::{Composition, Export, ExportState};
use splice_core::history::{Store, StoreError};
use splice_model::{JobId, SessionId};
use std::{fs, io, path::PathBuf, time::Duration};

enum State {
    Running(f32),
    Cancelling(f32),
    Done,
    Cancelled,
    Failed(StoreError),
}

/// An export encoded on the owner's main thread (OGPU's Metal handles live there): the MCP server
/// and the viewer `step` it between requests and frames.
pub(super) struct Job {
    pub(super) id: JobId,
    pub(super) owner: Option<(String, SessionId)>,
    revision: u64,
    settings: Json,
    path: PathBuf,
    output: Option<Output>,
    export: Option<Export>,
    state: State,
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
        let composition = Composition::prepare(project, store.path())
            .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
        let export = composition
            .start_export(output.staged(), hevc)
            .map_err(|error| export_failure(&error))?;
        Ok(Self {
            id,
            owner,
            revision,
            settings,
            path,
            output: Some(output),
            export: Some(export),
            state: State::Running(0.0),
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
        matches!(self.state, State::Running(_) | State::Cancelling(_))
    }

    pub(super) fn cancel(&mut self) {
        if let State::Running(progress) = self.state {
            self.state = State::Cancelling(progress);
        }
    }

    /// Encodes for up to `budget`; publishes the file when the export completes.
    pub(super) fn step(&mut self, budget: Duration) {
        let Some(export) = self.export.as_mut() else {
            return;
        };
        if matches!(self.state, State::Cancelling(_)) {
            export.cancel();
        }
        self.state = match export.step(budget) {
            Ok(ExportState::Running(progress)) => match self.state {
                State::Cancelling(_) => State::Cancelling(progress),
                _ => State::Running(progress),
            },
            Ok(ExportState::Complete) => match self.output.take().map(|output| output.publish()) {
                Some(Ok(())) => State::Done,
                Some(Err(error)) => State::Failed(error),
                None => State::Failed(export_failure(&"The export output was already used.")),
            },
            Ok(ExportState::Cancelled) => State::Cancelled,
            Err(error) => State::Failed(export_failure(&error)),
        };
        if !self.active() {
            // Releases the encoder and its GPU resources here, on the owner's thread.
            self.export = None;
            self.output = None;
        }
    }

    pub(super) fn status(&self) -> Result<Json, StoreError> {
        let (name, progress) = match &self.state {
            State::Running(progress) => ("running", *progress),
            State::Cancelling(progress) => ("cancelling", *progress),
            State::Done => ("done", 1.0),
            State::Cancelled => ("cancelled", 0.0),
            State::Failed(error) => return Err(failure(error.code, &error.message, &error.fix)),
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

    /// Encodes to the end (the CLI, and owners closing their projects).
    pub(super) fn wait(&mut self) {
        while self.active() {
            self.step(Duration::from_millis(100));
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        if self.active() {
            self.cancel();
            self.wait();
        }
    }
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
