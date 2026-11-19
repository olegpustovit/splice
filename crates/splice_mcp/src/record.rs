use crate::{
    Json, capture_agent, object,
    project::Projects,
    record_publish::{publish, stage_path, storage},
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
    terminal: Option<Json>,
    peer: Option<UnixStream>,
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
    let explicit = (args.field("projectId").is_some()
        || args.field("sessionId").is_some()
        || args.field("ifRevision").is_some())
    .then(|| write_options(args))
    .transpose()?;
    capture_agent::request("take.check", args)?;
    let project = projects.info(args)?.string("projectId")?.to_owned();
    let store = projects.lookup_id(&project)?;
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
    let reply = extend(
        public_status(reply),
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
        terminal: None,
        peer: Some(peer),
    });
    Ok(reply)
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
    let terminal = job.terminal.clone();
    let store = projects.lookup_id(&owner)?;
    if let Some(reply) = terminal {
        return Ok(reply);
    }
    let request = object(vec![("jobId", display(&id))]);
    if method == "take.status" {
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
    let reply = public_status(capture_agent::request(method, &request)?);
    let reply = if method == "take.stop" && reply.string("state")? == "ready" {
        let size = reply
            .field("bytes")
            .and_then(Json::as_u64)
            .ok_or_else(|| bad("bytes", "capture output size"))?;
        let descriptor = describe(
            &stage_path(store.path(), id),
            store.project().fps,
            still,
            size,
        )?;
        let sealed = publish(store, id, insert, descriptor, &options)?;
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
            ],
        );
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
                audio: Vec::new(),
            },
            None,
        )
    } else {
        let asset =
            apple_sys::media::probe(&primary).map_err(|error| bad_capture(&error.to_string()))?;
        let screen = asset
            .video
            .ok_or_else(|| bad_capture("The staged screen movie has no video track."))?;
        let frames = (asset.duration_seconds * f64::from(fps)).round();
        if !frames.is_finite() || frames > f64::from(Frame::MAX) || asset.duration_seconds <= 0.0 {
            return Err(bad_capture(
                "The staged screen movie has an unsupported duration.",
            ));
        }
        let take = TakeComponents {
            screen: Some("screen.mov".into()),
            mic: component(stage, "mic.m4a", false)?,
            system: component(stage, "system.m4a", false)?,
            camera: component(stage, "camera.mov", false)?,
            events: component(stage, "events.json", false)?,
            cursors: component(stage, "cursors", true)?,
        };
        let audio = [("mic", &take.mic), ("system", &take.system)]
            .into_iter()
            .filter(|(_, component)| component.is_some())
            .map(|(name, _)| AudioTrack {
                name: name.into(),
                channels: Some(2),
            })
            .collect();
        (
            Probe {
                fps: f64::from(fps),
                frames: frames.max(1.0) as Frame,
                width: screen.width,
                height: screen.height,
                codec: screen.codec.map(|code| {
                    if matches!(&code, b"hvc1" | b"hev1") {
                        "hevc".into()
                    } else {
                        String::from_utf8_lossy(&code).into_owned()
                    }
                }),
                audio,
            },
            Some(take),
        )
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
        publish(&mut store, video, true, movie, &WriteOptions::default()).unwrap();

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
        let receipt = publish(&mut store, still, true, image, &WriteOptions::default()).unwrap();
        assert!(
            package
                .join(format!("takes/{}/screen.png", receipt.media))
                .is_file()
        );
        assert!(receipt.clip.is_some());
    }
}
