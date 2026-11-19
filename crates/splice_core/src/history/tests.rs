use super::*;
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
