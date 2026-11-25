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
        Event::KeyPressed(HostKey::KeyChar('?')),
        Event::KeyPressed(HostKey::Meta),
        Event::KeyPressed(HostKey::KeyChar('k')),
    ] {
        keys.route(&event, &mut |action| seen.push(action.id));
    }
    assert_eq!(
        seen,
        [
            PLAY, BACK, FORWARD, UNDO, REDO, ADD_MOTION, SHORTCUTS, PALETTE
        ]
    );
    assert_eq!(keys.play_button().source, Source::Control);
}

#[test]
fn viewer_keyboard_routes_reference_editor_chords() {
    let mut keys = Keys::new(None);
    let cases: &[(&[HostKey], HostKey, Option<CommandId>)] = &[
        (&[], HostKey::Home, Some(START)),
        (&[], HostKey::End, Some(END)),
        (&[HostKey::Shift], HostKey::ArrowLeft, Some(BACK_SECOND)),
        (&[HostKey::Shift], HostKey::ArrowRight, Some(FORWARD_SECOND)),
        (&[], HostKey::KeyChar('v'), Some(SELECT_TOOL)),
        (&[], HostKey::KeyChar('s'), Some(BLADE_TOOL)),
        (&[], HostKey::KeyChar('n'), Some(SNAP)),
        (&[], HostKey::KeyChar('c'), Some(SPLIT)),
        (&[HostKey::Shift], HostKey::KeyChar('C'), Some(CROP)),
        (&[], HostKey::KeyChar('z'), Some(ADD_MOTION)),
        (&[], HostKey::KeyChar('t'), Some(ADD_TITLE)),
        (&[], HostKey::KeyChar('m'), Some(ADD_MARKER)),
        (&[], HostKey::KeyChar('q'), Some(TRIM_START)),
        (&[], HostKey::KeyChar('w'), Some(TRIM_END)),
        (&[], HostKey::Backspace, Some(DELETE)),
        (&[], HostKey::Delete, Some(DELETE)),
        (&[HostKey::Shift], HostKey::Backspace, Some(DELETE_GAP)),
        (&[HostKey::Shift], HostKey::Delete, Some(DELETE_GAP)),
        (&[HostKey::Meta], HostKey::KeyChar('d'), Some(DUPLICATE)),
        (&[HostKey::Meta], HostKey::KeyChar('x'), Some(CUT)),
        (&[HostKey::Meta], HostKey::KeyChar('c'), Some(COPY)),
        (&[HostKey::Meta], HostKey::KeyChar('v'), Some(PASTE)),
        (&[], HostKey::KeyChar(','), Some(NUDGE_BACK)),
        (&[], HostKey::KeyChar('.'), Some(NUDGE_FORWARD)),
        (
            &[HostKey::Shift],
            HostKey::KeyChar(','),
            Some(NUDGE_BACK_TEN),
        ),
        (
            &[HostKey::Shift],
            HostKey::KeyChar('>'),
            Some(NUDGE_FORWARD_TEN),
        ),
        (
            &[HostKey::Meta],
            HostKey::KeyChar('a'),
            Some(SELECT_LANE_ITEMS),
        ),
        (&[HostKey::Alt], HostKey::ArrowUp, Some(LANE_UP)),
        (&[HostKey::Alt], HostKey::ArrowDown, Some(LANE_DOWN)),
        (
            &[HostKey::Alt, HostKey::Meta],
            HostKey::ArrowUp,
            Some(LIFT_TO_OVERLAY),
        ),
        (&[], HostKey::KeyChar('f'), Some(ZOOM_SELECTION)),
        (&[], HostKey::KeyChar('='), Some(ZOOM_TIMELINE_IN)),
        (&[], HostKey::KeyChar('-'), Some(ZOOM_TIMELINE_OUT)),
        (&[], HostKey::Escape, Some(ESCAPE)),
        (&[HostKey::Meta], HostKey::KeyChar('f'), None),
        (
            &[HostKey::Meta, HostKey::Alt],
            HostKey::KeyChar('v'),
            Some(PASTE_EFFECTS),
        ),
        (&[], HostKey::KeyChar('d'), None),
        (&[HostKey::Meta], HostKey::KeyChar('t'), None),
        (&[HostKey::Alt], HostKey::KeyChar('q'), None),
        (&[HostKey::Shift, HostKey::Meta], HostKey::Delete, None),
        (
            &[HostKey::Shift, HostKey::Meta],
            HostKey::KeyChar('D'),
            None,
        ),
        (
            &[HostKey::Alt, HostKey::Meta],
            HostKey::ArrowDown,
            Some(REPLACE_SPINE),
        ),
        (&[HostKey::Meta], HostKey::KeyChar('='), Some(ZOOM_IN)),
        (&[HostKey::Meta], HostKey::KeyChar('-'), Some(ZOOM_OUT)),
        (&[HostKey::Shift], HostKey::KeyChar('Z'), Some(FIT)),
        (
            &[HostKey::Alt, HostKey::Meta],
            HostKey::KeyChar('z'),
            Some(SESSION_UNDO),
        ),
        (&[HostKey::Shift], HostKey::Home, None),
        (
            &[HostKey::Meta, HostKey::Shift],
            HostKey::KeyChar('c'),
            None,
        ),
    ];
    for &(modifiers, key, expected) in cases {
        let mut action = None;
        let mut invoke = |invoked: Invoked| action = Some(invoked.id);
        assert!(!keys.route(&Event::FocusLost, &mut invoke));
        for &modifier in modifiers {
            assert!(!keys.route(&Event::KeyPressed(modifier), &mut invoke));
        }
        assert_eq!(
            keys.route(&Event::KeyPressed(key), &mut invoke),
            expected.is_some()
        );
        assert_eq!(action, expected, "{modifiers:?} {key:?}");
    }

    for enabled in [true, false] {
        let states = states(enabled, false, (false, false));
        for id in [START, END, BACK_SECOND, FORWARD_SECOND] {
            assert_eq!(states[id.0 as usize].enabled, enabled);
        }
    }
    let help = keys.bindings();
    assert_eq!(help.len(), VIEW.len());
    assert_eq!(
        help.iter().find(|row| row.id == SHORTCUTS).unwrap().keys,
        "?"
    );
}

#[test]
fn shortcut_help_lists_effective_keymap_bindings_without_allocating() {
    let path = std::env::temp_dir().join(format!("splice-viewer-keymap-{}", std::process::id()));
    fs::write(
        &path,
        "# Move Space to Start and replace Active Pane Zoom In\n\
         Space = transport.start\nHome = transport.start\nCmd+Equals = transport.play\n\
         ? = timeline.snapping\nF5 = item.split\nShift+Left = marker.add\nBackspace = history.undo\n",
    )
    .unwrap();
    let mut keys = Keys::new(Some(&path));
    fs::remove_file(path).unwrap();

    let expected = [
        (START, Some("Home · Space")),
        (PLAY, Some("⌘=")),
        (ZOOM_IN, None),
        (BACK_SECOND, None),
        (SHORTCUTS, Some("⇧? · ⇧/")),
        (SNAP, Some("N · ?")),
        (SPLIT, Some("C · F5")),
        (ADD_MARKER, Some("M · ⇧←")),
        (DELETE, Some("⌦")),
        (UNDO, Some("⌘Z · ⌫")),
    ];
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            let help = keys.bindings();
            for (id, label) in expected {
                let actual = help.iter().find(|row| row.id == id);
                assert_eq!(actual.map(|row| row.keys.as_str()), label);
            }
            let split = help.iter().find(|row| row.id == SPLIT).unwrap();
            assert_eq!((split.group, split.title), ("Edit", "Split at Playhead"));
            assert!(
                keys.route(&Event::KeyPressed(HostKey::Backspace), &mut |action| {
                    assert_eq!(action.id, UNDO);
                })
            );
            let mut seen = None;
            assert!(keys.route(&Event::KeyPressed(HostKey::F5), &mut |action| {
                seen = Some(action.id);
            }));
            assert_eq!(seen, Some(SPLIT));
        },
    );
}

fn shortcut_project() -> Project {
    use splice_model::Clip;
    let mut project = Project {
        serial: 2,
        ..Project::default()
    };
    for (serial, start) in [(1, 0), (2, 60)] {
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(serial),
                timing: ClipTiming {
                    start,
                    length: 60,
                    source: 0,
                    speed: 1.0,
                },
                data: Default::default(),
            })
            .unwrap();
    }
    project
}

#[test]
fn edit_chords_check_playhead_and_live_selection() {
    let project = shortcut_project();
    let clip = TimelineId::Clip(ClipId::from_serial(1));
    let stale = TimelineId::Clip(ClipId::from_serial(9));
    for id in [TRIM_START, TRIM_END] {
        for at in [-1, 0, 2, 58, 60, 118, 120] {
            assert!(
                !edit_enabled(id, &project, &[clip], None, at),
                "{id:?} @ {at}"
            );
        }
        for at in [3, 12, 57, 63, 117] {
            assert!(edit_enabled(id, &project, &[], None, at), "{id:?} @ {at}");
        }
    }
    for id in [DELETE, DELETE_GAP, DUPLICATE] {
        assert!(!edit_enabled(id, &project, &[], None, 0));
        assert!(!edit_enabled(id, &project, &[clip, stale], None, 0));
        assert!(edit_enabled(id, &project, &[clip], None, 0));
        assert!(!edit_enabled(id, &project, &[clip], Some(false), 0));
    }
    assert!(edit_enabled(DELETE, &project, &[clip], Some(true), 0));
    assert!(!edit_enabled(DUPLICATE, &project, &[clip], Some(true), 0));
    assert!(edit_enabled(ADD_TITLE, &project, &[], None, 117));
    assert!(!edit_enabled(ADD_TITLE, &project, &[], None, 118));
    assert!(edit_enabled(ADD_MARKER, &project, &[], None, 120));
    assert!(!edit_enabled(ADD_MARKER, &project, &[], None, 121));
}

#[test]
fn edit_chords_commit_native_toolbar_and_trim_operations() {
    let path = std::env::temp_dir().join(format!(
        "splice-viewer-edit-keys-{}.splice",
        std::process::id()
    ));
    let mut store = Store::create(&path, &shortcut_project()).unwrap();
    let mut state = crate::timeline::State::default();
    let mut keys = Keys::new(None);
    let mut press =
        |store: &mut Store, state: &mut crate::timeline::State, modifiers: &[HostKey], key, at| {
            keys.route(&Event::FocusLost, &mut |_| {});
            for &modifier in modifiers {
                assert!(!keys.route(&Event::KeyPressed(modifier), &mut |_| {}));
            }
            let mut seek = None;
            assert!(keys.route(&Event::KeyPressed(key), &mut |action| {
                seek = crate::dispatch_edit_shortcut(action.id, store, state, at);
            }));
            seek
        };

    press(&mut store, &mut state, &[], HostKey::KeyChar('t'), 12);
    assert_eq!(store.project().items.order().len(), 1);
    press(&mut store, &mut state, &[], HostKey::KeyChar('m'), 14);
    let marker = &store.project().markers[0];
    assert_eq!(marker.at, 14);
    assert_eq!(state.selection(), Some(TimelineId::Marker(marker.id)));
    press(&mut store, &mut state, &[], HostKey::KeyChar('m'), 14);
    assert_eq!(store.project().rev, 2);
    press(
        &mut store,
        &mut state,
        &[HostKey::Shift],
        HostKey::Backspace,
        14,
    );
    assert!(store.project().markers.is_empty());

    let first = ClipId::from_serial(1);
    let second = ClipId::from_serial(2);
    state.set_selection(Some(TimelineId::Clip(second)), None);
    press(&mut store, &mut state, &[], HostKey::KeyChar('q'), 2);
    press(&mut store, &mut state, &[], HostKey::KeyChar('w'), 58);
    assert_eq!(store.project().rev, 3);
    assert_eq!(
        press(&mut store, &mut state, &[], HostKey::KeyChar('q'), 12),
        Some(0)
    );
    press(&mut store, &mut state, &[], HostKey::KeyChar('w'), 36);
    let timing = |store: &Store, id| {
        let clips = &store.project().clips;
        clips.timing(clips.find(id).unwrap()).unwrap()
    };
    assert_eq!(timing(&store, first).length, 36);
    assert_eq!(timing(&store, second).start, 36);
    state.set_selection(Some(TimelineId::Clip(first)), None);
    press(
        &mut store,
        &mut state,
        &[HostKey::Meta],
        HostKey::KeyChar('d'),
        12,
    );
    assert_eq!(store.project().clips.order().len(), 3);
    assert_eq!(timing(&store, second).start, 72);
    press(
        &mut store,
        &mut state,
        &[HostKey::Shift],
        HostKey::Delete,
        12,
    );
    assert_eq!(timing(&store, second).start, 72);
    assert!(store.project().clips.find(first).is_none());
    let clips = &store.project().clips;
    state.set_selection(clips.id(clips.order()[0]).map(TimelineId::Clip), None);
    press(&mut store, &mut state, &[], HostKey::Backspace, 50);
    assert_eq!(timing(&store, second).start, 36);
    assert_eq!(store.project().rev, 8);
    store.close().unwrap();
    fs::remove_dir_all(path).unwrap();
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
