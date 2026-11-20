use super::presets::{GIF_WIDTH, Kind, Output as Request, reframed};
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
    fs,
    io::{self, Write},
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
    gif: Option<Box<GifEncode>>,
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
    if request.kind == Kind::Poster {
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

/// Posters render synchronously from the frame reader.
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
    let composition = Composition::prepare(project, store.path())
        .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
    let output = Output::prepare(destination(store, path, request)?, overwrite)?;
    let [w, h] = project.size;
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    composition
        .read_frame(request.range[0], &mut rgba)
        .map_err(|e| export_failure(&e))?;
    let png = splice_av::encode_png(&rgba, [w, h], w.max(h)).map_err(|e| export_failure(&e))?;
    fs::write(output.staged(), png).map_err(|e| path_error(output.staged(), &e))?;
    output.publish()?;
    Ok(object(vec![
        ("state", text("done")),
        (
            "output",
            Json::String(output.file().to_string_lossy().into_owned()),
        ),
        (
            "settings",
            object(vec![
                ("preset", text(request.preset)),
                ("width", Json::Unsigned(u64::from(w))),
                ("height", Json::Unsigned(u64::from(h))),
            ]),
        ),
    ]))
}

/// GIF bytes per pixel per frame and seconds per megapixel frame (`settings.estimate`), measured
/// on the Mac mini with a noisy test video: 0.06–0.12 bytes a pixel, 16–22 ms a megapixel at 960
/// and 480 px. A screen recording's still stretches cost far less.
const GIF_BYTES_PER_PIXEL: f64 = 0.08;
const GIF_SECONDS_PER_MEGAPIXEL: f64 = 0.02;
/// Past this length a GIF still exports, with a warning: an MP4 is far smaller (Recorder warns
/// past 60 s too).
const GIF_LONG_SECONDS: f64 = 60.0;

/// A GIF in progress: timeline frames decoded in order and streamed through [`GifWriter`]
/// (dropped or repeated to the GIF's rate), so memory stays one frame whatever the length.
struct GifEncode {
    frames: splice_av::Frames,
    writer: super::gif::GifWriter,
    file: io::BufWriter<fs::File>,
    rgba: Vec<u8>,
    range: [Frame; 2],
    /// Timeline and GIF frames per second.
    rates: [u32; 2],
    /// Timeline frames done (progress).
    done: Frame,
}

impl GifEncode {
    /// Renders and writes frames until `budget` passes; true once the file is complete.
    fn step(&mut self, budget: Duration, staged: &std::path::Path) -> Result<bool, StoreError> {
        let started = Instant::now();
        let io = |error: io::Error| path_error(staged, &error);
        let [timeline, gif] = self.rates;
        loop {
            // The GIF frames that show the next timeline frame: none when a lower rate drops it.
            let [first, last] = splice_eval::resample::outputs(self.done, timeline, gif);
            let rgba = (last > first).then_some(self.rgba.as_mut_slice());
            let Some(at) = self.frames.next(rgba).map_err(|e| export_failure(&e))? else {
                break;
            };
            debug_assert_eq!(at, self.range[0] + self.done);
            for _ in first..last {
                self.writer.frame(&self.rgba, &mut self.file).map_err(io)?;
            }
            self.done += 1;
            if started.elapsed() >= budget {
                return Ok(false);
            }
        }
        self.writer.finish(&mut self.file).map_err(io)?;
        self.file.flush().map_err(io)?;
        Ok(true)
    }
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
        if request.kind == Kind::Gif {
            return Self::gif(store, id, owner, request, path, overwrite);
        }
        let copy = rendered(store, request)?;
        let project = copy.as_ref().unwrap_or(store.project());
        let codec = codec(request.kind);
        let frames = request.range[1] - request.range[0];
        let seconds = f64::from(frames) / f64::from(project.fps);
        let pixels = f64::from(project.size[0]) * f64::from(project.size[1]);
        let fps = f64::from(request.fps);
        let video_bits = match codec {
            Codec::ProRes422 | Codec::ProRes4444 => pixels * fps * 4.0,
            Codec::Aac | Codec::Wav => 0.0,
            _ => request.bitrate.unwrap_or(pixels * fps * 0.35),
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
        let mut settings = vec![
            ("preset", text(request.preset)),
            ("width", Json::Unsigned(u64::from(project.size[0]))),
            ("height", Json::Unsigned(u64::from(project.size[1]))),
            ("fps", Json::Unsigned(u64::from(request.fps))),
        ];
        // Resampled (export.start {fps}): the output's own frame count and the timeline's rate.
        if request.fps != project.fps && codec.video() {
            settings.extend([
                ("timelineFps", Json::Unsigned(u64::from(project.fps))),
                (
                    "frames",
                    frame(splice_eval::resample::frames(
                        frames,
                        project.fps,
                        request.fps,
                    )),
                ),
            ]);
        }
        settings.extend([
            ("codec", text(codec.name())),
            ("container", text(&request.extension.to_uppercase())),
            ("encoder", text("AVFoundation")),
            ("path", Json::String(path.to_string_lossy().into_owned())),
            ("from", frame(request.range[0])),
            ("durationFrames", frame(frames)),
            ("chapters", Json::Unsigned(request.chapters.len() as u64)),
            ("credits", credits),
        ]);
        let settings = object(settings);
        let composition = Composition::prepare(project, store.path())
            .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
        let encoding = Encoding {
            codec,
            range: request.range,
            bitrate: request.bitrate,
            fps: request.fps,
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
            gif: None,
            state: State::Running(0.0),
            started: Instant::now(),
            frames,
        })
    }

    /// A GIF at `request.fps`, `request.gif_width` wide: any length, streamed frame by frame.
    fn gif(
        store: &Store,
        id: JobId,
        owner: Option<(String, SessionId)>,
        request: &Request,
        path: Option<&str>,
        overwrite: bool,
    ) -> Result<Self, StoreError> {
        let mut project = rendered(store, request)?.unwrap_or_else(|| store.project().clone());
        let width = request.gif_width.unwrap_or(GIF_WIDTH);
        // The canvas scaled to the GIF's width (shape kept, even height); upscaling is allowed.
        project.size = [
            width,
            ((u64::from(project.size[1]) * u64::from(width) / u64::from(project.size[0].max(1)))
                as u32
                & !1)
                .max(2),
        ];
        let [w, h] = project.size;
        let range = request.range;
        let frames = range[1] - range[0];
        let total = splice_eval::resample::frames(frames, project.fps, request.fps);
        let seconds = f64::from(frames) / f64::from(project.fps);
        let megapixels = f64::from(w) * f64::from(h) / 1e6;
        let bytes = f64::from(total) * megapixels * 1e6 * GIF_BYTES_PER_PIXEL;
        let target = destination(store, path, request)?;
        check_space(target.parent().unwrap_or(store.path()), bytes)?;
        let output = Output::prepare(target, overwrite)?;
        let composition = Composition::prepare(&project, store.path())
            .map_err(|error| super::frames::prepare_error(&error, export_failure))?;
        let file = fs::File::create_new(output.staged())
            .map_err(|error| path_error(output.staged(), &error))?;
        let path = output.file().to_path_buf();
        let mut settings = vec![
            ("preset", text(request.preset)),
            ("width", Json::Unsigned(u64::from(w))),
            ("height", Json::Unsigned(u64::from(h))),
            ("fps", Json::Unsigned(u64::from(request.fps))),
            ("frames", frame(total)),
            ("from", frame(range[0])),
            ("to", frame(range[1])),
            ("path", Json::String(path.to_string_lossy().into_owned())),
            // Rough cost before it runs: a busier picture is larger and slower.
            (
                "estimate",
                object(vec![
                    ("megabytes", Json::Float((bytes / 1e5).round() / 10.0)),
                    (
                        "seconds",
                        Json::Float(
                            (f64::from(total) * megapixels * GIF_SECONDS_PER_MEGAPIXEL * 10.0)
                                .round()
                                / 10.0,
                        ),
                    ),
                ]),
            ),
        ];
        if seconds > GIF_LONG_SECONDS {
            settings.push((
                "warning",
                text(&format!(
                    "A {seconds:.0} s GIF is about {:.0} MB; an MP4 (preset social-h264) of the same range is a fraction of that.",
                    bytes / 1e6
                )),
            ));
        }
        let gif = GifEncode {
            frames: composition
                .frames(range)
                .map_err(|error| super::frames::prepare_error(&error, export_failure))?,
            writer: super::gif::GifWriter::new([w, h], request.fps),
            file: io::BufWriter::new(file),
            rgba: vec![0; (w * h * 4) as usize],
            range,
            rates: [project.fps, request.fps],
            done: 0,
        };
        Ok(Self {
            id,
            owner,
            revision: store.project().rev,
            settings: object(settings),
            path,
            output: Some(output),
            export: None,
            gif: Some(Box::new(gif)),
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
        if let Some(gif) = self.gif.as_mut() {
            let staged = self.output.as_ref().map(|o| o.staged().to_path_buf());
            self.state = match (&self.state, staged) {
                // Dropping the output removes the partial file.
                (State::Cancelling(_), _) => State::Cancelled,
                (_, None) => State::Failed(export_failure(&"The export output was already used.")),
                (_, Some(staged)) => match gif.step(budget, &staged) {
                    Ok(false) => State::Running(
                        gif.done as f32 / (gif.range[1] - gif.range[0]).max(1) as f32,
                    ),
                    Ok(true) => match self.output.take().map(|output| output.publish()) {
                        Some(Ok(())) => {
                            if let Json::Object(fields) = &mut self.settings {
                                fields.push(("bytes".into(), Json::Unsigned(gif.writer.written())));
                            }
                            State::Done
                        }
                        Some(Err(error)) => State::Failed(error),
                        None => {
                            State::Failed(export_failure(&"The export output was already used."))
                        }
                    },
                    Err(error) => State::Failed(error),
                },
            };
            if !self.active() {
                self.gif = None;
                self.output = None;
            }
            return;
        }
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

/// What an export writes; at the package root any other name could be the project's own
/// file (project.json, journal.ops, media.json, lock).
const EXPORT_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "m4a", "wav", "gif", "png", "jpg", "fcpxml", "srt", "vtt", "txt",
];

fn destination(
    store: &Store,
    path: Option<&str>,
    request: &Request,
) -> Result<PathBuf, StoreError> {
    if let Some(path) = path {
        let file = store.path().join(path);
        // Inside the package only exports/ takes output: anything else would overwrite the
        // project's own files (project.json, the journal, media).
        let package = store
            .path()
            .canonicalize()
            .map_err(|error| path_error(store.path(), &error))?;
        let parent = file
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .unwrap_or_default();
        let export_kind = file
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        let unsafe_spot = parent.starts_with(&package)
            && !parent.starts_with(package.join("exports"))
            && (parent != package || !export_kind);
        if unsafe_spot {
            return Err(failure(
                "BAD_ARGS",
                &format!(
                    "{} would write into the project package's own files.",
                    file.display()
                ),
                "Export into the package's exports/ folder (omit output), next to the package, or anywhere outside it.",
            ));
        }
        return Ok(file);
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
