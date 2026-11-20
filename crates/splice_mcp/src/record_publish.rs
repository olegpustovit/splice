use crate::{Json, object, text, wire::failure};
use splice_core::history::{Store, StoreError, WriteOptions};
use splice_model::{ClipId, JobId, Media, MediaId, MediaKind};
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};

pub(super) struct Publication {
    pub revision: u64,
    pub media: MediaId,
    pub clip: Option<ClipId>,
}

/// A microphone quieter than this (loudest 10 ms RMS window, dBFS) recorded silence.
const SILENT_DB: f64 = -60.0;

/// What a published take holds (record.stop `take`, T46.1) with the lints an edit returns plus
/// `TAKE_ASPECT` and `MIC_SILENT`: frames at the project rate, seconds, pixel size, the capture
/// rate, components, clicks, keys and the microphone's peak. `frames` are the capture agent's
/// written and dropped screen frames; `mic_peak` is the loudest 10 ms RMS window as linear
/// amplitude, when the take has a microphone track that could be read.
pub(super) fn summary(
    store: &Store,
    publication: &Publication,
    frames: [u64; 2],
    mic_peak: Option<f32>,
) -> (Json, Vec<Json>, Json) {
    let project = store.project();
    let (mut lints, pacing) = crate::lints::check(project);
    let Some(media) = project.media.iter().find(|m| m.id == publication.media) else {
        return (Json::Null, lints, pacing);
    };
    let probe = &media.probe;
    let mut fields = vec![
        ("frames", Json::Signed(i64::from(probe.frames))),
        ("fps", Json::Unsigned(u64::from(project.fps))),
        (
            "seconds",
            Json::Float(
                (f64::from(probe.frames) / f64::from(project.fps.max(1)) * 1000.0).round() / 1000.0,
            ),
        ),
        (
            "size",
            Json::Array(vec![
                Json::Unsigned(u64::from(probe.width)),
                Json::Unsigned(u64::from(probe.height)),
            ]),
        ),
    ];
    if let Some(take) = &media.take {
        fields.push(("captureFps", Json::Unsigned(60)));
        fields.push(("videoFrames", Json::Unsigned(frames[0])));
        fields.push(("droppedFrames", Json::Unsigned(frames[1])));
        let tracks = [
            ("screen", &take.screen),
            ("mic", &take.mic),
            ("system", &take.system),
            ("camera", &take.camera),
            ("events", &take.events),
        ];
        fields.push((
            "tracks",
            Json::Array(
                tracks
                    .iter()
                    .filter(|(_, file)| file.is_some())
                    .map(|(name, _)| text(name))
                    .collect(),
            ),
        ));
        if let Some(events) = &take.events {
            let dir = store.path().join("takes").join(media.id.to_string());
            if let Ok((clicks, keys)) = crate::media::input_counts(&dir, events) {
                fields.push(("clicks", Json::Unsigned(clicks)));
                fields.push(("keys", Json::Unsigned(keys)));
            }
        }
        if take.mic.is_some()
            && let Some(peak) = mic_peak
        {
            let db = (20.0 * f64::from(peak).max(1e-6).log10() * 10.0).round() / 10.0;
            fields.push(("micPeakDb", Json::Float(db)));
            if db < SILENT_DB {
                fields.push(("silent", Json::Bool(true)));
                lints.push(crate::lints::mic_silent(db, media.id));
            }
        }
    }
    lints.extend(crate::lints::take_fit(project, media, publication.clip));
    (object(fields), lints, pacing)
}

pub(super) fn stage_path(package: &Path, id: JobId) -> PathBuf {
    package.join("takes").join(format!(".{id}"))
}

pub(super) fn publish(
    store: &mut Store,
    job: JobId,
    insert: bool,
    mut descriptor: Media,
    options: &WriteOptions,
    (before, extra, after): (Vec<Json>, Vec<(&'static str, Json)>, Vec<Json>),
) -> Result<Publication, StoreError> {
    store.check_write(options)?;
    let primary = if descriptor.kind == MediaKind::Image {
        "screen.png"
    } else {
        "screen.mov"
    };
    let stage = stage_path(store.path(), job);
    let takes = stage.parent().expect("stage beneath takes").to_path_buf();
    if !fs::symlink_metadata(&takes).map_err(storage)?.is_dir()
        || !fs::symlink_metadata(&stage).map_err(storage)?.is_dir()
        || !fs::symlink_metadata(stage.join(primary))
            .map_err(storage)?
            .is_file()
    {
        return Err(failure(
            "BAD_MEDIA",
            "The capture staging path does not contain a regular take.",
            "Start another recording in this project.",
        ));
    }
    let request = format!("takes/.{job}/{primary}");
    let mut fields = vec![
        ("cmd", text("clip.insert")),
        ("path", text(&request)),
        ("insert", Json::Bool(insert)),
    ];
    if insert {
        fields.extend(extra);
    }
    let mut ops = before;
    ops.push(object(fields));
    if insert {
        ops.extend(after);
    }
    descriptor.path.clone_from(&request);
    let mut preview = options.clone();
    preview.dry_run = true;
    let plan = store.apply_with_media(&ops, &preview, &mut |_| Ok(descriptor.clone()))?;
    let media = plan
        .report
        .mints
        .iter()
        .find_map(|id| id.parse::<MediaId>().ok())
        .ok_or_else(|| {
            failure(
                "EXISTS",
                "This staged take already has a media entry.",
                "Use project.info to find the existing media entry.",
            )
        })?;
    let clip = plan
        .report
        .mints
        .iter()
        .find_map(|id| id.parse::<ClipId>().ok());
    let destination = takes.join(media.to_string());
    descriptor.path = format!("takes/{media}/{primary}");
    let occupied = match fs::symlink_metadata(&destination) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(storage(error)),
    };
    if occupied
        || store
            .project()
            .media
            .iter()
            .any(|entry| entry.path == descriptor.path)
    {
        return Err(failure(
            "EXISTS",
            &format!(
                "Capture directory {} already exists.",
                destination.display()
            ),
            "Preserve the existing directory at another path and retry record.stop.",
        ));
    }
    fs::rename(&stage, &destination).map_err(storage)?;
    let result = sync_directory(&takes)
        .and_then(|()| sync_directory(store.path()))
        .and_then(|()| store.apply_with_media(&ops, options, &mut |_| Ok(descriptor.clone())));
    match result {
        Ok(edit) => {
            debug_assert_eq!(edit.report.mints, plan.report.mints);
            Ok(Publication {
                revision: edit.revision,
                media,
                clip,
            })
        }
        Err(error) => {
            fs::rename(&destination, &stage).map_err(|rollback| {
                failure(
                    "IO",
                    &format!(
                        "{error} The staged take remains at {}: {rollback}.",
                        destination.display()
                    ),
                    "Preserve the take files while checking the project filesystem.",
                )
            })?;
            sync_directory(&takes)?;
            Err(error)
        }
    }
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(storage)
}

/// A publish that failed after the capture stopped: the take stays staged and the job keeps
/// the stop, so `record.stop` can publish it again (M16).
pub(super) fn retry_stop(mut error: StoreError) -> StoreError {
    error.fix = format!(
        "{} The take stays staged: call record.stop again to publish it.",
        error.fix
    );
    error
}

/// Whether `record.start {recover:true}` may publish a staged take, from the capture agent's
/// answer about it (M16): no agent running (`None`), or an agent that no longer holds the take
/// (`NOT_FOUND`, failed or cancelled). A take it is still writing or holding (`pending`,
/// `recording`, `paused`, `ready`), or an agent that cannot answer, is left alone.
pub(super) fn recoverable(agent: Option<&Result<String, &str>>) -> bool {
    match agent {
        None => true,
        Some(Ok(state)) => state == "cancelled",
        Some(Err(code)) => *code != "CAPTURE_AGENT_UNAVAILABLE",
    }
}

pub(super) fn storage(error: io::Error) -> StoreError {
    failure(
        "IO",
        &error.to_string(),
        "Check that the project is writable and has free space, then retry.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestDirectory;
    use splice_core::history::{Scope, Source};
    use splice_model::{MediaKind, Probe, Project, TakeComponents};

    #[test]
    fn m16_recovery_skips_takes_still_being_written_and_stop_can_retry() {
        for state in ["pending", "recording", "paused", "ready"] {
            assert!(!recoverable(Some(&Ok(state.into()))), "{state}");
        }
        assert!(!recoverable(Some(&Err("CAPTURE_AGENT_UNAVAILABLE"))));
        assert!(recoverable(None));
        assert!(recoverable(Some(&Ok("cancelled".into()))));
        assert!(recoverable(Some(&Err("NOT_FOUND"))));
        assert!(recoverable(Some(&Err("CAPTURE_FAILED"))));
        let error = retry_stop(failure("IO", "Disk full.", "Free space."));
        assert_eq!(error.code, "IO");
        assert!(error.fix.starts_with("Free space.") && error.fix.contains("record.stop again"));
    }

    #[test]
    fn recording_publication_is_one_undoable_batch_or_a_media_only_edit() {
        let temp = TestDirectory::new("record-publish");
        let path = temp.0.join("project.splice");
        let mut store = Store::create(&path, &Project::default()).unwrap();
        let session = store.start_session("recorder fixture", 1).unwrap().id;
        let mut write = WriteOptions {
            source: Source::Agent,
            session: Some(session),
            if_revision: Some(0),
            ..Default::default()
        };
        let id = JobId::from_serial(1001);
        let staged = stage_path(store.path(), id);
        fs::create_dir_all(&staged).unwrap();
        fs::write(
            staged.join("screen.mov"),
            b"stand-in for sealed screen media",
        )
        .unwrap();
        fs::write(staged.join("events.json"), b"{\"events\":[]}").unwrap();
        let descriptor = Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Take,
            path: String::new(),
            bookmark: None,
            size: 33,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 90,
                width: 80,
                height: 60,
                audio: Vec::new(),
                codec: Some("hevc".into()),
                loudness: None,
            },
            take: Some(TakeComponents {
                screen: Some("screen.mov".into()),
                events: Some("events.json".into()),
                ..Default::default()
            }),
        };

        let receipt = publish(
            &mut store,
            id,
            true,
            descriptor.clone(),
            &write,
            Default::default(),
        )
        .unwrap();
        assert_eq!(receipt.revision, 1);
        let clip = receipt.clip.unwrap();
        let slot = store.project().clips.find(clip).unwrap();
        assert_eq!(store.project().clips.timing(slot).unwrap().length, 90);
        assert_eq!(
            store.project().clips.data(slot).unwrap().media,
            Some(receipt.media)
        );
        let file = path.join(&store.project().media[0].path);
        assert!(file.ends_with(format!("takes/{}/screen.mov", receipt.media)));
        assert_eq!(
            fs::read(&file).unwrap(),
            b"stand-in for sealed screen media"
        );
        assert!(!staged.exists());
        assert_eq!(store.records().len(), 1);
        write.if_revision = Some(1);
        store.undo(Scope::Last, &write).unwrap();
        assert!(store.project().clips.order().is_empty() && store.project().media.is_empty());
        assert!(file.is_file());

        let id = JobId::from_serial(1002);
        let staged = stage_path(store.path(), id);
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("screen.mov"), b"second stand-in").unwrap();
        fs::write(staged.join("events.json"), b"{\"events\":[]}").unwrap();
        assert_eq!(
            publish(
                &mut store,
                id,
                false,
                descriptor.clone(),
                &write,
                Default::default()
            )
            .err()
            .unwrap()
            .code,
            "STALE_REVISION"
        );
        assert!(staged.is_dir());
        write.if_revision = Some(2);
        let second = publish(
            &mut store,
            id,
            false,
            descriptor,
            &write,
            Default::default(),
        )
        .unwrap();
        assert_eq!(second.revision, 3);
        assert_ne!(second.media, receipt.media);
        assert_eq!(second.clip, None);
        let hash = store.verify().unwrap();
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.verify().unwrap(), hash);
        assert_eq!(reopened.project().rev, 3);
        assert_eq!(reopened.project().media[0].id, second.media);
        assert!(reopened.project().clips.order().is_empty());
    }

    #[test]
    fn recording_summary_counts_input_and_flags_fit_and_silence() {
        let temp = TestDirectory::new("record-summary");
        let mut store = Store::create(&temp.0.join("p.splice"), &Project::default()).unwrap();
        let session = store.start_session("recorder fixture", 1).unwrap().id;
        let write = WriteOptions {
            source: Source::Agent,
            session: Some(session),
            if_revision: Some(0),
            ..Default::default()
        };
        let id = JobId::from_serial(7);
        let staged = stage_path(store.path(), id);
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("screen.mov"), b"stand-in").unwrap();
        fs::write(staged.join("mic.m4a"), b"stand-in").unwrap();
        fs::write(
            staged.join("events.json"),
            br#"{"events":[{"k":"move","t":0.1,"x":0.5,"y":0.5},{"k":"down","t":0.2,"x":0.5,"y":0.5,"b":0},{"k":"up","t":0.3,"x":0.5,"y":0.5},{"k":"key","t":0.4,"mods":0},{"k":"key","t":0.5,"mods":1048576}]}"#,
        )
        .unwrap();
        let descriptor = Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Take,
            path: String::new(),
            bookmark: None,
            size: 8,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 90,
                width: 2880,
                height: 1800,
                audio: Vec::new(),
                codec: Some("hevc".into()),
                loudness: None,
            },
            take: Some(TakeComponents {
                screen: Some("screen.mov".into()),
                mic: Some("mic.m4a".into()),
                events: Some("events.json".into()),
                ..Default::default()
            }),
        };
        let published =
            publish(&mut store, id, true, descriptor, &write, Default::default()).unwrap();
        // A quiet microphone: 0.0001 linear is -80 dBFS.
        let (take, lints, _) = summary(&store, &published, [178, 2], Some(0.0001));
        let mut text = String::new();
        splice_json::write(&take, &mut text).unwrap();
        assert_eq!(
            text,
            r#"{"frames":90,"fps":30,"seconds":3,"size":[2880,1800],"captureFps":60,"videoFrames":178,"droppedFrames":2,"tracks":["screen","mic","events"],"clicks":1,"keys":2,"micPeakDb":-80,"silent":true}"#
        );
        let codes: Vec<&str> = lints
            .iter()
            .filter_map(|l| l.field("code")?.as_str())
            .collect();
        assert!(
            codes.contains(&"MIC_SILENT") && codes.contains(&"TAKE_ASPECT"),
            "{codes:?}"
        );
        let fit = lints
            .iter()
            .find(|l| l.field("code").and_then(Json::as_str) == Some("TAKE_ASPECT"))
            .unwrap();
        let op = &fit.field("ops").unwrap().as_array().unwrap()[0];
        assert_eq!(op.field("aspect").and_then(Json::as_str), Some("16:10"));
        // The canvas a recording picks (record.start create:true) silences the fit warning.
        let mut options = write.clone();
        options.if_revision = Some(store.project().rev);
        store
            .apply(
                &[splice_json::parse(r#"{"cmd":"project.set","aspect":"16:10"}"#).unwrap()],
                &options,
            )
            .unwrap();
        assert_eq!(store.project().size, [1728, 1080]);
        let (_, lints, _) = summary(&store, &published, [178, 2], Some(0.5));
        assert!(
            lints
                .iter()
                .all(|l| l.field("code").and_then(Json::as_str) != Some("MIC_SILENT")),
        );
        // 16:10 on a 16:10 canvas fills it (T46.4 added the shape).
        assert!(
            lints
                .iter()
                .all(|l| l.field("code").and_then(Json::as_str) != Some("TAKE_ASPECT")),
        );
        // A 2880×1864 MacBook screen is nearest 16:10 but 3.5% off it: the fix crops to the canvas.
        let mut macbook = store.project().media[0].clone();
        macbook.probe.width = 2880;
        macbook.probe.height = 1864;
        let fit = crate::lints::take_fit(store.project(), &macbook, Some(ClipId::from_serial(1)))
            .expect("does not fill");
        let op = &fit.field("ops").unwrap().as_array().unwrap()[0];
        assert_eq!(op.field("cmd").and_then(Json::as_str), Some("clip.crop"));
    }
}
