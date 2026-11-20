use super::presets::{GIF_FRAMES, GIF_WIDTH, Kind, Output as Request, gif_step, reframed};
use super::{
    Json,
    export_file::{Output, path_error},
    object, text, timeline,
    wire::*,
};
use splice_av::{Codec, Composition, Encoding, Export, ExportState};
use splice_core::history::{Store, StoreError};
use splice_model::{Frame, JobId, Project, SessionId};
use std::{
    fs, io,
    path::PathBuf,
    time::{Duration, Instant},
};

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
    started: Instant,
    frames: Frame,
}

/// Exports a snapshot of the current project and waits until the file is published (the CLI).
pub fn export_project(
    store: Store,
    preset: Option<&str>,
    path: Option<&str>,
    overwrite: bool,
) -> Result<Json, StoreError> {
    let end = timeline::duration(store.project())?;
    let mut args = vec![];
    if let Some(preset) = preset {
        args.push(("preset".to_owned(), Json::String(preset.into())));
    }
    let request = super::presets::resolve(store.project(), end, &Json::Object(args))?
        .pop()
        .expect("one output");
    if matches!(request.kind, Kind::Poster | Kind::Gif) {
        return still(&store, &request, path, overwrite);
    }
    let mut job = Job::start(
        &store,
        JobId::from_serial(1),
        None,
        &request,
        path,
        overwrite,
    )?;
    drop(store);
    job.wait();
    job.status(false)
}

fn codec(kind: Kind) -> Codec {
    match kind {
        Kind::Hevc => Codec::Hevc,
        Kind::ProRes422 => Codec::ProRes422,
        Kind::ProRes4444 => Codec::ProRes4444,
        Kind::HevcAlpha => Codec::HevcAlpha,
        Kind::Aac => Codec::Aac,
        Kind::Wav => Codec::Wav,
        _ => Codec::H264,
    }
}

/// The project to render for a request: reframed when its canvas changes.
fn rendered(store: &Store, request: &Request) -> Result<Option<Project>, StoreError> {
    reframed(store.project(), request.size)
}

/// Refuses an export that would not fit on the destination volume.
fn check_space(dir: &std::path::Path, bytes: f64) -> Result<(), StoreError> {
    let free = apple_sys::media::free_space(dir).unwrap_or(u64::MAX) as f64;
    if bytes * 1.2 > free {
        return Err(failure(
            "DISK_FULL",
            &format!(
                "The export needs about {:.0} MB; {:.0} MB are free.",
                bytes / 1e6,
                free / 1e6
            ),
            "Free disk space, export a shorter range, or pick a smaller preset.",
        ));
    }
    Ok(())
}

/// Posters and GIFs render synchronously from the frame reader.
pub(super) fn still(
    store: &Store,
    request: &Request,
    path: Option<&str>,
    overwrite: bool,
) -> Result<Json, StoreError> {
    let copy;
    let project = match rendered(store, request)? {
        Some(p) => {
            copy = p;
            &copy
        }
        None => store.project(),
    };
    // GIFs render at most GIF_WIDTH px wide (the settings report the size).
    let gif = request.kind == Kind::Gif;
    let shrunk;
    let project = if gif && project.size[0] > GIF_WIDTH {
        let mut small = project.clone();
        small.size = [
            GIF_WIDTH,
            (project.size[1] * GIF_WIDTH / project.size[0]) & !1,
        ];
        shrunk = small;
        &shrunk
    } else {
        project
    };
    let composition = Composition::prepare(project, store.path())
        .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
    let output = Output::prepare(destination(store, path, request)?, overwrite)?;
    let [w, h] = project.size;
    let frame_bytes = (w * h * 4) as usize;
    let mut rgba = vec![0u8; frame_bytes];
    let mut settings = vec![
        ("preset", text(request.preset)),
        ("width", Json::Unsigned(u64::from(w))),
        ("height", Json::Unsigned(u64::from(h))),
    ];
    if gif {
        // About 15 fps; resolve refused a range over GIF_FRAMES, so nothing is cut short.
        let step = gif_step(project.fps) as Frame;
        let frames: Vec<Frame> = (request.range[0]..request.range[1])
            .step_by(step as usize)
            .collect();
        debug_assert!(frames.len() <= GIF_FRAMES);
        settings.extend([
            ("fps", Json::Float(f64::from(project.fps) / f64::from(step))),
            ("frames", Json::Unsigned(frames.len() as u64)),
            ("from", Json::Signed(i64::from(request.range[0]))),
            ("to", Json::Signed(i64::from(request.range[1]))),
        ]);
        let mut all = Vec::with_capacity(frames.len() * frame_bytes);
        for at in frames {
            composition
                .read_frame(at, &mut rgba)
                .map_err(|e| export_failure(&e))?;
            all.extend_from_slice(&rgba);
        }
        apple_sys::image::gif(
            output.staged(),
            &all,
            [w, h],
            f64::from(step as u32) / f64::from(project.fps),
        )
        .map_err(|e| export_failure(&e))?;
    } else {
        let at = request.range[0];
        composition
            .read_frame(at, &mut rgba)
            .map_err(|e| export_failure(&e))?;
        let png =
            splice_av::encode_frame_png(&rgba, [w, h], w.max(h)).map_err(|e| export_failure(&e))?;
        fs::write(output.staged(), png).map_err(|e| path_error(output.staged(), &e))?;
    }
    output.publish()?;
    Ok(object(vec![
        ("state", text("done")),
        (
            "output",
            Json::String(output.file().to_string_lossy().into_owned()),
        ),
        ("settings", object(settings)),
    ]))
}

impl Job {
    pub(super) fn start(
        store: &Store,
        id: JobId,
        owner: Option<(String, SessionId)>,
        request: &Request,
        path: Option<&str>,
        overwrite: bool,
    ) -> Result<Self, StoreError> {
        let revision = store.project().rev;
        let end = timeline::duration(store.project())?;
        if end == 0 {
            return Err(failure(
                "OUT_OF_RANGE",
                "The timeline has no frames to export.",
                "Add a clip or overlay before exporting.",
            ));
        }
        let copy = rendered(store, request)?;
        let project = copy.as_ref().unwrap_or(store.project());
        let codec = codec(request.kind);
        let frames = request.range[1] - request.range[0];
        let seconds = f64::from(frames) / f64::from(project.fps);
        let pixels = f64::from(project.size[0]) * f64::from(project.size[1]);
        let video_bits = match codec {
            Codec::ProRes422 | Codec::ProRes4444 => pixels * f64::from(project.fps) * 4.0,
            Codec::Aac | Codec::Wav => 0.0,
            _ => request
                .bitrate
                .unwrap_or(pixels * f64::from(project.fps) * 0.35),
        };
        let audio_bits = if codec == Codec::Wav {
            1_536_000.0
        } else {
            256_000.0
        };
        let target = destination(store, path, request)?;
        check_space(
            target.parent().unwrap_or(store.path()),
            (video_bits + audio_bits) * seconds / 8.0,
        )?;
        let output = Output::prepare(target, overwrite)?;
        let path = output.file().to_path_buf();
        if let Some(format) = request.sidecar {
            let cues = super::media::captions(
                store.project(),
                &Json::Object(vec![]),
                format,
                Some(request.range),
            )?;
            super::export_file::write_sidecar(
                &path.with_extension(format),
                cues.field("text")
                    .and_then(Json::as_str)
                    .unwrap_or_default(),
                overwrite,
            )?;
        }
        // Music credits (T45.3): the library tracks this range plays, next to the movie.
        let credits = match super::music::credits(store, request.range) {
            Some(body) => {
                let file = path.with_extension("credits.txt");
                super::export_file::write_sidecar(&file, &body, overwrite)?;
                Json::String(file.to_string_lossy().into_owned())
            }
            None => Json::Null,
        };
        let settings = object(vec![
            ("preset", text(request.preset)),
            ("width", Json::Unsigned(u64::from(project.size[0]))),
            ("height", Json::Unsigned(u64::from(project.size[1]))),
            ("fps", Json::Unsigned(u64::from(project.fps))),
            ("codec", text(&format!("{codec:?}"))),
            ("container", text(&request.extension.to_uppercase())),
            ("encoder", text("AVFoundation")),
            ("path", Json::String(path.to_string_lossy().into_owned())),
            ("from", frame(request.range[0])),
            ("durationFrames", frame(frames)),
            ("chapters", Json::Unsigned(request.chapters.len() as u64)),
            ("credits", credits),
        ]);
        let composition = Composition::prepare(project, store.path())
            .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
        let encoding = Encoding {
            codec,
            range: request.range,
            bitrate: request.bitrate,
            chapters: request.chapters.clone(),
        };
        let export = composition
            .start_export(output.staged(), &encoding)
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
            started: Instant::now(),
            frames,
        })
    }

    pub(super) fn started(&self) -> Json {
        object(vec![
            ("jobId", display(&self.id)),
            ("state", text("running")),
            ("settings", self.settings.clone()),
        ])
    }

    /// Fraction of frames encoded.
    pub(super) fn progress(&self) -> f64 {
        match self.state {
            State::Running(progress) | State::Cancelling(progress) => f64::from(progress),
            State::Done => 1.0,
            _ => 0.0,
        }
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

    /// The job's state; `queued` while two earlier exports are encoding.
    pub(super) fn status(&self, queued: bool) -> Result<Json, StoreError> {
        let (name, progress) = match &self.state {
            State::Running(progress) if queued => ("queued", *progress),
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
        if matches!(name, "running" | "cancelling") && progress > 0.0 {
            let elapsed = self.started.elapsed().as_secs_f64().max(1e-3);
            let done = f64::from(progress) * f64::from(self.frames);
            fields.push(("fps", Json::Float((done / elapsed * 10.0).round() / 10.0)));
            fields.push((
                "eta",
                Json::Float(((f64::from(self.frames) - done) / (done / elapsed)).round()),
            ));
        }
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

fn destination(
    store: &Store,
    path: Option<&str>,
    request: &Request,
) -> Result<PathBuf, StoreError> {
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
    let suffix = if request.size == store.project().size {
        String::new()
    } else {
        format!("-{}x{}", request.size[0], request.size[1])
    };
    Ok(dir.join(format!(
        "{name}-r{}{suffix}.{}",
        store.project().rev,
        request.extension
    )))
}

fn export_failure(error: &dyn std::fmt::Display) -> StoreError {
    failure(
        "EXPORT_FAILED",
        &error.to_string(),
        "Check the source media and export destination, then try the job again.",
    )
}
