use crate::{Command, CommandId};
use std::{fmt, fs, io, path::Path};

/// A character or named keyboard key from the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A Unicode character. ASCII letters are case-insensitive in shortcuts.
    Char(char),
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Space bar.
    Space,
    /// Return or Enter.
    Enter,
    /// Escape key.
    Escape,
    /// Backward delete.
    Backspace,
    /// Tab key.
    Tab,
    /// Function key, F1 through F12.
    Function(u8),
    /// Home key.
    Home,
    /// End key.
    End,
    /// Page Up key.
    PageUp,
    /// Page Down key.
    PageDown,
    /// Insert key.
    Insert,
    /// Forward delete.
    Delete,
}

/// The exact modifiers held for a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier.
    pub const NONE: Self = Self(0);
    /// Command or Super.
    pub const CMD: Self = Self(1);
    /// Shift.
    pub const SHIFT: Self = Self(2);
    /// Control.
    pub const CTRL: Self = Self(4);
    /// Option or Alt.
    pub const ALT: Self = Self(8);

    /// Combines two modifier sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Removes the given modifiers from this set.
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// A keyboard key and its exact modifier set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    key: Key,
    modifiers: Modifiers,
}

impl Shortcut {
    /// Creates a normalized shortcut from a host key and modifiers.
    pub const fn new(key: Key, modifiers: Modifiers) -> Self {
        let key = match key {
            Key::Char(' ') => Key::Space,
            Key::Char(ch) => Key::Char(ch.to_ascii_lowercase()),
            key => key,
        };
        Self { key, modifiers }
    }
}

/// An unreadable or invalid keymap file.
#[derive(Debug)]
pub enum KeymapError {
    /// The file could not be read as UTF-8.
    Io(io::Error),
    /// The binding does not contain a valid shortcut and command name.
    InvalidBinding {
        /// One-based line number.
        line: usize,
    },
    /// The named command is absent from the table.
    UnknownCommand {
        /// One-based line number.
        line: usize,
    },
    /// The same shortcut appears twice.
    Conflict {
        /// One-based line number of the duplicate.
        line: usize,
        /// One-based line number of the first binding.
        previous: usize,
    },
}

impl fmt::Display for KeymapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::InvalidBinding { line } => write!(
                f,
                "Keymap line {line} must be a valid shortcut = command binding"
            ),
            Self::UnknownCommand { line } => {
                write!(f, "Keymap line {line} names an unknown command")
            }
            Self::Conflict { line, previous } => write!(
                f,
                "Keymap line {line} repeats the shortcut on line {previous}"
            ),
        }
    }
}

struct Binding {
    shortcut: Shortcut,
    id: CommandId,
    line: usize,
}

/// User overrides of individual default shortcuts.
#[derive(Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Keymap {
    /// Reads a UTF-8 keymap file and validates its bindings against the table.
    pub fn read(path: &Path, table: &[Command]) -> Result<Self, KeymapError> {
        let text = fs::read_to_string(path).map_err(KeymapError::Io)?;
        Self::parse(&text, table)
    }

    /// Parses `key = command` lines, with blank lines and full-line `#` comments.
    pub fn parse(text: &str, table: &[Command]) -> Result<Self, KeymapError> {
        let mut map = Self::default();
        for (index, raw) in text.lines().enumerate() {
            let entry = raw.trim();
            if entry.is_empty() || entry.starts_with('#') {
                continue;
            }
            let line = index + 1;
            let invalid = || KeymapError::InvalidBinding { line };
            let (key, name) = entry.split_once('=').ok_or_else(invalid)?;
            let shortcut = parse_shortcut(key).ok_or_else(invalid)?;
            let name = name.trim();
            if name.is_empty() || name.contains('=') {
                return Err(invalid());
            }
            let command = table
                .iter()
                .find(|command| command.name == name)
                .ok_or(KeymapError::UnknownCommand { line })?;
            if let Some(previous) = map.bindings.iter().find(|entry| entry.shortcut == shortcut) {
                return Err(KeymapError::Conflict {
                    line,
                    previous: previous.line,
                });
            }
            map.bindings.push(Binding {
                shortcut,
                id: command.id,
                line,
            });
        }
        Ok(map)
    }

    pub(crate) fn command(&self, shortcut: Shortcut) -> Option<CommandId> {
        self.bindings
            .iter()
            .find(|entry| entry.shortcut == shortcut)
            .map(|entry| entry.id)
    }
}

fn parse_shortcut(text: &str) -> Option<Shortcut> {
    let mut parts = text.split('+').map(str::trim);
    let name = parts.next_back()?;
    let mut modifiers = Modifiers::NONE;
    for modifier in parts {
        let flag = match modifier {
            "Cmd" => Modifiers::CMD,
            "Shift" => Modifiers::SHIFT,
            "Ctrl" => Modifiers::CTRL,
            "Alt" => Modifiers::ALT,
            _ => return None,
        };
        let next = modifiers.union(flag);
        if next == modifiers {
            return None;
        }
        modifiers = next;
    }
    let key = match name {
        "Up" | "↑" => Key::Up,
        "Down" | "↓" => Key::Down,
        "Left" | "←" => Key::Left,
        "Right" | "→" => Key::Right,
        "Space" => Key::Space,
        "Enter" => Key::Enter,
        "Escape" => Key::Escape,
        "Backspace" => Key::Backspace,
        "Tab" => Key::Tab,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Insert" => Key::Insert,
        "Delete" => Key::Delete,
        "Plus" => Key::Char('+'),
        "Equals" => Key::Char('='),
        "Hash" => Key::Char('#'),
        _ => {
            let mut chars = name.chars();
            let ch = chars.next()?;
            if chars.next().is_none() && !ch.is_control() && !ch.is_whitespace() {
                Key::Char(ch)
            } else {
                let digits = name.strip_prefix('F')?;
                let function = digits.parse().ok()?;
                if digits.starts_with('0') || !(1..=12).contains(&function) {
                    return None;
                }
                Key::Function(function)
            }
        }
    };
    Some(Shortcut::new(key, modifiers))
}
