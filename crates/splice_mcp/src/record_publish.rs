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
}
