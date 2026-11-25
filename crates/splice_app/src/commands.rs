use input::{Event, Key as HostKey};
use splice_core::{
    history::{Store, StoreError},
    package::journal::Change,
};
use splice_kit::{
    Command, CommandId, CommandState, Entry, Invoked, Key, Keymap, KeymapError, Merge, Modifiers,
    Responders, Shortcut, Source, UndoStack, validate,
};
use splice_model::{ClipId, ClipTiming, ItemKind, Project, TimelineId};
use std::{borrow::Cow, fs, io, path::Path};

pub(super) const PLAY: CommandId = CommandId(0);
pub(super) const BACK: CommandId = CommandId(1);
pub(super) const FORWARD: CommandId = CommandId(2);
pub(super) const UNDO: CommandId = CommandId(3);
pub(super) const REDO: CommandId = CommandId(4);
pub(super) const PALETTE: CommandId = CommandId(5);
pub(super) const SHORTCUTS: CommandId = CommandId(6);
pub(super) const START: CommandId = CommandId(7);
pub(super) const END: CommandId = CommandId(8);
pub(super) const BACK_SECOND: CommandId = CommandId(9);
pub(super) const FORWARD_SECOND: CommandId = CommandId(10);
pub(super) const SELECT_TOOL: CommandId = CommandId(11);
pub(super) const BLADE_TOOL: CommandId = CommandId(12);
pub(super) const SNAP: CommandId = CommandId(13);
pub(super) const SPLIT: CommandId = CommandId(14);
pub(super) const CROP: CommandId = CommandId(15);
pub(super) const ZOOM_IN: CommandId = CommandId(16);
pub(super) const ZOOM_OUT: CommandId = CommandId(17);
pub(super) const FIT: CommandId = CommandId(18);
pub(super) const SESSION_UNDO: CommandId = CommandId(19);
pub(super) const ADD_MOTION: CommandId = CommandId(20);
pub(super) const REPLACE_SPINE: CommandId = CommandId(21);
pub(super) const ADD_TITLE: CommandId = CommandId(22);
pub(super) const ADD_MARKER: CommandId = CommandId(23);
pub(super) const TRIM_START: CommandId = CommandId(24);
pub(super) const TRIM_END: CommandId = CommandId(25);
pub(super) const DELETE: CommandId = CommandId(26);
pub(super) const DELETE_GAP: CommandId = CommandId(27);
pub(super) const DUPLICATE: CommandId = CommandId(28);
pub(super) const SHUTTLE_REVERSE: CommandId = CommandId(29);
pub(super) const STOP: CommandId = CommandId(30);
pub(super) const SHUTTLE_FORWARD: CommandId = CommandId(31);
pub(super) const PREVIOUS_EDIT: CommandId = CommandId(32);
pub(super) const NEXT_EDIT: CommandId = CommandId(33);
pub(super) const CUT: CommandId = CommandId(34);
pub(super) const COPY: CommandId = CommandId(35);
pub(super) const PASTE: CommandId = CommandId(36);
pub(super) const PASTE_EFFECTS: CommandId = CommandId(37);
pub(super) const NUDGE_BACK: CommandId = CommandId(38);
pub(super) const NUDGE_FORWARD: CommandId = CommandId(39);
pub(super) const NUDGE_BACK_TEN: CommandId = CommandId(40);
pub(super) const NUDGE_FORWARD_TEN: CommandId = CommandId(41);
pub(super) const SELECT_LANE_ITEMS: CommandId = CommandId(42);
pub(super) const LIFT_TO_OVERLAY: CommandId = CommandId(43);
pub(super) const LANE_UP: CommandId = CommandId(44);
pub(super) const LANE_DOWN: CommandId = CommandId(45);
pub(super) const ZOOM_SELECTION: CommandId = CommandId(46);
pub(super) const ESCAPE: CommandId = CommandId(47);
pub(super) const ZOOM_TIMELINE_IN: CommandId = CommandId(48);
pub(super) const ZOOM_TIMELINE_OUT: CommandId = CommandId(49);

pub(super) const EDIT_SHORTCUTS: [CommandId; 7] = [
    ADD_TITLE, ADD_MARKER, TRIM_START, TRIM_END, DELETE, DELETE_GAP, DUPLICATE,
];
pub(super) const CLIPBOARD_SHORTCUTS: [CommandId; 4] = [CUT, COPY, PASTE, PASTE_EFFECTS];
pub(super) const SELECTION_SHORTCUTS: [CommandId; 12] = [
    NUDGE_BACK,
    NUDGE_FORWARD,
    NUDGE_BACK_TEN,
    NUDGE_FORWARD_TEN,
    SELECT_LANE_ITEMS,
    LIFT_TO_OVERLAY,
    LANE_UP,
    LANE_DOWN,
    ZOOM_SELECTION,
    ESCAPE,
    ZOOM_TIMELINE_IN,
    ZOOM_TIMELINE_OUT,
];

mod shortcuts;
pub(super) use shortcuts::{GROUPS as SHORTCUT_GROUPS, ShortcutRow};

const WINDOW: u64 = 1;
const CATALOG: [(Command, &str); 50] = [
    binding(
        PLAY,
        "transport.play",
        "Play / Pause",
        "Transport",
        &[Shortcut::new(Key::Space, Modifiers::NONE)],
        "Space",
    ),
    binding(
        SHUTTLE_REVERSE,
        "transport.shuttleReverse",
        "Shuttle Reverse",
        "Transport",
        &[Shortcut::new(Key::Char('j'), Modifiers::NONE)],
        "J",
    ),
    binding(
        STOP,
        "transport.stop",
        "Stop Playback",
        "Transport",
        &[Shortcut::new(Key::Char('k'), Modifiers::NONE)],
        "K",
    ),
    binding(
        SHUTTLE_FORWARD,
        "transport.shuttleForward",
        "Shuttle Forward",
        "Transport",
        &[Shortcut::new(Key::Char('l'), Modifiers::NONE)],
        "L",
    ),
    binding(
        BACK,
        "transport.stepBack",
        "Previous Frame",
        "Transport",
        &[Shortcut::new(Key::Left, Modifiers::NONE)],
        "←",
    ),
    binding(
        FORWARD,
        "transport.step",
        "Next Frame",
        "Transport",
        &[Shortcut::new(Key::Right, Modifiers::NONE)],
        "→",
    ),
    binding(
        BACK_SECOND,
        "transport.stepBackSecond",
        "Back One Second",
        "Transport",
        &[Shortcut::new(Key::Left, Modifiers::SHIFT)],
        "⇧←",
    ),
    binding(
        FORWARD_SECOND,
        "transport.stepSecond",
        "Forward One Second",
        "Transport",
        &[Shortcut::new(Key::Right, Modifiers::SHIFT)],
        "⇧→",
    ),
    binding(
        PREVIOUS_EDIT,
        "transport.previousEdit",
        "Previous Edit Point",
        "Transport",
        &[Shortcut::new(Key::Up, Modifiers::NONE)],
        "↑",
    ),
    binding(
        NEXT_EDIT,
        "transport.nextEdit",
        "Next Edit Point",
        "Transport",
        &[Shortcut::new(Key::Down, Modifiers::NONE)],
        "↓",
    ),
    binding(
        START,
        "transport.start",
        "Go to Start",
        "Transport",
        &[Shortcut::new(Key::Home, Modifiers::NONE)],
        "Home",
    ),
    binding(
        END,
        "transport.end",
        "Go to End",
        "Transport",
        &[Shortcut::new(Key::End, Modifiers::NONE)],
        "End",
    ),
    binding(
        SELECT_TOOL,
        "timeline.selectTool",
        "Select Tool",
        "Tools",
        &[Shortcut::new(Key::Char('v'), Modifiers::NONE)],
        "V",
    ),
    binding(
        BLADE_TOOL,
        "timeline.bladeTool",
        "Blade Tool",
        "Tools",
        &[Shortcut::new(Key::Char('s'), Modifiers::NONE)],
        "S",
    ),
    binding(
        SNAP,
        "timeline.snapping",
        "Toggle Snapping",
        "Tools",
        &[Shortcut::new(Key::Char('n'), Modifiers::NONE)],
        "N",
    ),
    binding(
        SPLIT,
        "item.split",
        "Split at Playhead",
        "Edit",
        &[Shortcut::new(Key::Char('c'), Modifiers::NONE)],
        "C",
    ),
    binding(
        CROP,
        "clip.crop",
        "Crop",
        "Edit",
        &[Shortcut::new(Key::Char('c'), Modifiers::SHIFT)],
        "⇧C",
    ),
    binding(
        ADD_TITLE,
        "title.add",
        "Add Title",
        "Add",
        &[Shortcut::new(Key::Char('t'), Modifiers::NONE)],
        "T",
    ),
    binding(
        ADD_MARKER,
        "marker.add",
        "Add Marker",
        "Add",
        &[Shortcut::new(Key::Char('m'), Modifiers::NONE)],
        "M",
    ),
    binding(
        TRIM_START,
        "item.trim",
        "Trim Start to Playhead",
        "Edit",
        &[Shortcut::new(Key::Char('q'), Modifiers::NONE)],
        "Q",
    ),
    binding(
        TRIM_END,
        "item.trimEnd",
        "Trim End to Playhead",
        "Edit",
        &[Shortcut::new(Key::Char('w'), Modifiers::NONE)],
        "W",
    ),
    binding(
        DELETE,
        "item.delete",
        "Delete Selection",
        "Edit",
        &[
            Shortcut::new(Key::Backspace, Modifiers::NONE),
            Shortcut::new(Key::Delete, Modifiers::NONE),
        ],
        "⌫ · ⌦",
    ),
    binding(
        DELETE_GAP,
        "item.deleteLeavingGap",
        "Delete Leaving Gap",
        "Edit",
        &[
            Shortcut::new(Key::Backspace, Modifiers::SHIFT),
            Shortcut::new(Key::Delete, Modifiers::SHIFT),
        ],
        "⇧⌫ · ⇧⌦",
    ),
    binding(
        DUPLICATE,
        "item.duplicate",
        "Duplicate",
        "Edit",
        &[Shortcut::new(Key::Char('d'), Modifiers::CMD)],
        "⌘D",
    ),
    binding(
        CUT,
        "ui.cut",
        "Cut",
        "Edit",
        &[Shortcut::new(Key::Char('x'), Modifiers::CMD)],
        "⌘X",
    ),
    binding(
        COPY,
        "ui.copy",
        "Copy",
        "Edit",
        &[Shortcut::new(Key::Char('c'), Modifiers::CMD)],
        "⌘C",
    ),
    binding(
        PASTE,
        "ui.paste",
        "Paste",
        "Edit",
        &[Shortcut::new(Key::Char('v'), Modifiers::CMD)],
        "⌘V",
    ),
    binding(
        PASTE_EFFECTS,
        "effect.paste",
        "Paste Effects",
        "Edit",
        &[Shortcut::new(
            Key::Char('v'),
            Modifiers::CMD.union(Modifiers::ALT),
        )],
        "⌥⌘V",
    ),
    binding(
        NUDGE_BACK,
        "timeline.nudgeBack",
        "Nudge Back 1 Frame",
        "Edit",
        &[Shortcut::new(Key::Char(','), Modifiers::NONE)],
        ",",
    ),
    binding(
        NUDGE_FORWARD,
        "timeline.nudgeForward",
        "Nudge Forward 1 Frame",
        "Edit",
        &[Shortcut::new(Key::Char('.'), Modifiers::NONE)],
        ".",
    ),
    binding(
        NUDGE_BACK_TEN,
        "timeline.nudgeBackTen",
        "Nudge Back 10 Frames",
        "Edit",
        &[
            Shortcut::new(Key::Char(','), Modifiers::SHIFT),
            Shortcut::new(Key::Char('<'), Modifiers::SHIFT),
        ],
        "⇧,",
    ),
    binding(
        NUDGE_FORWARD_TEN,
        "timeline.nudgeForwardTen",
        "Nudge Forward 10 Frames",
        "Edit",
        &[
            Shortcut::new(Key::Char('.'), Modifiers::SHIFT),
            Shortcut::new(Key::Char('>'), Modifiers::SHIFT),
        ],
        "⇧.",
    ),
    binding(
        SELECT_LANE_ITEMS,
        "timeline.selectLaneItems",
        "Select All in Lane",
        "Edit",
        &[Shortcut::new(Key::Char('a'), Modifiers::CMD)],
        "⌘A",
    ),
    binding(
        ADD_MOTION,
        "motion.add",
        "Add Scale Animation",
        "Add",
        &[Shortcut::new(Key::Char('z'), Modifiers::NONE)],
        "Z",
    ),
    binding(
        REPLACE_SPINE,
        "overlay.replaceSpine",
        "Replace Spine Clip",
        "Lanes",
        &[Shortcut::new(
            Key::Down,
            Modifiers::CMD.union(Modifiers::ALT),
        )],
        "⌥⌘↓",
    ),
    binding(
        LIFT_TO_OVERLAY,
        "clip.lift",
        "Lift to Overlay",
        "Lanes",
        &[Shortcut::new(Key::Up, Modifiers::CMD.union(Modifiers::ALT))],
        "⌥⌘↑",
    ),
    binding(
        LANE_UP,
        "item.laneUp",
        "Move Overlay Up",
        "Lanes",
        &[Shortcut::new(Key::Up, Modifiers::ALT)],
        "⌥↑",
    ),
    binding(
        LANE_DOWN,
        "item.laneDown",
        "Move Overlay Down",
        "Lanes",
        &[Shortcut::new(Key::Down, Modifiers::ALT)],
        "⌥↓",
    ),
    binding(
        UNDO,
        "history.undo",
        "Undo",
        "History",
        &[Shortcut::new(Key::Char('z'), Modifiers::CMD)],
        "⌘Z",
    ),
    binding(
        REDO,
        "history.redo",
        "Redo",
        "History",
        &[Shortcut::new(
            Key::Char('z'),
            Modifiers::CMD.union(Modifiers::SHIFT),
        )],
        "⇧⌘Z",
    ),
    binding(
        SESSION_UNDO,
        "history.undoSession",
        "Undo All Agent Edits",
        "History",
        &[Shortcut::new(
            Key::Char('z'),
            Modifiers::CMD.union(Modifiers::ALT),
        )],
        "⌥⌘Z",
    ),
    binding(
        ZOOM_IN,
        "view.zoomIn",
        "Zoom In on Active Pane",
        "View",
        &[Shortcut::new(Key::Char('='), Modifiers::CMD)],
        "⌘=",
    ),
    binding(
        ZOOM_OUT,
        "view.zoomOut",
        "Zoom Out on Active Pane",
        "View",
        &[Shortcut::new(Key::Char('-'), Modifiers::CMD)],
        "⌘-",
    ),
    binding(
        ZOOM_SELECTION,
        "timeline.zoomSelection",
        "Zoom to Selection",
        "View",
        &[Shortcut::new(Key::Char('f'), Modifiers::NONE)],
        "F",
    ),
    binding(
        ZOOM_TIMELINE_IN,
        "timeline.zoomIn",
        "Zoom In on Timeline",
        "View",
        &[Shortcut::new(Key::Char('='), Modifiers::NONE)],
        "=",
    ),
    binding(
        ZOOM_TIMELINE_OUT,
        "timeline.zoomOut",
        "Zoom Out on Timeline",
        "View",
        &[Shortcut::new(Key::Char('-'), Modifiers::NONE)],
        "−",
    ),
    binding(
        FIT,
        "view.fit",
        "Fit Active Pane",
        "View",
        &[Shortcut::new(Key::Char('z'), Modifiers::SHIFT)],
        "⇧Z",
    ),
    binding(
        ESCAPE,
        "timeline.escape",
        "Cancel Drag, Leave Blade, or Clear Selection",
        "View",
        &[Shortcut::new(Key::Escape, Modifiers::NONE)],
        "Esc",
    ),
    binding(
        PALETTE,
        "viewer.commandPalette",
        "Command Palette",
        "Editor",
        &[Shortcut::new(Key::Char('k'), Modifiers::CMD)],
        "⌘K",
    ),
    binding(
        SHORTCUTS,
        "viewer.shortcuts",
        "Keyboard Shortcuts",
        "Editor",
        &[
            Shortcut::new(Key::Char('?'), Modifiers::NONE),
            Shortcut::new(Key::Char('?'), Modifiers::SHIFT),
            Shortcut::new(Key::Char('/'), Modifiers::SHIFT),
        ],
        "?",
    ),
];

const fn binding(
    id: CommandId,
    name: &'static str,
    title: &'static str,
    group: &'static str,
    keys: &'static [Shortcut],
    label: &'static str,
) -> (Command, &'static str) {
    (
        Command {
            id,
            name,
            title,
            keys,
            group,
        },
        label,
    )
}

const TABLES: ([Command; CATALOG.len()], [CommandId; CATALOG.len()]) = {
    let mut view = [const {
        Command {
            id: PLAY,
            name: "",
            title: "",
            keys: &[],
            group: "",
        }
    }; CATALOG.len()];
    let mut ids = [PLAY; CATALOG.len()];
    let mut index = 0;
    while index < CATALOG.len() {
        let command = &CATALOG[index].0;
        let slot = command.id.0 as usize;
        assert!(view[slot].name.is_empty());
        view[slot] = Command { ..*command };
        ids[slot] = command.id;
        index += 1;
    }
    (view, ids)
};
const VIEW: [Command; CATALOG.len()] = TABLES.0;
const IDS: [CommandId; CATALOG.len()] = TABLES.1;
pub(super) const COUNT: usize = VIEW.len();

pub(super) fn catalog() -> &'static [Command] {
    &VIEW
}

pub(super) const CHORD_SEPARATOR: &str = " · ";

pub(super) struct BindingDescription {
    pub(super) id: CommandId,
    pub(super) group: &'static str,
    pub(super) title: &'static str,
    pub(super) keys: String,
}

pub(super) struct Keys {
    responders: Responders,
    bindings: Vec<BindingDescription>,
    shortcuts: Vec<ShortcutRow>,
    modifiers: Modifiers,
    held_play_keys: Vec<HostKey>,
}

impl Keys {
    pub(super) fn new(keymap: Option<&Path>) -> Self {
        let mut responders = Responders::new(1);
        responders.responds(WINDOW, 0, &IDS);
        let mut custom = String::new();
        if let Some(path) = keymap {
            match load_keymap(path, &mut responders) {
                Ok(text) => custom = text,
                Err(KeymapError::Io(error)) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => eprintln!("splice: {}: {error}", path.display()),
            }
        }
        let bindings = describe_bindings(&responders, &custom);
        Self {
            shortcuts: shortcuts::describe(&bindings),
            bindings,
            responders,
            modifiers: Modifiers::NONE,
            held_play_keys: Vec::with_capacity(4),
        }
    }

    pub(super) fn bindings(&self) -> &[BindingDescription] {
        &self.bindings
    }

    pub(super) fn shortcuts(&self) -> &[ShortcutRow] {
        &self.shortcuts
    }

    pub(super) fn play_button(&self) -> Invoked {
        self.responders
            .invoke(WINDOW, PLAY, Source::Control)
            .expect("play button registered")
    }

    /// Recognized keys are owned by the command router before PXUI sees the event.
    pub(super) fn route(&mut self, event: &Event, invoke: &mut dyn FnMut(Invoked)) -> bool {
        let key = match *event {
            Event::FocusLost => {
                self.modifiers = Modifiers::NONE;
                self.held_play_keys.clear();
                return false;
            }
            Event::KeyReleased(key) => {
                if let Some(flag) = modifier(key) {
                    self.modifiers = self.modifiers.without(flag);
                }
                self.held_play_keys.retain(|&held| held != key);
                return false;
            }
            Event::KeyPressed(key) => key,
            _ => return false,
        };
        if let Some(flag) = modifier(key) {
            self.modifiers = self.modifiers.union(flag);
            return false;
        }
        let Some(chord) = shortcut_key(key) else {
            return false;
        };
        let Some(action) =
            self.responders
                .resolve(WINDOW, Shortcut::new(chord, self.modifiers), &VIEW)
        else {
            return false;
        };
        if action.id == PLAY {
            if self.held_play_keys.contains(&key) {
                return true;
            }
            self.held_play_keys.push(key);
        }
        invoke(action);
        true
    }
}

fn load_keymap(path: &Path, responders: &mut Responders) -> Result<String, KeymapError> {
    let text = fs::read_to_string(path).map_err(KeymapError::Io)?;
    responders.set_keymap(Keymap::parse(&text, &VIEW)?);
    Ok(text)
}

fn describe_bindings(responders: &Responders, custom: &str) -> Vec<BindingDescription> {
    let mut descriptions = Vec::with_capacity(CATALOG.len());
    for (command, label) in &CATALOG {
        let owns = |key| {
            responders
                .resolve(WINDOW, key, &VIEW)
                .is_some_and(|action| action.id == command.id)
        };
        let mut keys = String::new();
        if command.id == SHORTCUTS && !command.keys.iter().copied().all(owns) {
            // Keep host spellings distinct only if the keymap routes them to different commands.
            for (key, label) in command.keys.iter().copied().zip(["?", "⇧?", "⇧/"]) {
                if owns(key) {
                    append_label(&mut keys, label);
                }
            }
        } else if matches!(command.id, DELETE | DELETE_GAP) {
            for (key, label) in command
                .keys
                .iter()
                .copied()
                .zip(label.split(CHORD_SEPARATOR))
            {
                if owns(key) {
                    append_label(&mut keys, label);
                }
            }
        } else if command.keys.iter().copied().any(owns) {
            keys.push_str(label);
        }
        descriptions.push(BindingDescription {
            id: command.id,
            group: command.group,
            title: command.title,
            keys,
        });
    }
    // The kit already validated the file and installed these bindings. Retain only their labels.
    for line in custom
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
    {
        let Some((chord, name)) = line.split_once('=') else {
            continue;
        };
        let Some((description, _)) = descriptions
            .iter_mut()
            .zip(&CATALOG)
            .find(|(_, (command, _))| command.name == name.trim())
        else {
            continue;
        };
        let label = keymap_label(chord);
        if description.id == SHORTCUTS
            && description.keys == "?"
            && matches!(label.as_str(), "⇧?" | "⇧/")
        {
            continue;
        }
        append_label(&mut description.keys, &label);
    }
    descriptions.retain(|description| !description.keys.is_empty());
    descriptions
}

fn append_label(keys: &mut String, label: &str) {
    if !keys.split(CHORD_SEPARATOR).any(|key| key == label) {
        if !keys.is_empty() {
            keys.push_str(CHORD_SEPARATOR);
        }
        keys.push_str(label);
    }
}

fn keymap_label(chord: &str) -> String {
    let (modifiers, key) = chord.rsplit_once('+').unwrap_or(("", chord));
    let mut label = String::new();
    for (name, glyph) in [("Ctrl", '⌃'), ("Alt", '⌥'), ("Shift", '⇧'), ("Cmd", '⌘')] {
        if modifiers.split('+').any(|modifier| modifier.trim() == name) {
            label.push(glyph);
        }
    }
    match key.trim() {
        "Left" => label.push('←'),
        "Right" => label.push('→'),
        "Up" => label.push('↑'),
        "Down" => label.push('↓'),
        "Backspace" => label.push('⌫'),
        "Delete" => label.push('⌦'),
        "Enter" => label.push('↩'),
        "Escape" => label.push_str("Esc"),
        "Plus" => label.push('+'),
        "Equals" => label.push('='),
        "Hash" => label.push('#'),
        key if key.len() == 1 => label.push(char::from(key.as_bytes()[0].to_ascii_uppercase())),
        key => label.push_str(key),
    }
    label
}

pub(super) fn states(
    transport: bool,
    playing: bool,
    history: (bool, bool),
) -> [CommandState; VIEW.len()] {
    let mut states = [CommandState::default(); VIEW.len()];
    validate(&VIEW, &mut states, &mut |id, state| match id {
        PLAY => {
            state.enabled = transport;
            state.checked = playing;
            state.title = Some(if playing { "Pause" } else { "Play" });
        }
        BACK | FORWARD | START | END | BACK_SECOND | FORWARD_SECOND | SHUTTLE_REVERSE | STOP
        | SHUTTLE_FORWARD | PREVIOUS_EDIT | NEXT_EDIT => state.enabled = transport,
        ADD_TITLE | ADD_MARKER | TRIM_START | TRIM_END | DELETE | DELETE_GAP | DUPLICATE | CUT
        | COPY | PASTE | PASTE_EFFECTS => {
            state.enabled = false;
        }
        id if SELECTION_SHORTCUTS.contains(&id) => state.enabled = false,
        UNDO => state.enabled = history.0,
        REDO => state.enabled = history.1,
        PALETTE | SHORTCUTS | SELECT_TOOL | BLADE_TOOL | SNAP | SPLIT | CROP | ZOOM_IN
        | ZOOM_OUT | FIT | SESSION_UNDO | ADD_MOTION | REPLACE_SPINE => state.enabled = true,
        _ => unreachable!("viewer command"),
    });
    states
}

pub(super) fn edit_enabled(
    id: CommandId,
    project: &Project,
    selected: &[TimelineId],
    component_delete: Option<bool>,
    at: i32,
) -> bool {
    let spine_end = || {
        project
            .clips
            .order()
            .last()
            .and_then(|&slot| project.clips.timing(slot))
            .and_then(ClipTiming::end)
    };
    let selection_is_live = || {
        !selected.is_empty()
            && selected.iter().all(|target| match *target {
                TimelineId::Clip(clip) => project.clips.find(clip).is_some(),
                TimelineId::Item(item) => project
                    .items
                    .find(item)
                    .and_then(|slot| project.items.data(slot))
                    .is_some_and(|data| {
                        id != DUPLICATE
                            || !matches!(data.kind, ItemKind::Caption { text: None, .. })
                    }),
                TimelineId::Marker(marker) => {
                    project.markers.iter().any(|entry| entry.id == marker)
                }
            })
    };
    match id {
        ADD_TITLE => at >= 0 && spine_end().is_none_or(|end| end.saturating_sub(at) >= 3),
        ADD_MARKER => {
            at >= 0
                && spine_end().is_none_or(|end| at <= end)
                && project.markers.iter().all(|marker| marker.at != at)
        }
        TRIM_START | TRIM_END => trim_target(project, at).is_some(),
        DELETE | DELETE_GAP => component_delete.unwrap_or_else(selection_is_live),
        DUPLICATE => component_delete.is_none() && selection_is_live(),
        _ => unreachable!("editing shortcut"),
    }
}

pub(super) fn trim_target(project: &Project, at: i32) -> Option<(ClipId, ClipTiming)> {
    let id = clip_at(project, at)?;
    let timing = project.clips.timing(project.clips.find(id)?)?;
    (at - timing.start >= 3 && timing.end()? - at >= 3).then_some((id, timing))
}

pub(super) fn clip_at(project: &Project, at: i32) -> Option<ClipId> {
    let order = project.clips.order();
    let index = order.partition_point(|&slot| {
        project
            .clips
            .timing(slot)
            .is_some_and(|clip| clip.start.saturating_add(clip.length) <= at)
    });
    let slot = *order.get(index)?;
    (project.clips.timing(slot)?.start <= at)
        .then(|| project.clips.id(slot))
        .flatten()
}

fn modifier(key: HostKey) -> Option<Modifiers> {
    Some(match key {
        HostKey::Meta => Modifiers::CMD,
        HostKey::Ctrl => Modifiers::CTRL,
        HostKey::Alt => Modifiers::ALT,
        HostKey::Shift => Modifiers::SHIFT,
        _ => return None,
    })
}

fn shortcut_key(key: HostKey) -> Option<Key> {
    Some(match key {
        HostKey::KeyChar(ch) => Key::Char(ch),
        HostKey::ArrowUp => Key::Up,
        HostKey::ArrowDown => Key::Down,
        HostKey::ArrowLeft => Key::Left,
        HostKey::ArrowRight => Key::Right,
        HostKey::Space => Key::Space,
        HostKey::Enter => Key::Enter,
        HostKey::Escape => Key::Escape,
        HostKey::Backspace => Key::Backspace,
        HostKey::Tab => Key::Tab,
        HostKey::F1 => Key::Function(1),
        HostKey::F2 => Key::Function(2),
        HostKey::F3 => Key::Function(3),
        HostKey::F4 => Key::Function(4),
        HostKey::F5 => Key::Function(5),
        HostKey::F6 => Key::Function(6),
        HostKey::F7 => Key::Function(7),
        HostKey::F8 => Key::Function(8),
        HostKey::F9 => Key::Function(9),
        HostKey::F10 => Key::Function(10),
        HostKey::F11 => Key::Function(11),
        HostKey::F12 => Key::Function(12),
        HostKey::Home => Key::Home,
        HostKey::End => Key::End,
        HostKey::PageUp => Key::PageUp,
        HostKey::PageDown => Key::PageDown,
        HostKey::Insert => Key::Insert,
        HostKey::Delete => Key::Delete,
        _ => return None,
    })
}

pub(super) struct History {
    ring: UndoStack,
    seen: usize,
}

impl History {
    pub(super) fn new(store: &Store) -> Self {
        let origin = store
            .records()
            .first()
            .map_or(store.project().rev, |record| record.rev - 1);
        let mut history = Self {
            ring: UndoStack::new(origin, 128, 64 * 1024),
            seen: 0,
        };
        history.sync(store);
        history
    }

    pub(super) fn sync(&mut self, store: &Store) {
        for record in &store.records()[self.seen..] {
            let (merge, label) = match record.batch.change {
                Change::Undo(_) => {
                    self.ring.undo();
                    (Merge::Repair, Cow::Borrowed(""))
                }
                Change::Redo(_) => {
                    self.ring.redo();
                    (Merge::Repair, Cow::Borrowed(""))
                }
                _ => (Merge::Step, Cow::Owned(record.batch.label.clone())),
            };
            self.ring.push(Entry {
                label,
                merge,
                time_ms: record.batch.time,
                payload: record.rev,
            });
        }
        self.seen = store.records().len();
    }

    pub(super) fn invoke(
        &mut self,
        store: &mut Store,
        id: CommandId,
        now: u64,
    ) -> Result<bool, StoreError> {
        let (undo, redo) = self.ring.titles();
        let (title, result) = if id == UNDO {
            (undo, store.human_undo(now))
        } else {
            (redo, store.human_redo(now))
        };
        let result = result.map_err(|mut error| {
            if let Some(title) = title {
                error.message = format!("{title}: {}", error.message);
            }
            error
        })?;
        if let Some(warning) = result.warning {
            eprintln!("splice: {warning}");
        }
        self.sync(store);
        Ok(result.restored.is_some())
    }
}

#[cfg(test)]
mod tests;
