use super::*;
use splice_model::{CaptionEdit, ItemKind};
use splice_testkit::Xorshift;
use std::fs;
use std::path::PathBuf;

struct Temp(PathBuf);

impl Temp {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "splice-history-{}-{label}.splice",
            std::process::id()
        ));
        assert!(!path.exists());
        Self(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.0.exists() {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn op(text: &str) -> Json {
    splice_json::parse(text).unwrap()
}

fn human(time: u64) -> WriteOptions {
    WriteOptions {
        time,
        ..WriteOptions::default()
    }
}

fn agent(session: SessionId, rev: u64, time: u64) -> WriteOptions {
    WriteOptions {
        source: Source::Agent,
        session: Some(session),
        if_revision: Some(rev),
        ..human(time)
    }
}

#[test]
fn history_seeded_five_hundred_batches_restore_and_replay() {
    let temp = Temp::new("seeded");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("seeded", 0).unwrap().id;
    let mut rng = Xorshift::new(20261118);
    let mut hashes = vec![state_hash(store.project()).unwrap()];
    for rev in 1..=500 {
        let aspect = ["9:16", "4:5", "1:1", "4:3", "16:9"][(rng.next_u64() % 5) as usize];
        let command = if rev % 13 == 0 {
            op(&format!(
                r#"{{"cmd":"marker.add","at":{rev},"name":"Beat {rev}"}}"#
            ))
        } else {
            op(&format!(r#"{{"cmd":"project.set","aspect":"{aspect}"}}"#))
        };
        let options = if rng.next_u64() & 1 == 0 {
            human(rev)
        } else {
            agent(session, rev - 1, rev)
        };
        let committed = store.apply(&[command], &options).unwrap();
        assert_eq!(committed.revision, rev);
        hashes.push(state_hash(store.project()).unwrap());
    }
    let stamp = (store.project().rev, store.project().hash);
    store.close().unwrap();
    let mut store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none());
    assert_eq!((store.project().rev, store.project().hash), stamp);
    assert_eq!(state_hash(store.project()).unwrap(), hashes[500]);

    for (rev, hash) in hashes.into_iter().enumerate() {
        let before = store.project().rev;
        let restored = store
            .undo(Scope::To(rev as u64), &human(1000 + rev as u64))
            .unwrap();
        assert_eq!(restored.revision, before + 1);
        assert_eq!(state_hash(store.project()).unwrap(), hash, "revision {rev}");
    }
    let latest = (
        store.project().rev,
        store.project().hash,
        state_hash(store.project()).unwrap(),
    );
    drop(store); // Exercises replay after the last periodic snapshot.
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(
        (
            store.project().rev,
            store.project().hash,
            state_hash(store.project()).unwrap()
        ),
        latest
    );
}

#[test]
fn low_idle_undo_reports_nothing_to_undo() {
    let temp = Temp::new("idle-undo");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("agent", 1).unwrap().id;
    for scope in [Scope::Last, Scope::Session] {
        let error = store.undo(scope, &agent(session, 0, 2)).unwrap_err();
        assert_eq!(error.code, "NOTHING_TO_UNDO");
    }
    assert_eq!(store.project().rev, 0);
    store
        .apply(
            &[op(r#"{"cmd":"marker.add","at":5}"#)],
            &agent(session, 0, 3),
        )
        .unwrap();
    store.undo(Scope::Last, &agent(session, 1, 4)).unwrap();
    // The undone batch is no longer live: a second undo has nothing left.
    let error = store.undo(Scope::Last, &agent(session, 2, 5)).unwrap_err();
    assert_eq!(error.code, "NOTHING_TO_UNDO");
    assert_eq!(store.project().rev, 2);
}

#[test]
fn history_session_revert_replays_later_human_edits_and_undoes_the_undo() {
    let temp = Temp::new("revert");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("agent", 1).unwrap();
    assert_eq!(session.checkpoint, 0);
    store
        .apply(
            &[op(r#"{"cmd":"marker.add","at":5}"#)],
            &agent(session.id, 0, 2),
        )
        .unwrap();
    store
        .apply(
            &[op(
                r#"{"cmd":"marker.set","marker":"mrk_1","name":"Human edit"}"#,
            )],
            &human(3),
        )
        .unwrap();
    store
        .apply(&[op(r#"{"cmd":"project.set","aspect":"4:3"}"#)], &human(4))
        .unwrap();
    let original = state_hash(store.project()).unwrap();

    let undo = store
        .undo(Scope::Session, &agent(session.id, 3, 5))
        .unwrap();
    assert_eq!(undo.reverted, [1]);
    assert_eq!(undo.skipped.len(), 1);
    assert_eq!(
        (undo.skipped[0].revision, undo.skipped[0].error.as_str()),
        (2, "NOT_FOUND")
    );
    assert_eq!(store.project().aspect, splice_model::Aspect::Standard);
    assert!(store.project().markers.is_empty());
    let altered = state_hash(store.project()).unwrap();
    assert_ne!(altered, original);

    let revision = store.project().rev;
    store.human_undo(6).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), original);
    store.human_redo(7).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), altered);
    store.undo(Scope::To(3), &human(8)).unwrap();
    let again = store
        .undo(Scope::Last, &agent(session.id, revision + 3, 9))
        .unwrap();
    assert_eq!(again.skipped, undo.skipped);
    assert_eq!(state_hash(store.project()).unwrap(), altered);
    store.close().unwrap();
    let store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none());
    assert_eq!(state_hash(store.project()).unwrap(), altered);
}

#[test]
fn history_batch_undo_keeps_other_batches_and_replays_after_reopen() {
    let temp = Temp::new("batch-undo");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("agent", 1).unwrap().id;
    let other = store.start_session("other", 1).unwrap().id;
    store
        .apply(
            &[op(r#"{"cmd":"marker.add","at":5}"#)],
            &agent(session, 0, 2),
        )
        .unwrap();
    store
        .apply(
            &[op(
                r#"{"cmd":"marker.set","marker":"mrk_1","name":"Human edit"}"#,
            )],
            &WriteOptions {
                session: Some(session),
                ..human(3)
            },
        )
        .unwrap();
    store
        .apply(
            &[op(
                r#"{"cmd":"marker.add","at":12,"name":"Keep same session"}"#,
            )],
            &agent(session, 2, 4),
        )
        .unwrap();
    store
        .apply(
            &[op(
                r#"{"cmd":"marker.add","at":20,"name":"Keep other session"}"#,
            )],
            &agent(other, 3, 5),
        )
        .unwrap();
    store
        .apply(&[op(r#"{"cmd":"project.set","aspect":"4:3"}"#)], &human(6))
        .unwrap();

    let original = state_hash(store.project()).unwrap();
    let kept_markers = store.project().markers[1..].to_vec();
    for revision in [0, 2, 4, 6] {
        let error = store
            .undo(Scope::Batch(revision), &agent(session, 5, 7))
            .unwrap_err();
        assert_eq!(error.code, "NOT_FOUND");
    }
    store.end_session(session).unwrap();
    let closed = store
        .undo(Scope::Batch(1), &agent(session, 5, 7))
        .unwrap_err();
    assert_eq!(closed.code, "NO_SESSION");
    assert_eq!(store.project().rev, 5);

    let mut options = WriteOptions {
        session: Some(session),
        if_revision: Some(5),
        ..human(7)
    };
    let undo = store.undo(Scope::Batch(1), &options).unwrap();
    assert_eq!(undo.revision, 6);
    assert_eq!(undo.reverted, [1]);
    assert_eq!(undo.replayed, 3);
    assert_eq!(undo.skipped.len(), 1);
    assert_eq!(
        (undo.skipped[0].revision, undo.skipped[0].error.as_str()),
        (2, "NOT_FOUND")
    );
    let record = &store.records()[5];
    assert_eq!(
        (record.batch.source, record.batch.session),
        (Source::Human, Some(session))
    );
    assert_eq!(store.project().markers, kept_markers);
    assert_eq!(store.project().aspect, splice_model::Aspect::Standard);
    assert!(!store.is_active(1));
    assert!(!store.is_active(2));
    assert!([3, 4, 5].into_iter().all(|rev| store.is_active(rev)));

    options.if_revision = Some(6);
    assert_eq!(
        store.undo(Scope::Batch(1), &options).unwrap_err().code,
        "NOTHING_TO_UNDO"
    );
    assert_eq!(
        store.undo(Scope::Batch(6), &options).unwrap_err().code,
        "NOT_FOUND"
    );
    assert_eq!(store.project().rev, 6);
    let reverted = state_hash(store.project()).unwrap();
    drop(store);

    let mut store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none());
    assert_eq!(store.project().rev, 6);
    assert_eq!(state_hash(store.project()).unwrap(), reverted);
    assert!(!store.sessions()[0].open);
    assert_eq!(store.human_undo(8).unwrap().revision, 7);
    assert!(store.is_active(1));
    assert_eq!(state_hash(store.project()).unwrap(), original);
    assert_eq!(store.human_redo(9).unwrap().revision, 8);
    assert!(!store.is_active(1));
    assert_eq!(state_hash(store.project()).unwrap(), reverted);
}

#[test]
fn history_guards_dry_run_and_idempotency_survive_reopen() {
    let temp = Temp::new("idempotency");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("agent", 1).unwrap();
    let mut request = agent(session.id, 0, 2);
    request.key = Some("first".into());
    request.dry_run = true;
    let ops = [op(r#"{"cmd":"marker.add","at":8}"#)];
    let preview = store.apply(&ops, &request).unwrap();
    assert!(preview.dry_run);
    assert_eq!(store.project().rev, 0);
    request.dry_run = false;
    let committed = store.apply(&ops, &request).unwrap();
    assert_eq!(preview.report, committed.report);
    drop(store);

    let mut store = Store::open(&temp.0).unwrap();
    let replayed = store.apply(&ops, &request).unwrap();
    assert!(replayed.replayed);
    assert_eq!(
        (replayed.revision, replayed.hash, replayed.report),
        (committed.revision, committed.hash, committed.report)
    );
    let error = store
        .apply(&[op(r#"{"cmd":"marker.add","at":9}"#)], &request)
        .unwrap_err();
    assert_eq!(error.code, "CONFLICT");
    request.key = None;
    let stale = store.apply(&ops, &request).unwrap_err();
    assert_eq!(stale.code, "STALE_REVISION");
    let stale = stale.stale.unwrap();
    assert_eq!(stale.current, 1);
    assert_eq!(stale.changed_since[0].revision, 1);
    store.end_session(session.id).unwrap();
    drop(store);
    let mut store = Store::open(&temp.0).unwrap();
    assert_eq!(store.apply(&ops, &request).unwrap_err().code, "NO_SESSION");
}

#[test]
fn history_reopened_branch_preserves_mint_high_water_mark() {
    let temp = Temp::new("branch");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let add = [op(r#"{"cmd":"marker.add","at":12}"#)];
    assert_eq!(
        store.apply(&add, &human(1)).unwrap().report.mints,
        ["mrk_1"]
    );
    store.undo(Scope::To(0), &human(2)).unwrap();
    assert_eq!(store.project().serial, 1);
    store.close().unwrap();

    let mut store = Store::open(&temp.0).unwrap();
    assert_eq!(
        store.apply(&add, &human(3)).unwrap().report.mints,
        ["mrk_2"]
    );
    let expected = state_hash(store.project()).unwrap();
    drop(store);
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), expected);
    assert!(store.at(1).unwrap().markers[0].id != store.project().markers[0].id);
}

#[test]
fn history_session_undo_keeps_other_sessions_and_rejects_access_changes() {
    let temp = Temp::new("sessions");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let left = store.start_session("left", 1).unwrap().id;
    let right = store.start_session("right", 1).unwrap().id;
    assert_ne!(left, right);
    store
        .apply(&[op(r#"{"cmd":"marker.add","at":11}"#)], &agent(left, 0, 2))
        .unwrap();
    store
        .apply(
            &[op(r#"{"cmd":"marker.add","at":22}"#)],
            &agent(right, 1, 3),
        )
        .unwrap();
    store
        .apply(&[op(r#"{"cmd":"marker.add","at":33}"#)], &agent(left, 2, 4))
        .unwrap();
    let last = store.undo(Scope::Last, &agent(left, 3, 5)).unwrap();
    assert_eq!(last.reverted, [3]);
    assert!(store.is_active(1));
    assert!(store.is_active(2));
    let rest = store.undo(Scope::Session, &agent(left, 4, 6)).unwrap();
    assert_eq!(rest.reverted, [1]);
    assert_eq!(store.project().markers[0].at, 22);
    assert!(store.is_active(2));

    store
        .apply(&[op(r#"{"cmd":"project.set","access":"read"}"#)], &human(7))
        .unwrap();
    store
        .apply(&[op(r#"{"cmd":"project.set","access":"live"}"#)], &human(8))
        .unwrap();
    let denied = store.undo(Scope::To(6), &agent(left, 7, 9)).unwrap_err();
    assert_eq!(denied.code, "PERMISSION_DENIED");
    assert_eq!(store.project().rev, 7);
}

#[test]
fn history_verify_accepts_transcripts_stored_in_media_order() {
    use splice_model::{Media, MediaId, MediaKind, Probe, Transcript};

    let dir = Temp::new("verify");
    let ids = [MediaId::from_serial(1), MediaId::from_serial(2)];
    let project = Project {
        media: ids
            .iter()
            .map(|&id| Media {
                id,
                kind: MediaKind::Audio,
                path: format!("{id}.m4a"),
                bookmark: None,
                size: 0,
                mtime: 0,
                probe: Probe {
                    fps: 30.0,
                    frames: 30,
                    width: 0,
                    height: 0,
                    audio: Vec::new(),
                    codec: None,
                    loudness: None,
                },
                take: None,
            })
            .collect(),
        transcripts: ids
            .iter()
            .rev()
            .map(|&media| Transcript {
                media,
                hash: media.serial(),
                language: "en".into(),
                words: Vec::new(),
            })
            .collect(),
        ..Project::default()
    };
    Store::create(&dir.0, &project).unwrap().close().unwrap();
    let reopened = Store::open(&dir.0).unwrap();
    assert_eq!(reopened.verify().unwrap(), state_hash(&project).unwrap());
}

#[test]
fn m14_relink_reprobes_and_refuses_a_mismatching_file() {
    use crate::commands::CommandError;
    use splice_model::{AudioTrack, Media, MediaId, MediaKind, Probe};

    fn file(path: &str, frames: i32, fps: f64, width: u32, audio: usize) -> Media {
        Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Video,
            path: path.into(),
            bookmark: None,
            size: 100,
            mtime: 1,
            probe: Probe {
                fps,
                frames,
                width,
                height: 64,
                audio: (0..audio)
                    .map(|n| AudioTrack {
                        name: format!("track {n}"),
                        channels: Some(2),
                    })
                    .collect(),
                codec: None,
                loudness: None,
            },
            take: None,
        }
    }
    let temp = Temp::new("relink");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("test", 1).unwrap().id;
    let mut loader = |path: &str| -> Result<Media, CommandError> {
        match path {
            "a.mov" => Ok(file("a.mov", 30, 30.0, 96, 0)),
            "long.mov" => Ok(file("long.mov", 60, 30.0, 96, 0)),
            "b.mov" => Ok(file("b.mov", 30, 25.0, 192, 1)),
            _ => Err(CommandError::new(
                "MEDIA_OFFLINE",
                format!("Could not access {path}."),
                "Check the path.".into(),
            )),
        }
    };
    let mut rev = 0;
    let mut relink = |store: &mut Store, args: &str, rev: &mut u64| {
        let result = store.apply_with_media(
            &[op(&format!(
                r#"{{"cmd":"media.relink","media":"med_1",{args}}}"#
            ))],
            &agent(session, *rev, 2),
            &mut loader,
        );
        if result.is_ok() {
            *rev += 1;
        }
        result
    };
    store
        .apply_with_media(
            &[op(r#"{"cmd":"clip.insert","path":"a.mov"}"#)],
            &agent(session, 0, 2),
            &mut |path| Ok(file(path, 30, 30.0, 96, 0)),
        )
        .unwrap();
    rev += 1;
    // A different length is a different file.
    let error = relink(&mut store, r#""path":"long.mov""#, &mut rev).unwrap_err();
    assert_eq!(error.code, "MEDIA_MISMATCH");
    assert_eq!(error.rule.as_ref().unwrap().param, "path");
    // An agent's frames no longer skips the probe: a missing file is refused.
    let error = relink(&mut store, r#""path":"gone.mov","frames":30"#, &mut rev).unwrap_err();
    assert_eq!(error.code, "MEDIA_OFFLINE");
    let error = relink(&mut store, r#""path":"b.mov","frames":31"#, &mut rev).unwrap_err();
    assert_eq!(error.rule.as_ref().unwrap().param, "frames");
    // The same media re-encoded: fps, size and tracks follow the new file.
    relink(&mut store, r#""path":"b.mov""#, &mut rev).unwrap();
    let relinked = store.project().media[0].clone();
    assert_eq!(relinked.path, "b.mov");
    assert_eq!(
        (
            relinked.probe.fps,
            relinked.probe.width,
            relinked.probe.audio.len()
        ),
        (25.0, 192, 1)
    );
    relink(&mut store, r#""path":"long.mov","force":true"#, &mut rev).unwrap();
    assert_eq!(store.project().media[0].probe.frames, 60);
    let project = store.project().clone();
    store.close().unwrap();
    // Replay rebuilds both relinks from the journal without a prober.
    assert_eq!(Store::open(&temp.0).unwrap().project().media, project.media);
}

#[test]
fn history_media_import_is_atomic_and_replays_without_a_loader() {
    use splice_model::{Media, MediaId, MediaKind, Probe};

    let temp = Temp::new("import");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let session = store.start_session("test", 1).unwrap().id;
    let insertion = op(r#"{"cmd":"clip.insert","path":"./probe.mov"}"#);
    let ops = std::slice::from_ref(&insertion);
    let mut calls = 0;
    let mut loader = |path: &str| {
        assert_eq!(path, "./probe.mov");
        calls += 1;
        Ok(Media {
            id: MediaId::from_serial(0),
            kind: MediaKind::Video,
            path: "probe.mov".into(),
            bookmark: Some("Zml4dHVyZQ==".into()),
            size: 3014,
            mtime: 1,
            probe: Probe {
                fps: 30.0,
                frames: 30,
                width: 96,
                height: 64,
                audio: Vec::new(),
                codec: Some("h264".into()),
                loudness: None,
            },
            take: None,
        })
    };
    let options = WriteOptions {
        key: Some("once".into()),
        ..agent(session, 0, 2)
    };
    let bad = [
        insertion.clone(),
        op(r#"{"cmd":"item.delete","items":"clp_missing"}"#),
    ];
    assert_eq!(
        store
            .apply_with_media(&bad, &options, &mut loader)
            .unwrap_err()
            .op,
        Some(1)
    );
    assert!(store.project().media.is_empty());
    let preview = store
        .apply_with_media(
            ops,
            &WriteOptions {
                dry_run: true,
                ..options.clone()
            },
            &mut loader,
        )
        .unwrap();
    assert!(store.project().media.is_empty());
    assert!(store.records().is_empty());
    let result = store.apply_with_media(ops, &options, &mut loader).unwrap();
    assert_eq!(result.report.mints, ["med_1", "clp_2"]);
    assert_eq!(result.report, preview.report);
    assert_eq!(store.project().media[0].path, "probe.mov");
    assert_eq!(calls, 3);
    let hash = state_hash(store.project()).unwrap();

    let interrupted_checkpoint = temp.0.join("project.json.tmp");
    fs::create_dir(&interrupted_checkpoint).unwrap();
    assert_eq!(store.checkpoint().unwrap_err().code, "IO");
    fs::remove_dir(&interrupted_checkpoint).unwrap();
    drop(store);
    let mut reopened = Store::open(&temp.0).unwrap();
    assert_eq!(reopened.verify().unwrap(), hash);
    assert!(reopened.apply(ops, &options).unwrap().replayed);
    assert_eq!(reopened.records().len(), 1);
    reopened.checkpoint().unwrap();
    reopened.undo(Scope::Last, &agent(session, 1, 3)).unwrap();
    assert!(reopened.project().media.is_empty());
    assert!(reopened.project().clips.order().is_empty());
    fs::create_dir(&interrupted_checkpoint).unwrap();
    assert_eq!(reopened.close().unwrap_err().code, "IO");
    fs::remove_dir(&interrupted_checkpoint).unwrap();

    let reopened = Store::open(&temp.0).unwrap();
    assert_eq!(reopened.project().rev, 2);
    reopened.verify().unwrap();
    reopened.close().unwrap();
    assert!(Store::open(&temp.0).unwrap().project().media.is_empty());
}

#[test]
fn history_memory_stays_bounded_and_old_revisions_rebuild_exactly() {
    let temp = Temp::new("bounded");
    let mut store = Store::create(&temp.0, &Project::default()).unwrap();
    let mut rng = Xorshift::new(20261119);
    let mut hashes = vec![store.project().hash];
    let total = 1_500u64;
    for rev in 1..=total {
        let command = if rev % 7 == 0 {
            op(&format!(
                r#"{{"cmd":"marker.add","at":{rev},"name":"Beat {rev}"}}"#
            ))
        } else {
            let aspect = ["9:16", "4:5", "1:1", "4:3", "16:9"][(rng.next_u64() % 5) as usize];
            op(&format!(r#"{{"cmd":"project.set","aspect":"{aspect}"}}"#))
        };
        store.apply(&[command], &human(rev)).unwrap();
        hashes.push(store.project().hash);
    }
    // Kept projects: the recent window plus one per stride (not one per revision).
    let kept = store.versions.materialized();
    assert!(kept <= 32 + total as usize / 64 + 2, "{kept} projects kept");
    // Evicted revisions rebuild to their recorded hashes and contents.
    for _ in 0..40 {
        let rev = rng.next_u64() % total;
        let project = store.at(rev).unwrap();
        assert_eq!(project.hash, hashes[rev as usize]);
        assert_eq!(project.markers.len() as u64, rev / 7);
    }
    // Undo to an old revision still works.
    let target = 100;
    store
        .undo(
            Scope::To(target),
            &WriteOptions {
                time: total + 1,
                ..WriteOptions::default()
            },
        )
        .unwrap();
    assert_eq!(store.project().markers.len() as u64, target / 7);
}

/// T24.3: a 1-hour, 5,000-item project opens in under 250 ms and 300 MB of heap.
#[test]
fn budget_open_an_hour_long_five_thousand_item_project() {
    use splice_model::*;
    let temp = Temp::new("open-budget");
    let mut project = Project::default();
    let lane = LaneId::from_serial(9);
    project.lanes.push(Lane {
        id: lane,
        name: "Titles".into(),
        kind: LaneKind::Graphics,
        effects: Vec::new(),
    });
    let hour = 30 * 60 * 60;
    for index in 0..5_000u64 {
        let at = (index as i32 * hour) / 5_000;
        let mut appearance = Appearance::default();
        appearance.effects.push(Effect {
            id: EffectId::from_serial(100_000 + index),
            kind: "transform".into(),
            enabled: None,
            params: vec![EffectParam {
                name: "scale".into(),
                value: EffectValue::Keys(vec![
                    keys::Key {
                        at: 0,
                        value: 1.0,
                        ease: keys::Ease::InOut,
                    },
                    keys::Key {
                        at: 20,
                        value: 1.2,
                        ease: keys::Ease::Linear,
                    },
                ]),
            }],
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(10_000 + index),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(at),
                    length: 60,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Title {
                        text: format!("Chapter title number {index}"),
                    },
                    appearance,
                },
            })
            .unwrap();
    }
    project.serial = 200_000;
    Store::create(&temp.0, &project).unwrap().close().unwrap();
    let mut best = std::time::Duration::MAX;
    let mut peak = 0;
    for _ in 0..3 {
        mem_sys::local_reset();
        let started = std::time::Instant::now();
        let store = Store::open(&temp.0).unwrap();
        best = best.min(started.elapsed());
        peak = peak.max(mem_sys::local_peak_bytes());
        assert_eq!(store.project().items.order().len(), 5_000);
        store.close().unwrap();
    }
    eprintln!("open: best {best:?}, peak heap {} MB", peak / 1_000_000);
    assert!(
        best < std::time::Duration::from_millis(250),
        "open took {best:?}"
    );
    assert!(peak < 300_000_000, "open peaked at {peak} bytes");
}

/// T24.4 chaos: 1,000 rounds of two writers racing to open the same package. Each open is either
/// the one writer or `LOCKED` (never both at once). Every tenth round the winners also write a
/// batch (fsynced writes cost ~4 ms each, so not every round), and every one of those lands.
#[test]
fn writer_races_never_share_a_package_or_lose_a_batch() {
    let temp = Temp::new("writer-races");
    Store::create(&temp.0, &Project::default())
        .unwrap()
        .close()
        .unwrap();
    let path = std::sync::Arc::new(temp.0.clone());
    let holders = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let mut wins = 0u64;
    for round in 0..1_000u32 {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let racers: Vec<_> = (0..2u32)
            .map(|racer| {
                let (path, holders, barrier) = (path.clone(), holders.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    let mut store = match Store::open(&path) {
                        Ok(store) => store,
                        Err(error) => {
                            assert_eq!(error.code, "LOCKED");
                            return (0u64, 0u64);
                        }
                    };
                    let inside = holders.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    assert_eq!(inside, 0, "two writers held the package");
                    let writes = round % 10 == 0;
                    if writes {
                        let at = round * 2 + racer;
                        let op =
                            splice_json::parse(&format!(r#"{{"cmd":"marker.add","at":{at}}}"#))
                                .unwrap();
                        store
                            .apply(std::slice::from_ref(&op), &WriteOptions::default())
                            .unwrap();
                    }
                    holders.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    store.close().unwrap();
                    (1, u64::from(writes))
                })
            })
            .collect();
        let (opened, wrote) = racers
            .into_iter()
            .map(|racer| racer.join().unwrap())
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        assert!(opened >= 1, "round {round}: nobody got the package");
        wins += wrote;
    }
    let store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none());
    assert_eq!(store.project().markers.len() as u64, wins);
    assert_eq!(store.project().rev, wins);
}

/// A real 0.3 project (161 batches: motion items, the click-zoom curve, 0.3 autoZoom, keys on
/// parameters 0.4 does not animate, a marker past the end) opens under 0.4: its records
/// replay with their 0.3 meaning, the checkpoint 0.3 wrote is superseded by the replay, and
/// a new edit on top is recorded by the current engine.
#[test]
fn a_0_3_journal_opens_replays_and_takes_new_edits() {
    let temp = Temp::new("legacy");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/projects/legacy-0.3.splice"
    );
    fs::create_dir_all(&temp.0).unwrap();
    for entry in fs::read_dir(fixture).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), temp.0.join(entry.file_name())).unwrap();
    }
    let mut store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none(), "{:?}", store.problem());
    assert_eq!(store.project().rev, 161);
    store.verify().unwrap();
    // The 0.3 motion items are camera zooms now; the titles survived.
    let kinds: Vec<_> = store
        .project()
        .items
        .order()
        .iter()
        .map(|&s| store.project().items.data(s).unwrap().kind.clone())
        .collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| matches!(k, splice_model::ItemKind::Title { .. }))
            .count(),
        7
    );
    let committed = store
        .apply(
            &[op(r#"{"cmd":"marker.add","at":10,"name":"after 0.4"}"#)],
            &human(1),
        )
        .unwrap();
    assert_eq!(committed.revision, 162);
    let last = store.records().last().unwrap();
    assert_eq!(last.engine, crate::package::journal::ENGINE);
    store.close().unwrap();
    let store = Store::open(&temp.0).unwrap();
    assert!(store.problem().is_none());
    store.verify().unwrap();
}

fn sample_store(path: &Path) -> Store {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    fs::create_dir_all(path.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        fs::copy(source.join(file), path.join(file)).unwrap();
    }
    Store::open(path).unwrap()
}

fn caption_edits(project: &Project) -> &[CaptionEdit] {
    let slot = project.items.find("ovl_cap1".parse().unwrap()).unwrap();
    let ItemKind::Caption { edits, .. } = &project.items.data(slot).unwrap().kind else {
        panic!("expected ovl_cap1 to be a caption");
    };
    edits
}

fn set_caption(store: &mut Store, args: &str) {
    let edit = op(&format!(
        r#"{{"cmd":"caption.set","item":"ovl_cap1",{args}}}"#
    ));
    store
        .apply(&[edit], &human(store.project().rev + 1))
        .unwrap();
}

#[test]
fn caption_hide_preserves_replacement_and_retype_restores_words() {
    let temp = Temp::new("caption-hide");
    let mut store = sample_store(&temp.0);
    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"text":"weaver","emph":true,"emoji":"!"}}"#,
    );
    // The same edit written by engine 4 has the same canonical project bytes.
    let replacement = 0x3a94_7f41_8640_4c8e;
    assert_eq!(state_hash(store.project()).unwrap(), replacement);

    set_caption(&mut store, r#""words":{"w:screen:2":{"hidden":true}}"#);
    assert_eq!(
        store.records().last().unwrap().engine,
        package::journal::CAPTION_HIDE_ENGINE
    );
    store.checkpoint().unwrap();
    let snapshot = fs::read_to_string(temp.0.join("project.json")).unwrap();
    assert!(snapshot.contains(
        r#"{"word":"w:screen:2","text":"weaver","emph":true,"emoji":"!","hidden":true}"#
    ));
    drop(store);
    let mut store = Store::open(&temp.0).unwrap();
    assert!(store.verify().is_ok());
    let edit = &caption_edits(store.project())[0];
    assert!(edit.hidden && edit.is_hidden());
    assert_eq!(edit.text.as_deref(), Some("weaver"));

    set_caption(&mut store, r#""words":{"w:screen:2":{"hidden":false}}"#);
    assert_eq!(state_hash(store.project()).unwrap(), replacement);

    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"text":"another replacement","hidden":true}}"#,
    );
    assert_eq!(
        caption_edits(store.project())[0].text.as_deref(),
        Some("another replacement")
    );
    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"text":"still hidden"}}"#,
    );
    assert!(caption_edits(store.project())[0].is_hidden());
    assert_eq!(
        caption_edits(store.project())[0].text.as_deref(),
        Some("still hidden")
    );

    set_caption(&mut store, r#""text":"Today""#);
    let edits = caption_edits(store.project());
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit.hidden && edit.text.is_none()));
    assert!(edits[0].emph);
    assert_eq!(edits[0].emoji.as_deref(), Some("!"));

    set_caption(&mut store, r#""text":"Today we build in the lab""#);
    let edits = caption_edits(store.project());
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| !edit.is_hidden()));
    assert!(edits[0].text.is_none() && edits[0].emph);
    assert_eq!(edits[0].emoji.as_deref(), Some("!"));
    assert_eq!(edits[1].text.as_deref(), Some("build in the lab"));

    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"text":"cut replacement","hidden":true}}"#,
    );
    store
        .apply(
            &[op(r#"{"cmd":"cut.words","words":["w:screen:2"]}"#)],
            &human(store.project().rev + 1),
        )
        .unwrap();
    set_caption(&mut store, r#""text":"Today build after the cut""#);
    let edits = caption_edits(store.project());
    assert!(edits[0].is_hidden());
    assert_eq!(edits[0].text.as_deref(), Some("cut replacement"));
    assert_eq!(edits[1].text.as_deref(), Some("build after the cut"));

    set_caption(&mut store, r#""words":{"w:screen:2":{"text":"we"}}"#);
    let cut = &caption_edits(store.project())[0];
    assert!(cut.hidden && cut.text.is_none());

    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"hidden":true},"w:screen:3":{"hidden":true}}"#,
    );
    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"hidden":null,"text":null,"emph":false,"emoji":""},"w:screen:3":null}"#,
    );
    assert!(caption_edits(store.project()).is_empty());
    store.close().unwrap();
    let store = Store::open(&temp.0).unwrap();
    store.verify().unwrap();
    assert!(caption_edits(store.project()).is_empty());
}

#[test]
fn caption_hide_replays_engine_four_and_unhides_legacy_empty_text() {
    let temp = Temp::new("caption-hide-v4");
    drop(sample_store(&temp.0));
    // Captured from a pre-change Store after a replacement and a separate hide.
    let records = concat!(
        r#"{"rev":1,"parent":"6d4ac5c362bbe1d2","hash":"59ab1fb22698eef0","engine":4,"time":1,"source":"human","label":"Caption ovl_cap1","ops":[{"cmd":"caption.set","item":"ovl_cap1","words":{"w:screen:2":{"text":"weaver","emph":true,"emoji":"!"}}}],"mints":[]}"#,
        "\n",
        r#"{"rev":2,"parent":"59ab1fb22698eef0","hash":"82bf67582aa960aa","engine":4,"time":2,"source":"human","label":"Caption ovl_cap1","ops":[{"cmd":"caption.set","item":"ovl_cap1","words":{"w:screen:2":{"hidden":true}}}],"mints":[]}"#,
        "\n",
    );
    fs::write(temp.0.join("journal.ops"), records).unwrap();
    let path = temp.0.join("project.json");
    let word_edits = r#""words":["w:screen:1","w:screen:2","w:screen:3"],"edits":"#;
    let checkpoint = fs::read_to_string(&path)
        .unwrap()
        .replacen(
            r#""rev":0,"hash":"6d4ac5c362bbe1d2""#,
            r#""rev":2,"hash":"82bf67582aa960aa""#,
            1,
        )
        .replacen(
            &format!("{word_edits}[]"),
            &format!(r#"{word_edits}[{{"word":"w:screen:2","text":"","emph":true,"emoji":"!"}}]"#),
            1,
        );
    fs::write(&path, &checkpoint).unwrap();
    let (package, saved) = package::Package::open(&temp.0).unwrap();
    assert_eq!(
        package::json_bytes(&package::snapshot(&saved)).unwrap(),
        checkpoint.as_bytes()
    );
    drop(package);

    let mut store = Store::open(&temp.0).unwrap();
    assert_eq!(store.project().rev, 2);
    assert_eq!(store.verify().unwrap(), 0x55cc_7daa_a158_0322);
    let edit = &caption_edits(store.project())[0];
    assert!(!edit.hidden && edit.is_hidden());
    assert_eq!(edit.text.as_deref(), Some(""));

    set_caption(&mut store, r#""words":{"w:screen:2":{"hidden":false}}"#);
    let edit = &caption_edits(store.project())[0];
    assert!(!edit.is_hidden() && edit.text.is_none());
    assert!(edit.emph);
    assert_eq!(edit.emoji.as_deref(), Some("!"));
    store.close().unwrap();

    let mut store = Store::open(&temp.0).unwrap();
    store.verify().unwrap();
    assert!(!caption_edits(store.project())[0].is_hidden());
    assert!(caption_edits(&store.at(2).unwrap())[0].is_hidden());
    store.undo(Scope::To(2), &human(4)).unwrap();
    set_caption(
        &mut store,
        r#""words":{"w:screen:2":{"text":"new replacement"}}"#,
    );
    let edit = &caption_edits(store.project())[0];
    assert!(edit.hidden && edit.is_hidden());
    assert_eq!(edit.text.as_deref(), Some("new replacement"));
}

fn duplicate_effects(project: &Project, id: splice_model::TimelineId) -> &[splice_model::Effect] {
    use splice_model::TimelineId;
    match id {
        TimelineId::Clip(id) => {
            &project
                .clips
                .data(project.clips.find(id).unwrap())
                .unwrap()
                .appearance
                .effects
        }
        TimelineId::Item(id) => {
            &project
                .items
                .data(project.items.find(id).unwrap())
                .unwrap()
                .appearance
                .effects
        }
        TimelineId::Marker(_) => panic!("markers have no effects"),
    }
}

#[test]
fn duplicate_effect_ids_are_independent_before_settle_and_through_history() {
    use crate::commands::Target;
    use splice_model::{Anchor, EffectValue, TimelineId};

    let temp = Temp::new("duplicate-effects");
    let mut store = sample_store(&temp.0);
    let source = [
        TimelineId::Clip("clp_open".parse().unwrap()),
        TimelineId::Item("ovl_hook".parse().unwrap()),
        TimelineId::Item("ovl_label".parse().unwrap()),
        TimelineId::Item("ovl_broll".parse().unwrap()),
    ];
    store.apply(op(r#"[
        {"cmd":"param.keys","target":"clp_open","path":"transform.scale","keys":[{"at":0,"value":1},{"at":30,"value":1.2}]},
        {"cmd":"param.keys","target":"ovl_hook","path":"transform.scale","keys":[{"at":0,"value":1},{"at":30,"value":1.2}]},
        {"cmd":"item.anchor","item":"ovl_label","anchor":{"item":"ovl_hook","edge":"end","offset":18}},
        {"cmd":"param.keys","target":"ovl_label","path":"transform.scale","keys":[{"at":0,"value":1},{"at":30,"value":1.2}]},
        {"cmd":"param.keys","target":"ovl_broll","path":"transform.scale","keys":[{"at":0,"value":1},{"at":30,"value":1.2}]}
    ]"#).as_array().unwrap(), &human(1)).unwrap();
    let before = source.map(|id| duplicate_effects(store.project(), id).to_vec());
    let before_paste = state_hash(store.project()).unwrap();

    // The pasted clip sorts before its source. A deferred rekey let this command edit the copy.
    let batch = op(r#"[
        {"cmd":"item.duplicate","items":["clp_open","ovl_broll"],"at":0},
        {"cmd":"effect.set","effect":"fx_openwin","params":{"padding":0.12}}
    ]"#);
    let pasted = store.apply(batch.as_array().unwrap(), &human(2)).unwrap();
    assert_eq!(
        store.records()[1].engine,
        package::journal::DUPLICATE_EFFECT_ENGINE
    );
    let [
        Target::Timeline(clip @ TimelineId::Clip(clip_id)),
        Target::Timeline(child @ TimelineId::Item(child_id)),
        Target::Timeline(nested @ TimelineId::Item(nested_id)),
        Target::Timeline(overlay @ TimelineId::Item(_)),
    ] = pasted.report.changes[0].created.as_slice()
    else {
        panic!("expected the clip, its two descendants, and the selected overlay");
    };
    let project = store.project();
    let anchor = |id| {
        project
            .items
            .timing(project.items.find(id).unwrap())
            .unwrap()
            .anchor
    };
    assert!(matches!(anchor(*child_id), Anchor::ClipSource { clip, .. } if clip == *clip_id));
    assert!(matches!(anchor(*nested_id), Anchor::Item { item, .. } if item == *child));
    assert_eq!(
        duplicate_effects(project, source[0])[0].params[0].value,
        EffectValue::Number(0.12)
    );
    for ((copy, source), prior) in [*clip, *child, *nested, *overlay]
        .into_iter()
        .zip(source)
        .zip(&before)
    {
        let copied = duplicate_effects(project, copy);
        let original = duplicate_effects(project, source);
        assert_eq!(copied.len(), prior.len());
        for ((copy, original), prior) in copied.iter().zip(original).zip(prior) {
            assert_eq!(original.id, prior.id);
            assert_ne!(copy.id, prior.id);
            let mut expected = prior.clone();
            expected.id = copy.id;
            assert_eq!(*copy, expected);
        }
    }
    let overlay = *overlay;
    let overlay_fx = duplicate_effects(project, overlay)[0].id;
    let batch = state_hash(project).unwrap();

    store
        .apply(
            &[op(&format!(
                r#"{{"cmd":"effect.set","effect":"{overlay_fx}","params":{{"scale":1.3}}}}"#,
            ))],
            &human(3),
        )
        .unwrap();
    assert_eq!(duplicate_effects(store.project(), source[3]), before[3]);
    assert_eq!(
        duplicate_effects(store.project(), overlay)[0].params[0].value,
        EffectValue::Number(1.3)
    );
    let edited = state_hash(store.project()).unwrap();
    store.close().unwrap();

    let mut store = Store::open(&temp.0).unwrap();
    assert_eq!(store.verify().unwrap(), edited);
    store.human_undo(4).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), batch);
    store.human_undo(5).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), before_paste);
    store.human_redo(6).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), batch);
    store.human_redo(7).unwrap();
    assert_eq!(state_hash(store.project()).unwrap(), edited);

    // A new copy must also mint its own ID when the source is gone at settle.
    let result = store
        .apply(
            op(r#"[
        {"cmd":"item.duplicate","items":["clp_end"]},
        {"cmd":"item.delete","items":["clp_end"]}
    ]"#)
            .as_array()
            .unwrap(),
            &human(8),
        )
        .unwrap();
    let Target::Timeline(copy @ TimelineId::Clip(_)) = result.report.changes[0].created[0] else {
        panic!("expected a copied clip");
    };
    assert!(
        store
            .project()
            .clips
            .find("clp_end".parse().unwrap())
            .is_none()
    );
    let effect = &duplicate_effects(store.project(), copy)[0];
    assert_ne!(effect.id, "fx_endwin".parse().unwrap());
    assert_eq!(effect.kind, "window");
    assert_eq!(effect.enabled, Some(false));
    let replaced = state_hash(store.project()).unwrap();
    store.close().unwrap();
    assert_eq!(Store::open(&temp.0).unwrap().verify().unwrap(), replaced);
}

#[test]
fn duplicate_effects_keep_legacy_mint_order_and_unstyled_mints() {
    let temp = Temp::new("duplicate-effects-legacy");
    drop(sample_store(&temp.0));
    // Real pre-change Store batches with effect IDs minted at settle, after the marker IDs.
    let journal = concat!(
        r#"{"rev":1,"parent":"6d4ac5c362bbe1d2","hash":"e55b26437a594fd3","engine":4,"time":1,"source":"human","label":"Duplicated 1 item(s) · window on clp_5k · Added marker mrk_5m at frame 20","ops":[{"cmd":"item.duplicate","items":["clp_open"],"at":0},{"cmd":"effect.set","effect":"fx_openwin","params":{"padding":0.12}},{"cmd":"marker.add","at":20,"name":"After the copy"}],"mints":["clp_5k","ovl_5l","mrk_5m","fx_5n"]}"#,
        "\n",
        r#"{"rev":2,"parent":"e55b26437a594fd3","hash":"65fd678e7e2fd547","engine":5,"time":2,"source":"human","label":"Duplicated 1 item(s) · Caption ovl_cap1 · Added marker mrk_5p at frame 30","ops":[{"cmd":"item.duplicate","items":["clp_end"]},{"cmd":"caption.set","item":"ovl_cap1","words":{}},{"cmd":"marker.add","at":30,"name":"Mixed engine batch"}],"mints":["clp_5o","mrk_5p","fx_5q"]}"#,
        "\n",
    );
    fs::write(temp.0.join("journal.ops"), journal).unwrap();
    let store = Store::open(&temp.0).unwrap();
    assert_eq!(store.project().rev, 2);
    assert_eq!(
        state_hash(&store.at(1).unwrap()).unwrap(),
        0x7f02_3965_6b08_03a7
    );
    assert_eq!(store.verify().unwrap(), 0x9bbc_fa5e_8f66_4703);
    store.close().unwrap();

    let mut store = Store::open(&temp.0).unwrap();
    assert_eq!(store.verify().unwrap(), 0x9bbc_fa5e_8f66_4703);
    let unstyled = store
        .apply(
            op(r#"[
        {"cmd":"item.duplicate","items":["ovl_broll"],"at":0},
        {"cmd":"marker.add","at":10,"name":"No effect mint"}
    ]"#)
            .as_array()
            .unwrap(),
            &human(3),
        )
        .unwrap();
    assert_eq!(unstyled.report.mints, ["ovl_5r", "mrk_5s"]);
    let extended = state_hash(store.project()).unwrap();
    store.close().unwrap();
    assert_eq!(Store::open(&temp.0).unwrap().verify().unwrap(), extended);
}
