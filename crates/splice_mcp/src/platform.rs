use super::{
    Json,
    project::{Projects, package_path},
    wire::*,
};
use splice_core::history::{Store, StoreError};
use splice_model::{Frame, JobId};
use std::path::Path;

pub(super) enum FrameSelection {
    Explicit(Vec<Frame>),
    Filmstrip(u32),
}

impl FrameSelection {
    pub(super) fn parse(args: &Json) -> Result<Self, StoreError> {
        match (args.field("count"), args.field("at")) {
            (Some(count), None)
                if let Some(count) = count.as_u64().filter(|n| (1..=16).contains(n)) =>
            {
                Ok(Self::Filmstrip(count as u32))
            }
            (None, Some(frames))
                if let Some(frames) = frames
                    .as_array()
                    .filter(|items| (1..=16).contains(&items.len())) =>
            {
                let frames = frames
                    .iter()
                    .map(|value| {
                        value
                            .as_u64()
                            .and_then(|n| Frame::try_from(n).ok())
                            .ok_or_else(|| bad("at", "an array of nonnegative frame numbers"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Self::Explicit(frames))
            }
            _ => Err(bad(
                "at or count",
                "up to 16 explicit frames or a filmstrip count from 1 to 16",
            )),
        }
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn resolve(self, end: Frame) -> Result<Vec<Frame>, StoreError> {
        if end <= 0 {
            return Err(failure(
                "OUT_OF_RANGE",
                "The timeline has no frames.",
                "Add a clip or overlay before rendering.",
            ));
        }
        match self {
            Self::Explicit(frames) => {
                if let Some(at) = frames.iter().find(|&&at| at < 0 || at >= end) {
                    return Err(failure(
                        "OUT_OF_RANGE",
                        &format!("Frame {at} is outside the timeline."),
                        &format!("Choose a frame from 0 through {}.", end - 1),
                    ));
                }
                Ok(frames)
            }
            Self::Filmstrip(count) => {
                let divisor = u64::from(count.saturating_sub(1).max(1));
                let last = u64::from((end - 1) as u32);
                Ok((0..count)
                    .map(|i| ((u64::from(i) * last + divisor / 2) / divisor) as Frame)
                    .collect())
            }
        }
    }
}

pub(super) fn frames(
    store: &Store,
    args: &Json,
    content: &mut Vec<Json>,
) -> Result<Json, StoreError> {
    let selection = FrameSelection::parse(args)?;
    let sheet = args.flag("sheet")?;
    #[cfg(target_os = "macos")]
    {
        super::frames::render_tool(store, selection, sheet, content)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (store, content, sheet);
        // Keep argument validation common and leave timeline and media handling to the native renderer.
        match selection {
            FrameSelection::Explicit(values) => {
                let _ = values;
            }
            FrameSelection::Filmstrip(count) => {
                let _ = count;
            }
        }
        Err(unavailable("Frame rendering"))
    }
}

fn check_path(path: Option<&str>, extension: &str) -> Result<(), StoreError> {
    if path.is_some_and(|path| {
        path.contains('\0')
            || !Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
    }) {
        return Err(bad(
            "path",
            &format!("an output path ending in .{extension}"),
        ));
    }
    Ok(())
}

fn io_error(path: &Path, error: &std::io::Error) -> StoreError {
    failure(
        "IO",
        &format!("Could not write {}: {error}.", path.display()),
        "Check the destination exists and is writable.",
    )
}

/// `export.start {format:"fcpxml"}`: writes the timeline for Final Cut Pro (every platform).
fn export_fcpxml(store: &Store, path: Option<&str>) -> Result<Json, StoreError> {
    check_path(path, "fcpxml")?;
    let name = store
        .path()
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let file = match path {
        Some(path) => store.path().join(path),
        None => {
            let dir = store.path().join("exports");
            std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
            dir.join(format!("{name}-r{}.fcpxml", store.project().rev))
        }
    };
    let xml = super::fcpxml::write(store.project(), store.path(), &name);
    std::fs::write(&file, xml).map_err(|e| io_error(&file, &e))?;
    Ok(super::object(vec![
        ("state", super::text("done")),
        ("output", Json::String(file.to_string_lossy().into_owned())),
        (
            "settings",
            super::object(vec![("format", super::text("FCPXML 1.10"))]),
        ),
    ]))
}

pub(super) fn export(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let owner = args.string("projectId")?;
    if args.field("sessionId").is_some() {
        return export_in_session(projects, owner, args);
    }
    // An export reads one revision; without a session it gets its own, so the first call works
    // (a missing session cost agents a round trip). A failed export closes it again.
    let session = projects.lookup_id(owner)?.start_session("export", now())?;
    let mut implicit = args.clone();
    if let Json::Object(fields) = &mut implicit {
        fields.push(("sessionId".into(), Json::String(session.id.to_string())));
    }
    let result = export_in_session(projects, owner, &implicit);
    if result.is_err() {
        let _ = projects.lookup_id(owner)?.end_session(session.id);
    }
    result
}

fn export_in_session(
    projects: &mut Projects,
    owner: &str,
    args: &Json,
) -> Result<Json, StoreError> {
    let store = projects.lookup_id(owner)?;
    let path = args.optional_string("path")?;
    let overwrite = args.flag("overwrite")?;
    let options = write_options(args)?;
    if options.dry_run {
        return Err(bad("dryRun", "false or omitted for export.start"));
    }
    store.check_write(&options)?;
    match args.optional_string("format")? {
        None | Some("video") => {}
        Some("fcpxml") => {
            let receipt = export_fcpxml(store, path)?;
            return Ok(projects.finish_export(owner, receipt));
        }
        Some(_) => return Err(bad("format", "video (default) or fcpxml")),
    }
    let end = super::timeline::duration(store.project())?;
    let outputs = super::presets::resolve(store.project(), end, args)?;
    if outputs.len() == 1 {
        check_path(path, outputs[0].extension)?;
    } else if path.is_some() {
        return Err(bad(
            "path",
            "omitted with variants (each lands in exports/)",
        ));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = overwrite;
        Err(unavailable("Export"))
    }
    #[cfg(target_os = "macos")]
    {
        let mut receipts = Vec::with_capacity(outputs.len());
        // Jobs join the project only once every variant started: if one fails, dropping the
        // others cancels them instead of orphaning running jobs nobody was told about (M12).
        let mut started = Vec::with_capacity(outputs.len());
        for output in outputs {
            let store = projects.lookup_id(owner)?;
            match output.kind {
                super::presets::Kind::Poster | super::presets::Kind::Gif => {
                    let receipt = super::export::still(store, &output, path, overwrite)?;
                    receipts.push(projects.finish_export(owner, receipt));
                }
                _ => {
                    let id = projects.next_export_id();
                    let store = projects.lookup_id(owner)?;
                    let job = super::export::Job::start(
                        store,
                        id,
                        Some((owner.to_owned(), options.session.expect("agent session"))),
                        &output,
                        path,
                        overwrite,
                    )?;
                    receipts.push(job.started());
                    started.push(job);
                }
            }
        }
        projects.exports.append(&mut started);
        Ok(match receipts.len() {
            1 => receipts.pop().expect("one receipt"),
            _ => super::object(vec![
                ("state", super::text("running")),
                ("jobs", Json::Array(receipts)),
            ]),
        })
    }
}

pub(super) fn job(projects: &mut Projects, name: &str, args: &Json) -> Result<Json, StoreError> {
    // `wait` (seconds, at most 120) holds jobs.status until the job stops running, encoding
    // exports meanwhile (they advance on this thread), so one call or one code.run sees it done.
    let wait = match args.field("wait") {
        None => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|seconds| (0.0..=120.0).contains(seconds))
                .ok_or_else(|| bad("wait", "seconds from 0 to 120"))?,
        ),
    };
    let Some(wait) = wait.filter(|_| name == "jobs.status") else {
        return job_once(projects, name, args);
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f64(wait);
    loop {
        let status = job_once(projects, name, args)?;
        let running = matches!(
            status.field("state").and_then(Json::as_str),
            Some("queued" | "pending" | "running" | "recording" | "paused" | "cancelling")
        );
        if !running || std::time::Instant::now() >= deadline {
            return Ok(status);
        }
        #[cfg(target_os = "macos")]
        let encoding = projects.tick_exports(super::rpc::EXPORT_SLICE);
        #[cfg(not(target_os = "macos"))]
        let encoding = false;
        if !encoding {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

fn job_once(projects: &mut Projects, name: &str, args: &Json) -> Result<Json, StoreError> {
    let id = job_id(args)?;
    let project = args.optional_string("projectId")?;
    let cancel = name == "jobs.cancel";
    if cancel && args.flag("dryRun")? {
        return Err(bad("dryRun", "false or omitted for jobs.cancel"));
    }
    #[cfg(target_os = "macos")]
    if let Some(result) = super::background::job(projects, id, cancel) {
        return result;
    }
    #[cfg(target_os = "macos")]
    if projects.recordings.iter().any(|job| job.id == id) {
        return super::record::command(
            projects,
            id,
            if cancel { "take.cancel" } else { "take.status" },
            args,
        );
    }
    if let Some(project) = project {
        let store = projects.lookup_id(project)?;
        if cancel {
            store.check_write(&write_options(args)?)?;
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(index) = projects.exports.iter().position(|job| job.id == id) {
        let (owner, session) = projects.exports[index]
            .owner
            .clone()
            .ok_or_else(|| missing_job(id))?;
        if project.is_some_and(|project| project != owner) {
            return Err(missing_job(id));
        }
        if project.is_none() {
            let store = projects.lookup_id(&owner)?;
            if cancel {
                let options =
                    if args.field("sessionId").is_some() || args.field("ifRevision").is_some() {
                        write_options(args)?
                    } else {
                        splice_core::history::WriteOptions {
                            source: splice_core::history::Source::Agent,
                            session: Some(session),
                            if_revision: Some(store.project().rev),
                            ..Default::default()
                        }
                    };
                store.check_write(&options)?;
            }
        }
        let job = &mut projects.exports[index];
        if cancel {
            job.cancel();
        }
        // Polling advances the job: a code.run program that loops on jobs.status blocks the
        // serve loop that otherwise steps exports between requests. Two encode at once; the
        // rest wait their turn.
        let ahead = projects.exports[..index]
            .iter()
            .filter(|j| j.active())
            .count();
        let job = &mut projects.exports[index];
        if ahead < super::project::CONCURRENT_EXPORTS {
            job.step(super::rpc::EXPORT_SLICE);
        }
        return job.status(ahead >= super::project::CONCURRENT_EXPORTS);
    }
    // Instant exports (FCPXML, poster, GIF) are done jobs.
    if let Some((_, owner, status)) = projects.finished.iter().find(|(job, ..)| *job == id) {
        if project.is_some_and(|project| project != owner) {
            return Err(missing_job(id));
        }
        return Ok(status.clone());
    }
    Err(missing_job(id))
}

pub(super) fn missing_job(id: JobId) -> StoreError {
    failure(
        "NOT_FOUND",
        &format!("No job {id} is running or retained."),
        "Pass a jobId returned by export.start or record.start in this server.",
    )
}

/// Window titles can name private documents and conversations (D19): a capture dry run lists
/// them only when the caller passes `titles:true`. `window:{app}` selects without them.
#[cfg(any(target_os = "macos", test))]
pub(super) fn window_titles(mut reply: Json, titles: bool) -> Json {
    if titles {
        return reply;
    }
    let windows = match &mut reply {
        Json::Object(fields) => fields
            .iter_mut()
            .find(|(key, _)| key == "sources")
            .and_then(|(_, sources)| match sources {
                Json::Object(sources) => sources.iter_mut().find(|(key, _)| key == "windows"),
                _ => None,
            }),
        _ => None,
    };
    if let Some((_, Json::Array(windows))) = windows {
        for window in windows {
            if let Json::Object(fields) = window {
                fields.retain(|(key, _)| key != "title");
            }
        }
    }
    reply
}

pub(super) fn record_start(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    // record.start {resume: jobId} resumes a paused take; with marker, it marks the running take.
    if let Some(job) = args.optional_string("resume")? {
        let id: JobId = job
            .parse()
            .map_err(|_| bad("resume", "a recording jobId"))?;
        #[cfg(target_os = "macos")]
        {
            // Resuming a running take is a no-op, so `resume` with `marker` both continues a
            // paused take and marks it (it used to only mark, leaving the take paused).
            let status = super::record::command(projects, id, "take.resume", args)?;
            if args.field("marker").is_none() {
                return Ok(status);
            }
            return super::record::command(projects, id, "take.mark", args);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = id;
            return Err(unavailable("Recording"));
        }
    }
    if args.flag("recover")? {
        #[cfg(target_os = "macos")]
        return super::record::recover(projects, args);
        #[cfg(not(target_os = "macos"))]
        return Err(unavailable("Recording"));
    }
    let path = args.optional_string("path")?;
    let dry = args.flag("dryRun")?;
    let create = args.flag("create")?;
    if args.field("projectId").is_some() {
        if path.is_some() || create {
            return Err(bad(
                "projectId",
                "the only project selector, or use path and create",
            ));
        }
        let store = projects.lookup(args)?;
        if !dry {
            store.check_write(&write_options(args)?)?;
        }
    } else if let Some(path) = path {
        package_path(path)?;
    } else if create || !dry {
        return Err(bad(
            "projectId or path",
            "an open project ID or a .splice path",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        if dry {
            let titles = args.flag("titles")?;
            super::capture_agent::request("record.start", args)
                .map(|reply| window_titles(reply, titles))
        } else {
            super::record::start(projects, args)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(unavailable("Recording"))
    }
}

pub(super) fn record_stop(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let id = job_id(args)?;
    args.flag("insert")?;
    let method = if args.flag("pause")? {
        "take.pause"
    } else {
        "take.stop"
    };
    #[cfg(target_os = "macos")]
    {
        super::record::command(projects, id, method, args)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, method);
        if args.field("projectId").is_some() {
            projects.lookup(args)?.check_write(&write_options(args)?)?;
        }
        Err(unavailable("Recording"))
    }
}

fn job_id(args: &Json) -> Result<JobId, StoreError> {
    args.string("jobId")?
        .parse()
        .map_err(|_| bad("jobId", "a job ID returned by export.start or record.start"))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn unavailable(feature: &str) -> StoreError {
    failure(
        "UNSUPPORTED_PLATFORM",
        &format!("{feature} requires the native macOS backend."),
        "Use a macOS build with the native media and capture backend.",
    )
}

/// Transcription needs the native speech backend.
#[cfg(not(target_os = "macos"))]
pub(super) fn transcribe(_: &mut Projects, _: &Json) -> Result<Json, StoreError> {
    Err(unavailable("Transcription"))
}

/// `transcript.get {transcribe:true}`: recognizes the clip's speech on device and commits the
/// words as `transcript.set` in the caller's session, then answers like `transcript.get`.
#[cfg(target_os = "macos")]
pub(super) fn transcribe(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let session = args.session()?;
    let project = args.string("projectId")?.to_owned();
    let store = projects.lookup(args)?;
    let p = store.project();
    // A sound item or media overlay (voice-over, podcast track) transcribes its own media.
    let media_id = if args.field("item").is_some() {
        super::media::item_media(p, args)?.2
    } else {
        let slot = match args.optional_string("clip")? {
            Some(id) => {
                let id: splice_model::ClipId = id
                    .parse()
                    .map_err(|_| bad("clip", "a clip ID from timeline.snapshot"))?;
                p.clips.find(id).ok_or_else(|| {
                    failure(
                        "NOT_FOUND",
                        &format!("No clip {id}."),
                        "Call timeline.snapshot to find a current clip ID.",
                    )
                })?
            }
            None => p
                .clips
                .order()
                .iter()
                .copied()
                .find(|&slot| {
                    p.clips
                        .data(slot)
                        .and_then(|data| data.media)
                        .is_some_and(|id| {
                            p.media
                                .iter()
                                .any(|m| m.id == id && !m.probe.audio.is_empty())
                        })
                })
                .ok_or_else(|| {
                    failure(
                        "NOT_FOUND",
                        "No clip on the spine has audio.",
                        "Record with mic or insert a clip with speech, then transcribe it.",
                    )
                })?,
        };
        p.clips
            .data(slot)
            .and_then(|data| data.media)
            .ok_or_else(|| bad("clip", "a clip with media"))?
    };
    let media = p
        .media
        .iter()
        .find(|m| m.id == media_id)
        .ok_or_else(|| bad("clip", "a clip with media"))?;
    // A take's speech is in its microphone (else system audio) file; an import is one file.
    let base = store.path().join(&media.path);
    let audio = match media
        .take
        .as_ref()
        .and_then(|t| t.mic.as_deref().or(t.system.as_deref()))
    {
        Some(file) if base.is_dir() => base.join(file),
        Some(file) => base.with_file_name(file),
        None => base,
    };
    let language = args
        .optional_string("language")?
        .unwrap_or("en-US")
        .to_owned();
    let (fps, frames) = (media.probe.fps, media.probe.frames);
    let long = f64::from(frames) / fps > super::background::LONG_SECONDS || args.flag("async")?;
    let spoken = language.clone();
    let work = move || recognize(&audio, &spoken, fps, frames);
    let args = args.clone();
    let finish = move |store: &mut Store, words: Json| {
        let op = super::object(vec![
            ("cmd", super::text("transcript.set")),
            ("media", display(&media_id)),
            ("words", words),
            ("language", super::text(&language)),
        ]);
        let options = splice_core::history::WriteOptions {
            source: splice_core::history::Source::Agent,
            session: Some(session),
            if_revision: Some(store.project().rev),
            label: Some("Agent: Transcribe".into()),
            time: now(),
            ..Default::default()
        };
        store.apply(std::slice::from_ref(&op), &options)?;
        super::media::transcript(store.project(), &args)
    };
    if long {
        return Ok(super::background::Background::spawn(
            projects,
            "transcribe",
            &project,
            Box::new(work),
            Some(Box::new(finish)),
        ));
    }
    let words = work()?;
    finish(projects.lookup_id(&project)?, words)
}

/// Native speech recognition (worker-safe): words in source frames as transcript.set wants.
#[cfg(target_os = "macos")]
fn recognize(audio: &Path, language: &str, fps: f64, frames: Frame) -> Result<Json, StoreError> {
    let words = apple_sys::speech::transcribe(audio, language).map_err(|error| {
        failure(
            "TRANSCRIPTION_FAILED",
            &error.to_string(),
            "Check that the clip has speech; the first run downloads the language model.",
        )
    })?;
    let mut last = 0;
    let mut list = Vec::with_capacity(words.len());
    for apple_sys::speech::Word { text, start, end } in words {
        let from = ((start * fps).round() as Frame).max(last);
        let to = ((end * fps).round() as Frame).max(from + 1).min(frames);
        if to <= from {
            continue;
        }
        last = to;
        list.push(super::object(vec![
            ("text", Json::String(text)),
            ("from", Json::Signed(i64::from(from))),
            ("to", Json::Signed(i64::from(to))),
        ]));
    }
    if list.is_empty() {
        return Err(failure(
            "NO_SPEECH",
            "No words were recognized in the clip's audio.",
            "Check the microphone track, or pass language for a non-English take.",
        ));
    }
    Ok(Json::Array(list))
}

// ── 0.3 native analysis and inputs filled by `enrich` (macOS: `native.rs`) ──

#[cfg(target_os = "macos")]
pub(super) use super::native::{analyze, follow_face, sync_offset, track_mask, translate_captions};

/// Peaks, beats, sounds, shots, text, faces, people and mattes need native analysis.
#[cfg(not(target_os = "macos"))]
pub(super) fn analyze(
    _: &Path,
    _: &splice_model::Media,
    _: &super::analyze::Window,
    kinds: &[&str],
) -> Result<Vec<(&'static str, Json)>, StoreError> {
    Err(failure(
        "UNSUPPORTED_PLATFORM",
        &format!(
            "{} need the native macOS analysis backend.",
            kinds.join(", ")
        ),
        "Ask for loudness and pauses only, or run on macOS.",
    ))
}

/// Tracked mask centers for mask.track.
#[cfg(not(target_os = "macos"))]
pub(super) fn track_mask(
    _: &Path,
    _: &splice_model::Project,
    _: &[(String, Json)],
) -> Result<Json, StoreError> {
    Err(failure(
        "UNSUPPORTED_PLATFORM",
        "Region tracking needs the native Vision backend.",
        "Pass keys: [{at, x, y}] mask centers explicitly.",
    ))
}

/// Crop keys that keep a face in frame.
#[cfg(not(target_os = "macos"))]
pub(super) fn follow_face(
    _: &Path,
    _: &splice_model::Project,
    _: &str,
    _: Option<&str>,
) -> Result<Json, StoreError> {
    Err(failure(
        "UNSUPPORTED_PLATFORM",
        "Face following needs the native Vision backend.",
        "Use follow:\"cursor\" or pass keys.",
    ))
}

/// Audio offset for audio.sync, measured by envelope cross-correlation.
#[cfg(not(target_os = "macos"))]
pub(super) fn sync_offset(
    _: &Path,
    _: &splice_model::Project,
    _: &[(String, Json)],
) -> Result<splice_model::Frame, StoreError> {
    Err(failure(
        "UNSUPPORTED_PLATFORM",
        "Measuring sync needs the native audio decoder.",
        "Pass offset in source frames explicitly.",
    ))
}

/// On-device translation of a captions lane, one text per cue.
#[cfg(not(target_os = "macos"))]
pub(super) fn translate_captions(
    _: &splice_model::Project,
    _: &[(String, Json)],
) -> Result<Json, StoreError> {
    Err(failure(
        "NEEDS_TEXT",
        "On-device translation needs the macOS Translation framework.",
        "Pass texts: one translated string per cue (transcript.get {captions:\"json\"} lists cues).",
    ))
}
