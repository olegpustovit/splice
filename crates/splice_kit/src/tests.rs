use super::*;

#[test]
fn scrub_phase_distinguishes_preview_release_and_cancellation() {
    assert_eq!(Phase::from_interaction(false, false, false), None);
    assert_eq!(
        Phase::from_interaction(false, true, false),
        Some(Phase::Began)
    );
    assert_eq!(
        Phase::from_interaction(true, true, false),
        Some(Phase::Changed)
    );
    assert_eq!(
        Phase::from_interaction(true, false, true),
        Some(Phase::Ended)
    );
    assert_eq!(
        Phase::from_interaction(true, false, false),
        Some(Phase::Cancelled)
    );
    assert_eq!(
        Phase::from_interaction(false, false, true),
        Some(Phase::Ended)
    );
}

const PLAY: CommandId = CommandId(1);
const UNDO: CommandId = CommandId(2);
const TEXT_UNDO: CommandId = CommandId(3);
const SPACE: Shortcut = Shortcut::new(Key::Space, Modifiers::NONE);
const CMD_Z: Shortcut = Shortcut::new(Key::Char('z'), Modifiers::CMD);
const CMD_SHIFT_Z: Shortcut = Shortcut::new(Key::Char('z'), Modifiers::CMD.union(Modifiers::SHIFT));
const TABLE: &[Command] = &[
    Command {
        id: PLAY,
        name: "transport.play",
        title: "Play",
        keys: &[SPACE],
        group: "Transport",
    },
    Command {
        id: UNDO,
        name: "history.undo",
        title: "Undo",
        keys: &[CMD_Z],
        group: "History",
    },
    Command {
        id: TEXT_UNDO,
        name: "text.undo",
        title: "Undo Typing",
        keys: &[CMD_Z],
        group: "Text",
    },
];

#[test]
fn commands_resolve_focus_ancestors_window_and_app_without_allocating() {
    let mut responders = Responders::new(5);
    responders.responds(1, 0, &[PLAY]);
    responders.responds(2, 1, &[UNDO]);
    responders.responds(3, 2, &[TEXT_UNDO]);
    responders.responds(4, 3, &[]);
    responders.responds(5, 5, &[]);

    let run = || {
        let undo = Shortcut::new(Key::Char('Z'), Modifiers::CMD);
        assert_eq!(
            responders.resolve(4, undo, TABLE),
            Some(Invoked {
                id: TEXT_UNDO,
                source: Source::Key,
                target: 3
            })
        );
        assert_eq!(responders.resolve(2, undo, TABLE).unwrap().id, UNDO);
        assert_eq!(responders.resolve(4, SPACE, TABLE).unwrap().target, 1);
        assert_eq!(responders.resolve(4, CMD_SHIFT_Z, TABLE), None);
        assert_eq!(responders.resolve(5, SPACE, TABLE), None);
        for source in [
            Source::Menu,
            Source::Palette,
            Source::A11y,
            Source::Automation,
        ] {
            assert_eq!(
                responders.invoke(4, UNDO, source),
                Some(Invoked {
                    id: UNDO,
                    source,
                    target: 2
                })
            );
        }
    };
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for _ in 0..256 {
                run();
            }
        },
    );

    responders.responds(3, 2, &[]);
    assert_eq!(responders.resolve(4, CMD_Z, TABLE).unwrap().target, 2);
    responders.responds(1, 4, &[PLAY]);
    assert_eq!(responders.resolve(4, CMD_SHIFT_Z, TABLE), None);
}

#[test]
fn commands_validate_into_reused_state() {
    let mut states = [CommandState::default(); 3];
    validate(TABLE, &mut states, &mut |id, state| {
        if id == PLAY {
            state.checked = true;
            state.title = Some("Pause");
        }
        if id == UNDO {
            state.enabled = false;
        }
    });
    assert_eq!(
        states[0],
        CommandState {
            enabled: true,
            checked: true,
            title: Some("Pause")
        }
    );
    assert!(!states[1].enabled);
    validate(TABLE, &mut states, &mut |_, _| {});
    assert_eq!(states, [CommandState::default(); 3]);
}

#[test]
fn commands_load_overrides_and_reject_conflicts() {
    let mut responders = Responders::new(2);
    responders.responds(1, 0, &[PLAY, UNDO]);
    responders.responds(2, 1, &[TEXT_UNDO]);
    let temp = std::env::temp_dir().join(format!("splice-keymap-{}", std::process::id()));
    std::fs::create_dir(&temp).unwrap();
    let path = temp.join("keymap");
    std::fs::write(
        &path,
        "# Custom keys\n Cmd+z = transport.play\n Shift+Cmd+Z = history.undo\n F2 = text.undo\n",
    )
    .unwrap();
    let keymap = Keymap::read(&path, TABLE);
    std::fs::remove_dir_all(temp).unwrap();
    responders.set_keymap(keymap.unwrap());
    let f2 = Shortcut::new(Key::Function(2), Modifiers::NONE);
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            assert_eq!(responders.resolve(2, SPACE, TABLE).unwrap().id, PLAY);
            assert_eq!(responders.resolve(2, CMD_Z, TABLE).unwrap().id, PLAY);
            assert_eq!(responders.resolve(2, CMD_SHIFT_Z, TABLE).unwrap().id, UNDO);
            assert_eq!(responders.resolve(2, f2, TABLE).unwrap().id, TEXT_UNDO);
            assert_eq!(responders.resolve(1, f2, TABLE), None);
        },
    );
    assert!(matches!(
        Keymap::parse(
            "Cmd+Shift+Z = history.undo\nShift+Cmd+z = transport.play",
            TABLE
        ),
        Err(KeymapError::Conflict {
            line: 2,
            previous: 1
        })
    ));
    assert!(matches!(
        Keymap::parse("Cmd+Z = unknown", TABLE),
        Err(KeymapError::UnknownCommand { line: 1 })
    ));
    for invalid in ["Cmd+Cmd+Z", "Cmd+", "F13", "Cmd+Z+B", "???", ""] {
        let map = format!("{invalid} = transport.play");
        assert!(
            matches!(
                Keymap::parse(&map, TABLE),
                Err(KeymapError::InvalidBinding { line: 1 })
            ),
            "{map}"
        );
    }
}
