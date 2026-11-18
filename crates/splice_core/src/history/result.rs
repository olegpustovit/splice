use crate::commands::CommandError;
use crate::edit::{ApplyError, ApplyReport};
use crate::package::{PackageError, journal::Source};
use splice_model::SessionId;
use std::fmt;

/// Inputs shared by a write and its preview.
#[derive(Clone, Debug)]
pub struct WriteOptions {
    /// The actor making the change.
    pub source: Source,
    /// Required for agent writes.
    pub session: Option<SessionId>,
    /// Revision from the caller's last read. Required for agent writes.
    pub if_revision: Option<u64>,
    /// Preview without publishing or appending.
    pub dry_run: bool,
    /// Key for idempotent edit retries within the actor's session.
    pub key: Option<String>,
    /// Optional human-readable description.
    pub label: Option<String>,
    /// Unix time in milliseconds.
    pub time: u64,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            source: Source::Human,
            session: None,
            if_revision: None,
            dry_run: false,
            key: None,
            label: None,
            time: 0,
        }
    }
}

/// A committed edit, a retry receipt, or a dry-run preview.
#[derive(Debug)]
pub struct EditResult {
    /// Revision of the receipt.
    pub revision: u64,
    /// Journal hash at that revision.
    pub hash: u64,
    /// Whether the request ran without being committed.
    pub dry_run: bool,
    /// Whether this receipt was recovered from a prior commit with the same key.
    pub replayed: bool,
    /// The batch's changes and diff.
    pub report: ApplyReport,
    /// A periodic snapshot failure after a successful journal append.
    pub warning: Option<PackageError>,
}

/// Agent history undo scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Remove the session's latest live edit from the active history.
    Last,
    /// Remove all live edits from that session in one revision.
    Session,
    /// Restore the exact state at a revision.
    To(u64),
}

/// A retained edit that failed to replay after a selective undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// Revision of the original edit.
    pub revision: u64,
    /// Description of that edit.
    pub title: String,
    /// Typed replay error.
    pub error: String,
    /// Why this edit failed to replay.
    pub message: String,
}

/// History operation result.
#[derive(Debug)]
pub struct HistoryResult {
    /// Resulting revision, or the current revision for a no-op.
    pub revision: u64,
    /// Journal hash at the resulting revision.
    pub hash: u64,
    /// Edits removed by a selective undo.
    pub reverted: Vec<u64>,
    /// Revision restored by a history restore, human undo, or redo.
    pub restored: Option<u64>,
    /// Number of later edits replayed after a selective undo.
    pub replayed: usize,
    /// Later edits that could not be replayed.
    pub skipped: Vec<Skipped>,
    /// A periodic snapshot failure after a successful journal append.
    pub warning: Option<PackageError>,
}

/// Short revision summary for stale edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionSummary {
    /// Revision number.
    pub revision: u64,
    /// Actor who committed the revision.
    pub source: Source,
    /// Batch label.
    pub label: String,
}

/// Context returned with a stale write.
#[derive(Debug)]
pub struct StaleRevision {
    /// Current revision to use when rebasing.
    pub current: u64,
    /// Recent changes following the stale revision.
    pub changed_since: Vec<RevisionSummary>,
}

/// Typed store rejection with optional revision and operation context.
#[derive(Debug)]
pub struct StoreError {
    /// Stable error code.
    pub code: &'static str,
    /// Description.
    pub message: String,
    /// How to recover.
    pub fix: String,
    /// Failed operation index, if known.
    pub op: Option<usize>,
    /// Suggested command IDs for unknown commands.
    pub valid: Vec<&'static str>,
    /// Current revision and intervening changes for a stale write.
    pub stale: Option<Box<StaleRevision>>,
}

impl StoreError {
    /// Creates a typed failure with recovery guidance and no operation context.
    pub fn new(code: &'static str, message: &str, fix: &str) -> Self {
        CommandError::new(code, message.into(), fix.into()).into()
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} {}", self.code, self.message, self.fix)
    }
}

impl std::error::Error for StoreError {}

impl From<CommandError> for StoreError {
    fn from(error: CommandError) -> Self {
        Self {
            code: error.code,
            message: error.message,
            fix: error.fix,
            valid: error.valid,
            op: None,
            stale: None,
        }
    }
}

impl From<PackageError> for StoreError {
    fn from(error: PackageError) -> Self {
        Self::new(error.code, &error.message, error.fix)
    }
}

impl From<ApplyError> for StoreError {
    fn from(error: ApplyError) -> Self {
        let mut result = Self::from(error.error);
        result.op = error.op;
        result
    }
}
