use crate::{
    Json,
    capture_agent::{self, CAPTURE_FPS},
    object,
    platform::record_write_options,
    project::Projects,
    record_publish::{publish, recoverable, retry_stop, stage_path, storage, summary},
    record_windows::{self, Layer},
    text,
    wire::{Args, bad, display, extend, failure, now, rev, write_options},
};
use splice_core::history::{Source, StoreError, WriteOptions};
use splice_model::{
    AudioTrack, Frame, JobId, Media, MediaId, MediaKind, Probe, SessionId, TakeComponents,
};
use std::{
    fs::{self, File},
    io::{self, Read},
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::Path,
};

pub(super) struct Job {
    pub id: JobId,
    project: String,
    session: SessionId,
    insert: bool,
    still: bool,
    /// Where the take lands (T22.2–T22.3).
    placement: Placement,
    terminal: Option<Json>,
    /// The agent's `ready` reply once the capture stopped, kept until the take is published so
    /// a failed publish can be retried (M16).
    stopped: Option<Json>,
    peer: Option<UnixStream>,
}

/// How a finished take joins the timeline.
#[derive(Clone, Default)]
struct Placement {
    /// Voice-over: the take's audio as a sound on a new audio lane at this frame.
    voiceover: Option<Frame>,
    /// Retake: right after this clip, named after it.
    retake_of: Option<splice_model::ClipId>,
    /// Camera-only: the screen component starts off.
    no_screen: bool,
    /// A project this recording created: its canvas shape (`auto` picks the nearest to the
    /// recorded picture at publish).
    canvas: Option<String>,
    fresh: bool,
    /// A multi-window take shows only the window clicked last (Recorder's focus shots).
    focus: bool,
}

impl Placement {
    fn parse(args: &Json) -> Result<Self, StoreError> {
        let voiceover = args
            .flag("voiceover")?
            .then(|| args.frame("at").map(|at| at.unwrap_or(0)))
            .transpose()?;
        let retake_of = args
            .optional_string("retakeOf")?
            .map(|id| id.parse().map_err(|_| bad("retakeOf", "a take clip ID")))
            .transpose()?;
        let no_screen = args.field("screen").and_then(Json::as_bool) == Some(false);
        // A voice-over never records the screen, so screen:false is redundant there, not wrong.
        if no_screen && voiceover.is_none() && !args.flag("camera")? {
            return Err(bad(
                "screen",
                "false only with camera:true (a camera-only take) or voiceover:true",
            ));
        }
        let focus = args.flag("focus")?;
        if focus
            && args
                .field("source")
                .and_then(|source| source.field("windows"))
                .is_none()
        {
            return Err(bad(
                "focus",
                "true only with source.windows (a multi-window take)",
            ));
        }
        Ok(Self {
            voiceover,
            retake_of,
            no_screen,
            canvas: None,
            fresh: false,
            focus,
        })
    }

    /// Ops before and after the take's clip.insert, and extra clip.insert arguments.
    fn ops(
        &self,
        project: &splice_model::Project,
        markers: &[(f64, String)],
        [width, height]: [u32; 2],
    ) -> (Vec<Json>, Vec<(&'static str, Json)>, Vec<Json>) {
        let mut before = Vec::new();
        let mut settings = vec![("cmd", text("project.set"))];
        if let Some(canvas) = &self.canvas {
            let aspect = match canvas.as_str() {
                "auto" => crate::lints::nearest_aspect(width, height),
                aspect => aspect,
            };
            settings.push(("aspect", text(aspect)));
        }
        if self.fresh {
            settings.push(("fps", Json::Unsigned(u64::from(CAPTURE_FPS))));
        }
        if settings.len() > 1 {
            before.push(object(settings));
        }
        let mut insert = vec![("ref", text("take"))];
        let mut after = Vec::new();
        let spine_end = project
            .clips
            .order()
            .last()
            .and_then(|&s| project.clips.timing(s))
            .map_or(0, |t| t.start + t.length);
        let mut start = spine_end;
        if let Some(at) = self.voiceover {
            before.push(object(vec![
                ("cmd", text("lane.create")),
                ("kind", text("audio")),
                ("name", text("Voice-over")),
                ("ref", text("vo")),
            ]));
            insert.push(("lane", text("$vo")));
            insert.push(("at", Json::Signed(i64::from(at))));
            start = at;
        } else if let Some(clip) = self.retake_of
            && let Some(slot) = project.clips.find(clip)
        {
            let t = project.clips.timing(slot).expect("live clip");
            start = t.start + t.length;
            insert.push(("at", Json::Signed(i64::from(start))));
            after.push(object(vec![
                ("cmd", text("item.set")),
                ("items", Json::Array(vec![text("$take")])),
                ("name", text(&format!("Retake of {clip}"))),
            ]));
        }
        // A voice-over is a sound item, not a spine clip: it has no screen to switch off (found
        // live on a Mac with a microphone: voiceover with screen:false failed to publish).
        if self.no_screen && self.voiceover.is_none() {
            after.push(object(vec![
                ("cmd", text("component.set")),
                ("clip", text("$take")),
                ("component", text("screen")),
                ("enabled", Json::Bool(false)),
            ]));
        }
        let fps = f64::from(if self.fresh { CAPTURE_FPS } else { project.fps });
        for (seconds, name) in markers {
            let at = start + (seconds * fps).round() as Frame;
            if !project.markers.iter().any(|m| m.at == at) {
                after.push(object(vec![
                    ("cmd", text("marker.add")),
                    ("at", Json::Signed(i64::from(at))),
                    ("name", text(name)),
                ]));
            }
        }
        (before, insert, after)
    }
}

impl Job {
    /// Still recording or publishing into project `id`.
    pub(super) fn active(&self, id: &str) -> bool {
        self.project == id && self.terminal.is_none()
    }
}

pub(super) fn start(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let insert = args.flag("insert")? || args.field("insert").is_none();
    let still = args.flag("still")?;
    let mut placement = Placement::parse(args)?;
    let aspect = args.optional_string("aspect")?;
    if let Some(aspect) = aspect
        && aspect != "auto"
        && !splice_model::Aspect::ALL
            .iter()
            .any(|(_, name)| *name == aspect)
    {
        return Err(bad("aspect", "auto, 9:16, 4:5, 1:1, 4:3, 16:9 or 16:10"));
    }
    capture_agent::request("take.check", args)?;
    let project = projects.info(args)?.string("projectId")?.to_owned();
    let store = projects.lookup_id(&project)?;
    // record.start writes nothing until record.stop: a session is enough, ifRevision optional.
    let explicit = record_write_options(store, args)?;
    // A take into an empty project sets its canvas: `aspect` when given, else the source's
    // shape (A7) unless someone already chose a canvas (any batch on an empty project, e.g.
    // project.set) and this call did not create the project.
    let empty = store.project().clips.order().is_empty()
        && store.project().media.is_empty()
        && placement.voiceover.is_none();
    if aspect.is_some() && !empty {
        return Err(bad(
            "aspect",
            "omitted when the project already has media; change the canvas with edit.apply project.set {aspect}",
        ));
    }
    let untouched = args.flag("create")? || store.project().rev == 0;
    // create:true also opens an existing package, whose chosen rate should be kept.
    placement.fresh = empty && store.project().rev == 0;
    placement.canvas = match aspect {
        Some(aspect) => Some(aspect.to_owned()),
        None => (empty && untouched).then(|| "auto".to_owned()),
    };
    let kept = store.project().size;
    // A retake of a clip that is gone would silently append at the end: refuse before capture.
    if let Some(clip) = placement.retake_of
        && store.project().clips.find(clip).is_none()
    {
        return Err(bad(
            "retakeOf",
            "a clip on the timeline (timeline.snapshot lists clip IDs)",
        ));
    }
    let session = match explicit {
        Some(options) => {
            store.check_write(&options)?;
            options.session.expect("agent write options")
        }
        None => store.start_session("Screen recording", now())?.id,
    };
    let path = store.path().canonicalize().map_err(storage)?;
    let package = path
        .to_str()
        .ok_or_else(|| bad("path", "a UTF-8 .splice package path"))?;
    let mut request = args.clone();
    if let Json::Object(fields) = &mut request {
        fields.retain(|(key, _)| key != "package");
        fields.push(("package".into(), text(package)));
    }
    let (reply, peer) = capture_agent::begin(&request).inspect_err(|_| {
        if args.field("sessionId").is_none() {
            let _ = store.end_session(session);
        }
    })?;
    let id = reply
        .string("jobId")?
        .parse()
        .map_err(|_| bad("jobId", "a capture agent job ID"))?;
    // A take without a picture reports no picture source (the agent records a display by
    // default, which a voice-over or screen:false take does not keep).
    let reply = if placement.no_screen || placement.voiceover.is_some() {
        without_source(public_status(reply))
    } else {
        public_status(reply)
    };
    let reply = extend(
        with_canvas(reply, placement.canvas.as_deref(), kept),
        vec![
            ("projectId", text(&project)),
            ("sessionId", display(&session)),
        ],
    );
    projects.recordings.push(Job {
        id,
        project,
        session,
        insert,
        still,
        placement,
        terminal: None,
        stopped: None,
        peer: Some(peer),
    });
    Ok(reply)
}

/// `record.start {recover:true}`: publishes takes left staged by a crash (fragmented writing
/// keeps them readable up to the last fragment, T22.4).
pub(super) fn recover(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let options = write_options(args)?;
    let store = projects.lookup(args)?;
    store.check_write(&options)?;
    let takes = store.path().join("takes");
    let mut staged: Vec<JobId> = fs::read_dir(&takes)
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().into_string().ok()?;
                    let id = name.strip_prefix('.')?.parse::<JobId>().ok()?;
                    entry.path().join("screen.mov").is_file().then_some(id)
                })
                .collect()
        })
        .unwrap_or_default();
    staged.sort_unstable_by_key(|id| id.serial());
    let busy: Vec<JobId> = projects
        .recordings
        .iter()
        .filter(|j| j.terminal.is_none())
        .map(|j| j.id)
        .collect();
    let store = projects.lookup(args)?;
    let mut recovered = Vec::new();
    for id in staged.into_iter().filter(|id| !busy.contains(id)) {
        // Another session's capture may still be writing this take (M16).
        if !recoverable(capture_agent::take_state(id).as_ref()) {
            continue;
        }
        let stage = stage_path(store.path(), id);
        let size = fs::read_dir(&stage)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0);
        let descriptor = describe(&stage, store.project().fps, false, size)?;
        // A multi-window take keeps its layers; the bounds come from its events, when recorded.
        let bounds = fs::read_to_string(stage.join("events.json"))
            .ok()
            .and_then(|events| splice_json::parse(&events).ok());
        let source = bounds
            .as_ref()
            .and_then(|events| events.field("source")?.field("windows"));
        let (layers, windows) = windows(&stage, store.project().fps, &descriptor, source)?;
        let mut ops: (Vec<Json>, Vec<(&'static str, Json)>, Vec<Json>) = Default::default();
        if !layers.is_empty() {
            let (plan, _) = arrange(store.project(), &layers, None);
            ops.1.push(("ref", text("take")));
            ops.2 = record_windows::ops(&layers, &plan, store.project().fps);
        }
        let sealed = publish(store, id, true, descriptor, windows, &options, ops)?;
        recovered.push(object(vec![
            ("mediaId", display(&sealed.media)),
            (
                "clipId",
                sealed.clip.map_or(Json::Null, |clip| display(&clip)),
            ),
        ]));
    }
    Ok(object(vec![
        ("state", text("done")),
        ("revision", rev(store.project().rev)),
        ("recovered", Json::Array(recovered)),
    ]))
}

pub(super) fn command(
    projects: &mut Projects,
    id: JobId,
    method: &str,
    args: &Json,
) -> Result<Json, StoreError> {
    let index = projects
        .recordings
        .iter()
        .position(|job| job.id == id)
        .ok_or_else(|| super::platform::missing_job(id))?;
    let job = &projects.recordings[index];
    let owner = job.project.clone();
    if args
        .optional_string("projectId")?
        .is_some_and(|project| project != owner)
    {
        return Err(super::platform::missing_job(id));
    }
    let session = job.session;
    let insert = args
        .field("insert")
        .map_or(Ok(job.insert), |_| args.flag("insert"))?;
    let still = job.still;
    let mut placement = job.placement.clone();
    let terminal = job.terminal.clone();
    let stopped = job.stopped.clone().filter(|_| method == "take.stop");
    let store = projects.lookup_id(&owner)?;
    if let Some(reply) = terminal {
        return Ok(reply);
    }
    let mut request = object(vec![("jobId", display(&id))]);
    if let Json::Object(fields) = &mut request {
        // marker for take.mark; duration and then for take.resume (record.start {resume}).
        for key in ["marker", "duration", "then"] {
            if let Some(value) = args.field(key) {
                fields.push((key.into(), value.clone()));
            }
        }
    }
    if matches!(
        method,
        "take.status" | "take.pause" | "take.resume" | "take.mark"
    ) {
        return capture_agent::request(method, &request).map(public_status);
    }
    let options = WriteOptions {
        source: Source::Agent,
        session: Some(if args.field("sessionId").is_some() {
            args.session()?
        } else {
            session
        }),
        if_revision: Some(if args.field("ifRevision").is_some() {
            args.revision("ifRevision")?
        } else {
            store.project().rev
        }),
        label: (method == "take.stop").then(|| {
            if still {
                "Agent: Capture screenshot".into()
            } else {
                "Agent: Save recording".into()
            }
        }),
        time: now(),
        ..Default::default()
    };
    store.check_write(&options)?;
    let reply = match stopped {
        Some(reply) => reply,
        None => public_status(capture_agent::request(method, &request)?),
    };
    let reply = if method == "take.stop" && reply.string("state")? == "ready" {
        projects.recordings[index].stopped = Some(reply.clone());
        let store = projects.lookup_id(&owner)?;
        let size = reply
            .field("bytes")
            .and_then(Json::as_u64)
            .ok_or_else(|| bad("bytes", "capture output size"))?;
        let stage = stage_path(store.path(), id);
        placement.fresh &= store.project().rev == 0;
        let fps = if placement.fresh {
            CAPTURE_FPS
        } else {
            store.project().fps
        };
        let descriptor = describe(&stage, fps, still, size).map_err(retry_stop)?;
        let source = reply
            .field("settings")
            .and_then(|settings| settings.field("source")?.field("windows"));
        let (layers, windows) = windows(&stage, fps, &descriptor, source).map_err(retry_stop)?;
        let silent = descriptor
            .take
            .as_ref()
            .is_some_and(|take| take.mic.is_none() && take.system.is_none());
        // Focus shots show one window at a time, each fitted like the first.
        let picture = if layers.is_empty() || placement.focus {
            [descriptor.probe.width, descriptor.probe.height]
        } else {
            record_windows::picture(&layers)
        };
        let frames = ["videoFrames", "droppedFrames"]
            .map(|key| reply.field(key).and_then(Json::as_u64).unwrap_or(0));
        let markers: Vec<(f64, String)> = reply
            .field("markers")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some((
                    m.field("seconds")?.as_f64()?,
                    m.field("name")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        let mut ops = placement.ops(store.project(), &markers, picture);
        let arranged = (!layers.is_empty()).then(|| {
            let aspect = placement.canvas.as_deref().map(|canvas| match canvas {
                "auto" => crate::lints::nearest_aspect(picture[0], picture[1]),
                aspect => aspect,
            });
            let (plan, canvas) = arrange(store.project(), &layers, aspect);
            if !placement.focus {
                return (plan, canvas);
            }
            // Focus shots: from the clicks in the take's event log (window 0 only without one).
            let events = fs::read_to_string(stage.join("events.json"))
                .ok()
                .and_then(|events| splice_json::parse(&events).ok())
                .unwrap_or(Json::Null);
            let shots = record_windows::shots(&events, layers.len());
            (record_windows::focus(plan, &layers, canvas, shots), canvas)
        });
        if let Some((plan, _)) = &arranged {
            ops.2.extend(record_windows::ops(&layers, plan, fps));
        }
        let sealed =
            publish(store, id, insert, descriptor, windows, &options, ops).map_err(retry_stop)?;
        let mic_peak = store
            .project()
            .media
            .iter()
            .find(|media| media.id == sealed.media)
            .and_then(|media| media.take.as_ref()?.mic.as_ref())
            .and_then(|mic| {
                let path = store
                    .path()
                    .join("takes")
                    .join(sealed.media.to_string())
                    .join(mic);
                // Loudest 10 ms RMS window.
                apple_sys::audio::envelope(&path, 0, 100.0)
                    .ok()?
                    .into_iter()
                    .reduce(f32::max)
            });
        let (take, lints, pacing) = summary(
            store,
            &sealed,
            frames,
            mic_peak,
            arranged
                .as_ref()
                .map(|(plan, canvas)| (layers.as_slice(), plan, *canvas)),
        );
        let Json::Object(mut fields) = reply else {
            unreachable!("job status object")
        };
        fields.retain(|(key, _)| key != "state");
        let media = sealed.media;
        let mut reply = extend(
            Json::Object(fields),
            vec![
                ("state", text("done")),
                ("revision", rev(sealed.revision)),
                ("mediaId", display(&media)),
                (
                    "clipId",
                    sealed.clip.map_or(Json::Null, |clip| display(&clip)),
                ),
                ("take", take),
                ("lints", Json::Array(lints)),
                ("pacing", pacing),
            ],
        );
        if silent && !still {
            reply = extend(
                reply,
                vec![(
                    "warning",
                    text(
                        "This take has no audio. Add music with clip.insert, or record with mic or systemAudio.",
                    ),
                )],
            );
        }
        if still {
            reply = extend(
                reply,
                vec![("output", text(&format!("takes/{media}/screen.png")))],
            );
        }
        reply
    } else {
        reply
    };
    projects.recordings[index].terminal = Some(reply.clone());
    projects.recordings[index].peer = None;
    Ok(reply)
}

/// The start receipt with `settings.source` null.
fn without_source(reply: Json) -> Json {
    let Json::Object(mut fields) = reply else {
        return reply;
    };
    for (key, value) in &mut fields {
        if key == "settings"
            && let Json::Object(settings) = value
        {
            for (name, source) in settings.iter_mut() {
                if name == "source" {
                    *source = Json::Null;
                }
            }
        }
    }
    Json::Object(fields)
}

/// Adds the canvas decision to the start receipt's settings: `auto` (the shape nearest the
/// source, decided from the recorded pixels at stop; `aspect` here is the estimate),
/// `explicit`, or `kept` for a project that already had content or was not created here.
fn with_canvas(mut reply: Json, canvas: Option<&str>, kept: [u32; 2]) -> Json {
    let size = |[w, h]: [u32; 2]| {
        Json::Array(vec![
            Json::Unsigned(u64::from(w)),
            Json::Unsigned(u64::from(h)),
        ])
    };
    let estimate = reply
        .field("settings")
        .and_then(|settings| settings.field("source")?.field("size")?.as_array())
        .and_then(|size| match size {
            [w, h] => Some([
                u32::try_from(w.as_u64()?).ok()?,
                u32::try_from(h.as_u64()?).ok()?,
            ]),
            _ => None,
        });
    let decision = match canvas {
        None => object(vec![
            ("decision", text("kept")),
            (
                "aspect",
                text(crate::lints::nearest_aspect(kept[0], kept[1])),
            ),
            ("size", size(kept)),
        ]),
        Some("auto") => object(vec![
            ("decision", text("auto")),
            (
                "aspect",
                estimate.map_or(Json::Null, |[w, h]| {
                    text(crate::lints::nearest_aspect(w, h))
                }),
            ),
        ]),
        Some(aspect) => object(vec![
            ("decision", text("explicit")),
            ("aspect", text(aspect)),
        ]),
    };
    if let Json::Object(fields) = &mut reply
        && let Some((_, Json::Object(settings))) =
            fields.iter_mut().find(|(key, _)| key == "settings")
    {
        settings.push(("canvas".into(), decision));
    }
    reply
}

fn public_status(mut status: Json) -> Json {
    if let Json::Object(fields) = &mut status {
        fields.retain(|(key, _)| key != "staged");
    }
    status
}

fn describe(stage: &Path, fps: u32, still: bool, size: u64) -> Result<Media, StoreError> {
    let primary = stage.join(if still { "screen.png" } else { "screen.mov" });
    let metadata = fs::symlink_metadata(&primary).map_err(storage)?;
    if !metadata.is_file() || !fs::symlink_metadata(stage).map_err(storage)?.is_dir() {
        return Err(bad_capture("The staged screen file is not a regular file."));
    }
    let (probe, take) = if still {
        let [width, height] = png_size(&primary)?;
        (
            Probe {
                fps: f64::from(fps),
                frames: 3,
                width,
                height,
                codec: Some("png".into()),
                loudness: None,
                audio: Vec::new(),
            },
            None,
        )
    } else {
        let mut probe = movie(&primary, fps)?;
        let take = TakeComponents {
            screen: Some("screen.mov".into()),
            mic: component(stage, "mic.m4a", false)?,
            system: component(stage, "system.m4a", false)?,
            camera: component(stage, "camera.mov", false)?,
            events: component(stage, "events.json", false)?,
            cursors: component(stage, "cursors", true)?,
            pointer: Vec::new(),
        };
        probe.audio = [("mic", &take.mic), ("system", &take.system)]
            .into_iter()
            .filter(|(_, component)| component.is_some())
            .map(|(name, _)| AudioTrack {
                name: name.into(),
                channels: Some(2),
            })
            .collect();
        (probe, Some(take))
    };
    Ok(Media {
        id: MediaId::from_serial(0),
        kind: if still {
            MediaKind::Image
        } else {
            MediaKind::Take
        },
        path: String::new(),
        bookmark: None,
        size,
        mtime: metadata.mtime(),
        probe,
        take,
    })
}

/// A staged movie's picture and length at the project rate (`fps`).
fn movie(path: &Path, fps: u32) -> Result<Probe, StoreError> {
    let asset = apple_sys::media::probe(path).map_err(|error| bad_capture(&error.to_string()))?;
    let video = asset
        .video
        .ok_or_else(|| bad_capture("A staged screen or window movie has no video track."))?;
    let frames = (asset.duration_seconds * f64::from(fps)).round();
    if !frames.is_finite() || frames > f64::from(Frame::MAX) || asset.duration_seconds <= 0.0 {
        return Err(bad_capture(
            "A staged screen or window movie has an unsupported duration.",
        ));
    }
    Ok(Probe {
        fps: f64::from(fps),
        frames: frames.max(1.0) as Frame,
        width: video.width,
        height: video.height,
        codec: video.codec.map(|code| {
            if matches!(&code, b"hvc1" | b"hev1") {
                "hevc".into()
            } else {
                String::from_utf8_lossy(&code).into_owned()
            }
        }),
        audio: Vec::new(),
        loudness: None,
    })
}

/// The windows of a multi-window take (T46.6): every `window-<n>.mov` staged beside the screen,
/// as layers (the screen first) and as video media descriptors whose `path` is the file name.
/// `bounds` lists each window's `win_` ID and bounds in file order (the start receipt's
/// `settings.source.windows`, or `source.windows` in events.json); without them the layers
/// stand in a row. A single-source take has none.
fn windows(
    stage: &Path,
    fps: u32,
    screen: &Media,
    bounds: Option<&Json>,
) -> Result<(Vec<Layer>, Vec<Media>), StoreError> {
    let files: Vec<String> = (1..*crate::record_options::WINDOWS.end())
        .map(|n| format!("window-{n}.mov"))
        .take_while(|file| stage.join(file).is_file())
        .collect();
    if files.is_empty() || screen.take.is_none() {
        return Ok((Vec::new(), Vec::new()));
    }
    let listed = bounds.and_then(Json::as_array).unwrap_or(&[]);
    let layer = |n: usize, file: &str, probe: &Probe| {
        let entry = listed.get(n);
        let rect = entry.map(|entry| entry.field("rect").unwrap_or(entry));
        let side = |key| rect.and_then(|rect| rect.field(key)).and_then(Json::as_f64);
        Layer {
            file: file.into(),
            window: entry.and_then(|entry| {
                entry
                    .field("id")?
                    .as_str()?
                    .strip_prefix("win_")?
                    .parse()
                    .ok()
            }),
            rect: match (side("x"), side("y"), side("width"), side("height")) {
                (Some(x), Some(y), Some(w), Some(h)) => Some([x, y, w, h]),
                _ => None,
            },
            size: [probe.width, probe.height],
            frames: probe.frames,
        }
    };
    let mut layers = vec![layer(0, "screen.mov", &screen.probe)];
    let mut media = Vec::new();
    for (n, file) in files.iter().enumerate() {
        let path = stage.join(file);
        let probe = movie(&path, fps)?;
        layers.push(layer(n + 1, file, &probe));
        media.push(Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Video,
            path: file.clone(),
            bookmark: None,
            size: fs::metadata(&path).map_err(storage)?.len(),
            mtime: fs::metadata(&path).map_err(storage)?.mtime(),
            probe,
            take: None,
        });
    }
    Ok((layers, media))
}

/// Where a multi-window take's layers land on the canvas this batch leaves (`aspect` when it
/// sets one), inside the spine's window look; and that canvas.
fn arrange(
    project: &splice_model::Project,
    layers: &[Layer],
    aspect: Option<&str>,
) -> (record_windows::Plan, [f64; 2]) {
    let canvas = record_windows::canvas(project, aspect);
    let window = splice_eval::resolve_effect(&[&project.effects, &project.spine.effects], "window");
    (
        record_windows::plan(layers, canvas, window.as_ref()),
        canvas,
    )
}

fn component(stage: &Path, name: &str, directory: bool) -> Result<Option<String>, StoreError> {
    match fs::symlink_metadata(stage.join(name)) {
        Ok(meta) if (directory && meta.is_dir()) || (!directory && meta.is_file()) => {
            Ok(Some(name.into()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage(error)),
        _ => Err(bad_capture(
            "A staged take component has the wrong file type.",
        )),
    }
}

fn png_size(path: &Path) -> Result<[u32; 2], StoreError> {
    let mut header = [0u8; 24];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(storage)?;
    let width = u32::from_be_bytes(header[16..20].try_into().expect("PNG width"));
    let height = u32::from_be_bytes(header[20..24].try_into().expect("PNG height"));
    if !header.starts_with(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR") || width == 0 || height == 0 {
        return Err(bad_capture("The staged screenshot is not a valid PNG."));
    }
    Ok([width, height])
}

fn bad_capture(message: &str) -> StoreError {
    failure(
        "BAD_MEDIA",
        message,
        "Check the capture source and retry the recording.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestDirectory;
    use splice_core::history::Store;
    use splice_model::Project;

    #[test]
    fn capture_agent_staged_movie_and_still_publish_with_native_geometry() {
        let temp = TestDirectory::new("capture-publish-native");
        let package = temp.0.join("capture.splice");
        let mut store = Store::create(&package, &Project::default()).unwrap();
        let video = JobId::from_serial(1);
        let stage = stage_path(store.path(), video);
        fs::create_dir_all(&stage).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media/probe.mov"),
            stage.join("screen.mov"),
        )
        .unwrap();
        fs::write(stage.join("events.json"), br#"{"events":[]}"#).unwrap();
        let movie = describe(&stage, 30, false, 3000).unwrap();
        assert_eq!(
            (movie.probe.width, movie.probe.height, movie.probe.frames),
            (96, 64, 30)
        );
        assert_eq!(
            movie.take.as_ref().unwrap().events.as_deref(),
            Some("events.json")
        );
        publish(
            &mut store,
            video,
            true,
            movie,
            Vec::new(),
            &WriteOptions::default(),
            Default::default(),
        )
        .unwrap();

        let still = JobId::from_serial(2);
        let stage = stage_path(store.path(), still);
        fs::create_dir(&stage).unwrap();
        let png = apple_sys::image::png(&[255; 32], [4, 2], 0).unwrap();
        fs::write(stage.join("screen.png"), &png).unwrap();
        let image = describe(&stage, 30, true, png.len() as u64).unwrap();
        assert_eq!(image.kind, MediaKind::Image);
        assert_eq!(
            (image.probe.width, image.probe.height, image.probe.frames),
            (4, 2, 3)
        );
        let receipt = publish(
            &mut store,
            still,
            true,
            image,
            Vec::new(),
            &WriteOptions::default(),
            Default::default(),
        )
        .unwrap();
        assert!(
            package
                .join(format!("takes/{}/screen.png", receipt.media))
                .is_file()
        );
        assert!(receipt.clip.is_some());
    }

    #[test]
    fn capture_agent_placement_sets_only_fresh_projects_to_the_capture_rate() {
        for (rev, expected_fps) in [(0, 60), (1, 30)] {
            let mut project = Project {
                rev,
                ..Project::default()
            };
            let placement = Placement {
                fresh: rev == 0,
                canvas: Some("16:10".into()),
                ..Placement::default()
            };
            let markers = [(0.5, "Step".into())];
            let (before, _, after) = placement.ops(&project, &markers, [1920, 1200]);
            assert_eq!(before.len(), 1);
            assert_eq!(
                before[0].field("fps").and_then(Json::as_u64),
                (rev == 0).then_some(60)
            );
            splice_core::edit::apply(&mut project, &before, Source::Agent, false).unwrap();
            assert_eq!(project.fps, expected_fps);
            assert_eq!(project.aspect, splice_model::Aspect::Wide);
            assert_eq!(
                after[0].field("at").and_then(Json::as_i64),
                Some(i64::from(expected_fps / 2))
            );
        }
    }

    /// Where a take lands (Recorder's voiceover at the playhead, retake, resume marker and
    /// camera-only recording): a voice-over on a new audio lane at `at`, a retake right after
    /// the clip it redoes and named after it, resume markers at their recorded second from the
    /// take's start, and a camera-only take with its screen off.
    #[test]
    fn capture_agent_placement_puts_voiceovers_retakes_markers_and_camera_only_takes() {
        let project = crate::tests::sample_project();
        let cmds = |ops: &[Json]| -> Vec<String> {
            ops.iter()
                .map(|op| op.field("cmd").and_then(Json::as_str).unwrap().to_owned())
                .collect()
        };
        let field = |fields: &[(&'static str, Json)], key: &str| {
            fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };
        let markers = [(2.0, "Step 2".to_owned())];
        let voiceover = Placement {
            voiceover: Some(90),
            ..Placement::default()
        };
        let (before, insert, after) = voiceover.ops(&project, &markers, [1920, 1080]);
        assert_eq!(cmds(&before), ["lane.create"]);
        assert_eq!(
            before[0].field("kind").and_then(Json::as_str),
            Some("audio")
        );
        assert_eq!(field(&insert, "lane"), Some(text("$vo")));
        assert_eq!(field(&insert, "at"), Some(Json::Signed(90)));
        assert_eq!(cmds(&after), ["marker.add"]);
        assert_eq!(after[0].field("at"), Some(&Json::Signed(90 + 60)));
        // Found live on a Mac with a microphone: voiceover with screen:false switched the
        // sound item's "screen" off and failed to publish (WRONG_KIND).
        let silent_screen = Placement {
            voiceover: Some(90),
            no_screen: true,
            ..Placement::default()
        };
        let (_, _, after) = silent_screen.ops(&project, &[], [1920, 1080]);
        assert!(cmds(&after).is_empty(), "{after:?}");

        let retake = Placement {
            retake_of: Some("clp_take".parse().unwrap()),
            ..Placement::default()
        };
        let (before, insert, after) = retake.ops(&project, &markers, [1920, 1080]);
        assert!(before.is_empty());
        // clp_take covers 120..540.
        assert_eq!(field(&insert, "at"), Some(Json::Signed(540)));
        assert_eq!(cmds(&after), ["item.set", "marker.add"]);
        assert_eq!(
            after[0].field("name").and_then(Json::as_str),
            Some("Retake of clp_take")
        );
        assert_eq!(after[1].field("at"), Some(&Json::Signed(540 + 60)));

        let camera = Placement {
            no_screen: true,
            ..Placement::default()
        };
        let (_, insert, after) = camera.ops(&project, &[], [1920, 1080]);
        // Appended at the spine's end (660).
        assert_eq!(field(&insert, "at"), None);
        assert_eq!(cmds(&after), ["component.set"]);
        assert_eq!(
            after[0].field("component").and_then(Json::as_str),
            Some("screen")
        );
        assert_eq!(after[0].field("enabled"), Some(&Json::Bool(false)));
        // camera-only needs camera:true; a voice-over never records the screen.
        assert!(Placement::parse(&splice_json::parse(r#"{"screen":false}"#).unwrap()).is_err());
        assert!(
            Placement::parse(&splice_json::parse(r#"{"screen":false,"camera":true}"#).unwrap())
                .is_ok()
        );
        assert_eq!(
            Placement::parse(&splice_json::parse(r#"{"voiceover":true,"at":45}"#).unwrap())
                .unwrap()
                .voiceover,
            Some(45)
        );
    }

    /// A staged multi-window take (T46.6) as the capture agent leaves it: the screen, two more
    /// windows and events with every window's bounds. Live capture needs the installed app, so
    /// the movies are the probe fixture.
    #[test]
    fn capture_agent_multi_window_take_publishes_every_window_as_a_layer() {
        let temp = TestDirectory::new("capture-publish-windows");
        let mut store = Store::create(&temp.0.join("w.splice"), &Project::default()).unwrap();
        let id = JobId::from_serial(3);
        let stage = stage_path(store.path(), id);
        fs::create_dir_all(&stage).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media/probe.mov");
        for file in ["screen.mov", "window-1.mov", "window-2.mov"] {
            fs::copy(&fixture, stage.join(file)).unwrap();
        }
        fs::write(
            stage.join("events.json"),
            br#"{"t0":1,"source":{"kind":"window","rect":{"x":0,"y":0,"width":48,"height":32},"scale":2,"windows":[{"x":0,"y":0,"width":48,"height":32},{"x":60,"y":0,"width":48,"height":32},{"x":0,"y":40,"width":48,"height":32}]},"events":[]}"#,
        )
        .unwrap();
        let screen = describe(&stage, 30, false, 9000).unwrap();
        // The start receipt's settings name each window.
        let settings = splice_json::parse(
            r#"[{"id":"win_5","file":"screen.mov","rect":{"x":0,"y":0,"width":48,"height":32}},{"id":"win_6","file":"window-1.mov","rect":{"x":60,"y":0,"width":48,"height":32}},{"id":"win_7","file":"window-2.mov","rect":{"x":0,"y":40,"width":48,"height":32}}]"#,
        )
        .unwrap();
        let (layers, media) = windows(&stage, 30, &screen, Some(&settings)).unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(
            (layers[2].file.as_str(), layers[2].window, layers[2].rect),
            ("window-2.mov", Some(7), Some([0.0, 40.0, 48.0, 32.0]))
        );
        assert_eq!((layers[1].size, layers[1].frames), ([96, 64], 30));
        assert_eq!(media[0].kind, MediaKind::Video);
        let (plan, canvas) = arrange(store.project(), &layers, None);
        assert_eq!(plan.arrangement, "captured");
        let ops = (
            Vec::new(),
            vec![("ref", text("take"))],
            record_windows::ops(&layers, &plan, store.project().fps),
        );
        let sealed = publish(
            &mut store,
            id,
            true,
            screen,
            media,
            &WriteOptions::default(),
            ops,
        )
        .unwrap();
        let project = store.project();
        assert_eq!(project.media.len(), 3);
        let take = sealed.media;
        for (media, file) in
            project
                .media
                .iter()
                .zip(["screen.mov", "window-1.mov", "window-2.mov"])
        {
            assert_eq!(media.path, format!("takes/{take}/{file}"));
            assert!(store.path().join(&media.path).is_file());
        }
        assert_eq!(sealed.layers.len(), 3);
        assert!(sealed.layers[1].1.as_deref().unwrap().starts_with("ovl_"));
        let (summary, _, _) = crate::record_publish::summary(
            &store,
            &sealed,
            [30, 0],
            None,
            Some((&layers, &plan, canvas)),
        );
        let listed = summary.field("layers").unwrap().as_array().unwrap();
        assert_eq!(
            listed[1].field("window").and_then(Json::as_str),
            Some("win_6")
        );
        // A crash recovery without the receipt reads the bounds from events.json.
        let events = splice_json::parse(
            &fs::read_to_string(store.path().join(format!("takes/{take}/events.json"))).unwrap(),
        )
        .unwrap();
        let bounds = events
            .field("source")
            .and_then(|source| source.field("windows"));
        let stage = store.path().join(format!("takes/{take}"));
        let screen = describe(&stage, 30, false, 9000).unwrap();
        let (recovered, _) = windows(&stage, 30, &screen, bounds).unwrap();
        assert_eq!(
            recovered.iter().map(|layer| layer.rect).collect::<Vec<_>>(),
            layers.iter().map(|layer| layer.rect).collect::<Vec<_>>()
        );
        assert_eq!(recovered[1].window, None);
    }
}
