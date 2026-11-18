//! PXUI-shaped app kit. Product code depends only on std so it can move into PXUI.
#![deny(missing_docs)]

mod commands;
mod keymap;
mod undo;

pub use commands::{Command, CommandId, CommandState, Invoked, Responders, Source, validate};
pub use keymap::{Key, Keymap, KeymapError, Modifiers, Shortcut};
pub use undo::{Entry, Merge, UndoStack};

#[cfg(test)]
mod tests;
