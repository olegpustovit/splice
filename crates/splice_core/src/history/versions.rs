use super::*;

#[derive(Clone)]
pub(super) struct Version {
    pub project: Project,
    pub active: Vec<u64>,
}

pub(super) struct Versions {
    // ponytail: keep materialized revisions while the project is open. Index and
    // evict old states when large project history exceeds the open-time memory budget.
    all: Vec<Version>,
    pub undo: Vec<u64>,
    pub redo: Vec<u64>,
    pub serial_floor: u64,
}

impl Versions {
    pub fn new(project: Project) -> Self {
        let serial_floor = project.serial;
        Self {
            all: vec![Version {
                project,
                active: Vec::new(),
            }],
            undo: Vec::new(),
            redo: Vec::new(),
            serial_floor,
        }
    }

    pub fn origin(&self) -> &Project {
        &self.all[0].project
    }

    pub fn tip(&self) -> &Version {
        self.all.last().expect("origin")
    }

    pub fn read(&self, rev: u64) -> Result<&Version, StoreError> {
        rev.checked_sub(self.origin().rev)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| self.all.get(index))
            .ok_or_else(|| {
                StoreError::new(
                    "NOT_FOUND",
                    &format!("No revision r{rev}."),
                    "Read history.list and use a retained revision.",
                )
            })
    }

    pub fn edit(&self, project: Project, rev: u64) -> Version {
        let mut active = self.tip().active.clone();
        active.push(rev);
        Version { project, active }
    }

    pub fn restore(&self, action: &Change) -> Result<Version, StoreError> {
        let target = match *action {
            Change::Undo(rev) if self.undo.last() == Some(&rev) => rev,
            Change::Redo(rev) if self.redo.last() == Some(&rev) => rev,
            Change::Restore(rev) => rev,
            _ => {
                return Err(StoreError::new(
                    "BAD_PACKAGE",
                    "History action does not match the undo or redo stack.",
                    "Recover a verified history prefix.",
                ));
            }
        };
        Ok(self.read(target)?.clone())
    }

    pub fn push(&mut self, version: Version, change: &Change) {
        let previous = self.tip().project.rev;
        match change {
            Change::Undo(_) => {
                self.undo.pop();
                self.redo.push(previous);
            }
            Change::Redo(_) => {
                self.redo.pop();
                self.undo.push(previous);
            }
            _ => {
                self.undo.push(previous);
                self.redo.clear();
            }
        }
        self.serial_floor = self.serial_floor.max(version.project.serial);
        self.all.push(version);
    }

    pub fn replay(&mut self, records: &[Record]) -> Result<Project, StoreError> {
        let record = records.last().expect("record to replay");
        let action = &record.batch.change;
        let mut next = match action {
            Change::Edit { ops, mints } => {
                let (candidate, _) =
                    edit::stage(&self.tip().project, ops, record.batch.source, Some(mints))?;
                self.edit(candidate, record.rev)
            }
            Change::Revert { revisions, skipped } => {
                let rebuilt = self.rebuild(records, revisions)?;
                if encode_skipped(&rebuilt.skipped) != *skipped {
                    return Err(StoreError::new(
                        "JOURNAL_CORRUPT",
                        "The recorded skipped edits differ from replay.",
                        "Restore the journal from a verified backup.",
                    ));
                }
                rebuilt.version
            }
            _ => self.restore(action)?,
        };
        let access = self.tip().project.access;
        if record.batch.source == Source::Agent
            && (access != Access::Live || next.project.access != access)
        {
            return Err(StoreError::new(
                "JOURNAL_CORRUPT",
                "A recorded agent action violated project access.",
                "Recover a verified history prefix.",
            ));
        }
        next.project.rev = record.rev;
        next.project.hash = record.hash;
        let project = next.project.clone();
        self.push(next, action);
        Ok(project)
    }

    pub fn rebuild(&self, records: &[Record], removed: &[u64]) -> Result<Rebuilt, StoreError> {
        let current = &self.tip().active;
        let first = removed.iter().copied().min().ok_or_else(invalid_revert)?;
        if removed.iter().any(|rev| !current.contains(rev)) {
            return Err(invalid_revert());
        }
        let mut project = self.origin().clone();
        let mut active = Vec::with_capacity(current.len().saturating_sub(removed.len()));
        let mut skipped = Vec::new();
        let mut replayed = 0;
        for &rev in current {
            if removed.contains(&rev) {
                continue;
            }
            let record = at(records, self.origin().rev, rev).ok_or_else(invalid_revert)?;
            let Change::Edit { ops, mints } = &record.batch.change else {
                return Err(invalid_revert());
            };
            match edit::stage(&project, ops, record.batch.source, Some(mints)) {
                Ok((next, _)) => {
                    project = next;
                    active.push(rev);
                    replayed += usize::from(rev > first);
                }
                Err(error) if rev > first => skipped.push(Skipped {
                    revision: rev,
                    title: record.batch.label.clone(),
                    error: error.error.code.into(),
                    message: error.error.message,
                }),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(Rebuilt {
            version: Version { project, active },
            skipped,
            replayed,
        })
    }
}

pub(super) struct Rebuilt {
    pub version: Version,
    pub skipped: Vec<Skipped>,
    pub replayed: usize,
}

pub(super) fn at(records: &[Record], origin: u64, rev: u64) -> Option<&Record> {
    let index = rev.checked_sub(origin)?.checked_sub(1)?;
    records
        .get(usize::try_from(index).ok()?)
        .filter(|record| record.rev == rev)
}

pub(super) fn encode_skipped(skipped: &[Skipped]) -> Vec<Json> {
    skipped
        .iter()
        .map(|edit| {
            crate::package::codec::object(vec![
                ("revision", Json::Unsigned(edit.revision)),
                ("title", Json::String(edit.title.clone())),
                ("error", Json::String(edit.error.clone())),
                ("message", Json::String(edit.message.clone())),
            ])
        })
        .collect()
}

fn invalid_revert() -> StoreError {
    StoreError::new(
        "JOURNAL_CORRUPT",
        "A selective undo refers to an inactive or unknown edit.",
        "Recover a verified history prefix.",
    )
}
