use input::{Event, Key as HostKey};
use splice_core::{
    history::{Store, StoreError},
    package::journal::Change,
};
use splice_kit::{
    Command, CommandId, CommandState, Entry, Invoked, Key, Keymap, KeymapError, Merge, Modifiers,
    Responders, Shortcut, Source, UndoStack, validate,
};
use std::{borrow::Cow, io, path::Path};

pub(super) const PLAY: CommandId = CommandId(0);
pub(super) const BACK: CommandId = CommandId(1);
pub(super) const FORWARD: CommandId = CommandId(2);
pub(super) const UNDO: CommandId = CommandId(3);
pub(super) const REDO: CommandId = CommandId(4);

const WINDOW: u64 = 1;
const IDS: &[CommandId] = &[PLAY, BACK, FORWARD, UNDO, REDO];
const VIEW: [Command; 5] = [
    Command {
        id: PLAY,
        name: "transport.play",
        title: "Play",
        keys: &[Shortcut::new(Key::Space, Modifiers::NONE)],
        group: "Transport",
    },
    Command {
        id: BACK,
        name: "transport.stepBack",
        title: "Previous Frame",
        keys: &[Shortcut::new(Key::Left, Modifiers::NONE)],
        group: "Transport",
    },
    Command {
        id: FORWARD,
        name: "transport.step",
        title: "Next Frame",
        keys: &[Shortcut::new(Key::Right, Modifiers::NONE)],
        group: "Transport",
    },
    Command {
        id: UNDO,
        name: "history.undo",
        title: "Undo",
        keys: &[Shortcut::new(Key::Char('z'), Modifiers::CMD)],
        group: "History",
    },
    Command {
        id: REDO,
        name: "history.redo",
        title: "Redo",
        keys: &[Shortcut::new(
            Key::Char('z'),
            Modifiers::CMD.union(Modifiers::SHIFT),
        )],
        group: "History",
    },
];

pub(super) struct Keys {
    responders: Responders,
    modifiers: Modifiers,
    held_play_keys: Vec<HostKey>,
}

impl Keys {
    pub(super) fn new(keymap: Option<&Path>) -> Self {
        let mut responders = Responders::new(1);
        responders.responds(WINDOW, 0, IDS);
        if let Some(path) = keymap {
            let table: Vec<Command> =
                VIEW.into_iter()
                    .chain(splice_core::commands::COMMANDS.iter().enumerate().map(
                        |(index, row)| Command {
                            id: CommandId((VIEW.len() + index) as u32),
                            name: row.id,
                            title: row.title,
                            keys: &[],
                            group: row.group,
                        },
                    ))
                    .collect();
            match Keymap::read(path, &table) {
                Ok(keymap) => responders.set_keymap(keymap),
                Err(KeymapError::Io(error)) if error.kind() == io::ErrorKind::NotFound => (),
                Err(error) => eprintln!("splice: {}: {error}", path.display()),
            }
        }
        Self {
            responders,
            modifiers: Modifiers::NONE,
            held_play_keys: Vec::with_capacity(4),
        }
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
        BACK | FORWARD => state.enabled = transport,
        UNDO => state.enabled = history.0,
        REDO => state.enabled = history.1,
        _ => unreachable!("viewer command"),
    });
    states
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
