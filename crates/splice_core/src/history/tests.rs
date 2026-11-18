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
