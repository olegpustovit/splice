use crate::edit;
use crate::package::{
    self, PackageError,
    journal::{Batch, Change, Checkpoint, Journal, Record},
};
use splice_json::Value as Json;
use splice_model::{Access, Project, SessionId};
use std::path::Path;

mod result;
mod sessions;
mod versions;

pub use crate::package::journal::Source;
pub use result::*;
pub use sessions::Session;
use versions::{Version, Versions};

#[cfg(test)]
mod tests;

/// The durable single writer for a project opened through CLI, MCP, or the viewer.
pub struct Store {
    journal: Journal,
    versions: Versions,
    sessions: Vec<Session>,
}

impl Store {
    /// Creates a project, its origin and its journal.
    pub fn create(path: &Path, project: &Project) -> Result<Self, StoreError> {
        let (journal, state) = Journal::create(path, project)?;
        Ok(Self {
            journal,
            versions: Versions::new(state),
            sessions: Vec::new(),
        })
    }

    /// Acquires the project lock and rebuilds retained revision history.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut versions = None;
        let (journal, state) = Journal::open_history(path, &mut |_, origin, prefix| {
            let history = versions.get_or_insert_with(|| Versions::new(origin.clone()));
            history
                .replay(prefix)
                .map_err(|error| PackageError::invalid(&error))
        })?;
        let sessions = sessions::read(journal.path())?;
        Ok(Self {
            journal,
            versions: versions.unwrap_or_else(|| Versions::new(state)),
            sessions,
        })
    }

    /// Current timeline state, including its revision and journal hash.
    pub fn project(&self) -> &Project {
        &self.versions.tip().project
    }

    /// An immutable retained revision.
    pub fn at(&self, rev: u64) -> Result<&Project, StoreError> {
        Ok(&self.versions.read(rev)?.project)
    }

    /// Journal records in chronological order.
    pub fn records(&self) -> &[Record] {
        self.journal.records()
    }

    /// Whether an edit revision still contributes to the active timeline.
    pub fn is_active(&self, revision: u64) -> bool {
        self.versions.tip().active.contains(&revision)
    }

    /// Open and closed sessions, in creation order.
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    /// Package directory for exports and other ancillary files.
    pub fn path(&self) -> &Path {
        self.journal.path()
    }

    /// Journal problem that prevents editing, when present.
    pub fn problem(&self) -> Option<&PackageError> {
        self.journal.problem()
    }

    /// Saves the current revision as a checkpoint.
    pub fn checkpoint(&mut self) -> Result<(), StoreError> {
        self.journal
            .checkpoint(&self.versions.tip().project)
            .map_err(Into::into)
    }

    /// Saves the current revision and releases the package writer lock.
    pub fn close(self) -> Result<(), StoreError> {
        self.journal
            .close(&self.versions.tip().project)
            .map_err(Into::into)
    }

    /// Starts a session at the current revision without creating a timeline revision.
    pub fn start_session(&mut self, agent: &str, time: u64) -> Result<Session, StoreError> {
        self.writable()?;
        self.agent_access()?;
        let last = self
            .sessions
            .iter()
            .map(|session| session.id.serial())
            .chain(
                self.records()
                    .iter()
                    .filter_map(|record| record.batch.session.map(SessionId::serial)),
            )
            .max()
            .unwrap_or(0);
        let serial = last.checked_add(1).ok_or_else(|| {
            StoreError::new(
                "OUT_OF_RANGE",
                "Session IDs are exhausted.",
                "Open another project.",
            )
        })?;
        let session = Session {
            id: SessionId::from_serial(serial),
            agent: agent.into(),
            checkpoint: self.project().rev,
            started: time,
            open: true,
        };
        let mut next = self.sessions.clone();
        next.push(session.clone());
        sessions::write(self.journal.path(), &next)?;
        self.sessions = next;
        Ok(session)
    }

    /// Disables additional writes under a session ID while retaining its history.
    pub fn end_session(&mut self, id: SessionId) -> Result<(), StoreError> {
        self.writable()?;
        let mut next = self.sessions.clone();
        let session = next
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or_else(|| unknown_session(id))?;
        session.open = false;
        sessions::write(self.journal.path(), &next)?;
        self.sessions = next;
        Ok(())
    }

    /// Checks whether an agent may start an operation against the current project revision.
    pub fn check_write(&self, options: &WriteOptions) -> Result<(), StoreError> {
        self.authorize(options)?;
        self.writable()?;
        self.guard_revision(options)
    }

    /// Previews or durably commits one batch, with stale-write and idempotency checks.
    pub fn apply(
        &mut self,
        ops: &[Json],
        options: &WriteOptions,
    ) -> Result<EditResult, StoreError> {
        self.authorize(options)?;
        if !options.dry_run {
            self.writable()?;
            if let Some(receipt) = self.prior_receipt(ops, options)? {
                return Ok(receipt);
            }
        }
        self.guard_revision(options)?;
        if ops.is_empty() {
            return Err(StoreError::new(
                "BAD_ARGS",
                "The edit batch is empty.",
                "Pass a nonempty ops array.",
            ));
        }
        let (candidate, report) = edit::stage_above(
            self.project(),
            ops,
            options.source,
            None,
            self.versions.serial_floor,
        )?;
        if options.dry_run {
            return Ok(EditResult {
                revision: self.project().rev,
                hash: self.project().hash,
                dry_run: true,
                replayed: false,
                report,
                warning: None,
            });
        }
        let rev = self.project().rev.checked_add(1).ok_or_else(|| {
            StoreError::new(
                "OUT_OF_RANGE",
                "Revisions are exhausted.",
                "Open another project.",
            )
        })?;
        let label = options.label.clone().unwrap_or_else(|| {
            let joined = report
                .changes
                .iter()
                .map(|change| change.summary.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            if options.source == Source::Agent {
                format!("Agent: {joined}")
            } else {
                joined
            }
        });
        let next = self.versions.edit(candidate, rev);
        let warning = self.commit(
            next,
            options,
            label,
            Change::Edit {
                ops: ops.to_vec(),
                mints: report.mints.clone(),
            },
        )?;
        Ok(EditResult {
            revision: self.project().rev,
            hash: self.project().hash,
            dry_run: false,
            replayed: false,
            report,
            warning,
        })
    }

    /// Appends a session revert or a restore to a retained revision.
    pub fn undo(
        &mut self,
        scope: Scope,
        options: &WriteOptions,
    ) -> Result<HistoryResult, StoreError> {
        self.authorize(options)?;
        self.guard_revision(options)?;
        if let Scope::To(revision) = scope {
            return self.restore(Change::Restore(revision), options);
        }
        let session = options.session.ok_or_else(no_session)?;
        self.sessions
            .iter()
            .find(|candidate| candidate.id == session)
            .ok_or_else(|| unknown_session(session))?;
        let mut revisions: Vec<_> = self
            .versions
            .tip()
            .active
            .iter()
            .copied()
            .filter(|&rev| {
                versions::at(self.records(), self.versions.origin().rev, rev).is_some_and(
                    |record| {
                        record.batch.session == Some(session)
                            && record.batch.source == Source::Agent
                    },
                )
            })
            .collect();
        if scope == Scope::Last
            && let Some(last) = revisions.pop()
        {
            revisions.clear();
            revisions.push(last);
        }
        if revisions.is_empty() {
            return Ok(self.history_result(None, None));
        }
        let rebuilt = self.versions.rebuild(self.records(), &revisions)?;
        self.preserve_agent_access(&rebuilt.version, options.source)?;
        if options.dry_run {
            let mut preview = self.history_result(None, None);
            preview.reverted = revisions;
            preview.skipped = rebuilt.skipped;
            preview.replayed = rebuilt.replayed;
            return Ok(preview);
        }
        let label = if scope == Scope::Last {
            format!("Undo agent revision r{}", revisions[0])
        } else {
            format!("Undo agent session {session}")
        };
        let warning = self.commit(
            rebuilt.version,
            options,
            label,
            Change::Revert {
                revisions: revisions.clone(),
                skipped: versions::encode_skipped(&rebuilt.skipped),
            },
        )?;
        let mut result = self.history_result(None, warning);
        result.reverted = revisions;
        result.skipped = rebuilt.skipped;
        result.replayed = rebuilt.replayed;
        Ok(result)
    }

    /// Human undo of the most recent undoable batch, including a revert or restore.
    pub fn human_undo(&mut self, time: u64) -> Result<HistoryResult, StoreError> {
        if let Some(&previous) = self.versions.undo.last() {
            return self.restore(
                Change::Undo(previous),
                &WriteOptions {
                    time,
                    ..WriteOptions::default()
                },
            );
        }
        Ok(self.history_result(None, None))
    }

    /// Human redo of the most recently undone batch.
    pub fn human_redo(&mut self, time: u64) -> Result<HistoryResult, StoreError> {
        if let Some(&next) = self.versions.redo.last() {
            return self.restore(
                Change::Redo(next),
                &WriteOptions {
                    time,
                    ..WriteOptions::default()
                },
            );
        }
        Ok(self.history_result(None, None))
    }

    fn restore(
        &mut self,
        action: Change,
        options: &WriteOptions,
    ) -> Result<HistoryResult, StoreError> {
        let (revision, verb) = match action {
            Change::Undo(rev) => (rev, "Undo to"),
            Change::Redo(rev) => (rev, "Redo to"),
            Change::Restore(rev) => (rev, "Restore"),
            _ => unreachable!("restore action"),
        };
        let next = self.versions.restore(&action)?;
        self.preserve_agent_access(&next, options.source)?;
        if options.dry_run {
            return Ok(self.history_result(Some(revision), None));
        }
        let warning = self.commit(next, options, format!("{verb} r{revision}"), action)?;
        Ok(self.history_result(Some(revision), warning))
    }

    fn history_result(
        &self,
        restored: Option<u64>,
        warning: Option<PackageError>,
    ) -> HistoryResult {
        HistoryResult {
            revision: self.project().rev,
            hash: self.project().hash,
            reverted: Vec::new(),
            restored,
            replayed: 0,
            skipped: Vec::new(),
            warning,
        }
    }

    fn commit(
        &mut self,
        mut next: Version,
        options: &WriteOptions,
        label: String,
        change: Change,
    ) -> Result<Option<PackageError>, StoreError> {
        self.writable()?;
        next.project.rev = self.project().rev;
        next.project.hash = self.project().hash;
        let key = if matches!(change, Change::Edit { .. }) {
            options.key.clone()
        } else {
            None
        };
        let status = self.journal.commit(
            &mut next.project,
            Batch {
                time: options.time,
                source: options.source,
                session: options.session,
                label,
                key,
                change,
            },
        )?;
        let last = self.journal.records().last().expect("committed record");
        self.versions.push(next, &last.batch.change);
        Ok(match status {
            Checkpoint::Failed(error) => Some(error),
            Checkpoint::Written | Checkpoint::Deferred => None,
        })
    }

    fn prior_receipt(
        &self,
        ops: &[Json],
        options: &WriteOptions,
    ) -> Result<Option<EditResult>, StoreError> {
        let Some(key) = &options.key else {
            return Ok(None);
        };
        if key.is_empty() {
            return Err(StoreError::new(
                "BAD_ARGS",
                "The idempotency key is empty.",
                "Pass a nonempty key or omit it.",
            ));
        }
        let Some(record) = self.records().iter().find(|record| {
            record.batch.key.as_ref() == Some(key)
                && record.batch.session == options.session
                && record.batch.source == options.source
        }) else {
            return Ok(None);
        };
        let Change::Edit {
            ops: original,
            mints,
        } = &record.batch.change
        else {
            return Err(StoreError::new(
                "JOURNAL_CORRUPT",
                "An idempotency key references a history action.",
                "Recover a verified project history.",
            ));
        };
        if original != ops
            && package::json_bytes(&Json::Array(original.clone()))?
                != package::json_bytes(&Json::Array(ops.to_vec()))?
        {
            return Err(StoreError::new(
                "CONFLICT",
                "This idempotency key was committed with different operations.",
                "Retry the original operations or use a new key.",
            ));
        }
        let previous = self.at(record.rev - 1)?;
        let (_, report) = edit::stage(previous, original, record.batch.source, Some(mints))?;
        Ok(Some(EditResult {
            revision: record.rev,
            hash: record.hash,
            dry_run: false,
            replayed: true,
            report,
            warning: None,
        }))
    }

    fn authorize(&self, options: &WriteOptions) -> Result<(), StoreError> {
        if options.source == Source::Agent {
            self.agent_access()?;
            let id = options.session.ok_or_else(no_session)?;
            let session = self
                .sessions
                .iter()
                .find(|session| session.id == id)
                .ok_or_else(|| unknown_session(id))?;
            if !session.open {
                return Err(no_session());
            }
        }
        Ok(())
    }

    fn guard_revision(&self, options: &WriteOptions) -> Result<(), StoreError> {
        let Some(expected) = options.if_revision else {
            if options.source == Source::Agent {
                return Err(StoreError::new(
                    "BAD_ARGS",
                    "Agent writes require ifRevision.",
                    "Pass the revision from the last project read.",
                ));
            }
            return Ok(());
        };
        let current = self.project().rev;
        if expected == current {
            return Ok(());
        }
        let mut error = StoreError::new(
            "STALE_REVISION",
            &format!("ifRevision r{expected} is stale. Current revision is r{current}."),
            "Re-read the project and rebase the edit.",
        );
        let mut changed_since: Vec<_> = self
            .records()
            .iter()
            .rev()
            .take_while(|record| record.rev > expected)
            .take(8)
            .map(|record| RevisionSummary {
                revision: record.rev,
                source: record.batch.source,
                label: record.batch.label.clone(),
            })
            .collect();
        changed_since.reverse();
        error.stale = Some(Box::new(StaleRevision {
            current,
            changed_since,
        }));
        Err(error)
    }

    fn writable(&self) -> Result<(), StoreError> {
        if let Some(error) = self.problem() {
            return Err(error.clone().into());
        }
        Ok(())
    }

    fn agent_access(&self) -> Result<(), StoreError> {
        if self.project().access == Access::Live {
            Ok(())
        } else {
            Err(StoreError::new(
                "PERMISSION_DENIED",
                "The project does not allow live agent edits.",
                "Ask the human editor to enable live access.",
            ))
        }
    }

    fn preserve_agent_access(&self, next: &Version, source: Source) -> Result<(), StoreError> {
        if source == Source::Agent && next.project.access != self.project().access {
            return Err(StoreError::new(
                "PERMISSION_DENIED",
                "This restore would change project access.",
                "Ask the human editor to restore that revision.",
            ));
        }
        Ok(())
    }
}

/// Hashes the canonical project content, including its serial and media, without the journal stamp.
pub fn state_hash(project: &Project) -> Result<u64, PackageError> {
    let mut value = package::journal::encode_state(project);
    let Json::Object(fields) = &mut value else {
        unreachable!("encoded state")
    };
    let Json::Object(snapshot) = &mut fields[0].1 else {
        unreachable!("encoded snapshot")
    };
    snapshot.retain(|(name, _)| name != "rev" && name != "hash");
    Ok(package::hash_bytes(&package::json_bytes(&value)?))
}

fn no_session() -> StoreError {
    StoreError::new(
        "NO_SESSION",
        "An open session is required.",
        "Call session.start and pass its sessionId.",
    )
}

fn unknown_session(id: SessionId) -> StoreError {
    StoreError::new(
        "NOT_FOUND",
        &format!("No session {id}."),
        "Call session.start to obtain a session ID for this project.",
    )
}
