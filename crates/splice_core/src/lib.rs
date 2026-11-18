//! Single writer: command registry, apply(ops) -> changes, revisions, journal, undo/history, sessions, replay, friction log.
#![deny(missing_docs)]

/// On-disk Splice packages and the single-writer lock.
pub mod package;
#[cfg(test)]
mod package_tests;
