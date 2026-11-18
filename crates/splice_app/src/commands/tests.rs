use super::*;
use splice_core::history::{Source as BatchSource, WriteOptions};

#[test]
fn viewer_keyboard_invokes_one_play_and_exact_history_chords() {
    let mut keys = Keys::new(None);
    let mut seen = Vec::new();
    for event in [
        Event::KeyPressed(HostKey::Space),
        Event::KeyPressed(HostKey::Space),
        Event::KeyReleased(HostKey::Space),
        Event::KeyPressed(HostKey::ArrowLeft),
        Event::KeyPressed(HostKey::ArrowRight),
        Event::KeyPressed(HostKey::Meta),
        Event::KeyPressed(HostKey::KeyChar('z')),
        Event::KeyPressed(HostKey::Shift),
        Event::KeyPressed(HostKey::KeyChar('Z')),
        Event::FocusLost,
        Event::KeyPressed(HostKey::KeyChar('z')),
    ] {
        keys.route(&event, &mut |action| seen.push(action.id));
    }
    assert_eq!(seen, [PLAY, BACK, FORWARD, UNDO, REDO]);
    assert_eq!(keys.play_button().source, Source::Control);
}

#[test]
fn viewer_history_keys_commit_and_reopen_core_revisions() {
    let dir = std::env::temp_dir().join(format!("splice-viewer-history-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("Keys.splice");
    let mut store = Store::create(&path, &Default::default()).unwrap();
    let session = store.start_session("live-test", 0).unwrap();
    let ops = splice_json::parse(r#"[{"cmd":"marker.add","at":12,"name":"Agent edit"}]"#).unwrap();
    store
        .apply(
            ops.as_array().unwrap(),
            &WriteOptions {
                source: BatchSource::Agent,
                session: Some(session.id),
                if_revision: Some(0),
                label: Some("Agent: Add marker".into()),
                ..WriteOptions::default()
            },
        )
        .unwrap();
    let mut history = History::new(&store);
    assert_eq!(history.ring.titles(), (Some("Agent: Add marker"), None));
    let mut keys = Keys::new(None);
    for event in [
        Event::KeyPressed(HostKey::Meta),
        Event::KeyPressed(HostKey::KeyChar('z')),
        Event::KeyPressed(HostKey::Shift),
        Event::KeyPressed(HostKey::KeyChar('Z')),
    ] {
        keys.route(&event, &mut |action| {
            crate::dispatch(action, Some(&mut store), Some(&mut history), None).unwrap();
            assert_eq!(store.project().markers.is_empty(), action.id == UNDO);
        });
    }
    assert_eq!(store.project().rev, 3);
    assert_eq!(store.human_history_available(), (true, false));
    store.close().unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.records()[1].batch.change, Change::Undo(0));
    assert_eq!(store.records()[2].batch.change, Change::Redo(1));
    assert_eq!(
        History::new(&store).ring.titles(),
        (Some("Agent: Add marker"), None)
    );
    store.close().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
