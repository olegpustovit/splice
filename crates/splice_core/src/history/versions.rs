use super::*;

#[derive(Clone)]
pub(super) struct Version {
    pub project: Project,
    pub active: Vec<u64>,
}

/// A retained revision: its active edits always, its project only when kept materialized.
struct Entry {
    project: Option<Project>,
    active: Vec<u64>,
    rev: u64,
    hash: u64,
}

/// Recent revisions stay materialized for fast undo.
const RECENT: usize = 32;
/// Every so many revisions stay materialized as rebuild bases.
const STRIDE: usize = 64;

pub(super) struct Versions {
    // Memory stays bounded over long sessions (T15.4): older revisions keep only their active
    // edit list and rebuild from the nearest kept base when read.
    all: Vec<Entry>,
    pub undo: Vec<u64>,
    pub redo: Vec<u64>,
    pub serial_floor: u64,
}

impl Versions {
    pub fn new(project: Project) -> Self {
        let serial_floor = project.serial;
        Self {
            all: vec![Entry {
                rev: project.rev,
                hash: project.hash,
                project: Some(project),
                active: Vec::new(),
            }],
            undo: Vec::new(),
            redo: Vec::new(),
            serial_floor,
        }
    }

    pub fn origin(&self) -> &Project {
        self.all[0]
            .project
            .as_ref()
            .expect("origin stays materialized")
    }

    pub fn tip(&self) -> Version {
        let entry = self.all.last().expect("origin");
        Version {
            project: entry.project.clone().expect("tip stays materialized"),
            active: entry.active.clone(),
        }
    }

    /// The tip's project (always materialized).
    pub fn tip_project(&self) -> &Project {
        self.all
            .last()
            .and_then(|entry| entry.project.as_ref())
            .expect("tip stays materialized")
    }

    /// Revisions kept materialized (bounded by RECENT plus one per STRIDE).
    #[cfg(test)]
    pub fn materialized(&self) -> usize {
        self.all
            .iter()
            .filter(|entry| entry.project.is_some())
            .count()
    }

    /// The tip's active edits.
    pub fn tip_active(&self) -> &[u64] {
        &self.all.last().expect("origin").active
    }

    fn index(&self, rev: u64) -> Result<usize, StoreError> {
        rev.checked_sub(self.all[0].rev)
            .and_then(|index| usize::try_from(index).ok())
            .filter(|&index| index < self.all.len())
            .ok_or_else(|| {
                StoreError::new(
                    "NOT_FOUND",
                    &format!("No revision r{rev}."),
                    "Read history.list and use a retained revision.",
                )
            })
    }

    /// A retained revision's project, borrowed when materialized, else rebuilt from `records`.
    pub fn read(
        &self,
        rev: u64,
        records: &[Record],
    ) -> Result<std::borrow::Cow<'_, Project>, StoreError> {
        let index = self.index(rev)?;
        let entry = &self.all[index];
        if let Some(project) = &entry.project {
            return Ok(std::borrow::Cow::Borrowed(project));
        }
        // The nearest kept revision whose active edits prefix this one's.
        let (base, done) = self.all[..index]
            .iter()
            .rev()
            .filter_map(|other| {
                let project = other.project.as_ref()?;
                entry
                    .active
                    .starts_with(&other.active)
                    .then_some((project, other.active.len()))
            })
            .next()
            .unwrap_or((self.origin(), 0));
        let mut project = base.clone();
        for &edit in &entry.active[done..] {
            let record = at(records, self.origin().rev, edit).ok_or_else(invalid_revert)?;
            let Change::Edit {
                ops,
                mints,
                imports,
            } = &record.batch.change
            else {
                return Err(invalid_revert());
            };
            project = edit::stage_recorded(
                &project,
                ops,
                record.batch.source,
                mints,
                imports,
                record.engine,
            )?
            .0;
        }
        project.rev = entry.rev;
        project.hash = entry.hash;
        Ok(std::borrow::Cow::Owned(project))
    }

    pub fn edit(&self, project: Project, rev: u64) -> Version {
        let mut active = self.tip_active().to_vec();
        active.push(rev);
        Version { project, active }
    }

    pub fn restore(&self, action: &Change, records: &[Record]) -> Result<Version, StoreError> {
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
        let index = self.index(target)?;
        Ok(Version {
            project: self.read(target, records)?.into_owned(),
            active: self.all[index].active.clone(),
        })
    }

    pub fn push(&mut self, version: Version, change: &Change) {
        let previous = self.tip_project().rev;
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
        self.all.push(Entry {
            rev: version.project.rev,
            hash: version.project.hash,
            project: Some(version.project),
            active: version.active,
        });
        // Leaving the recent window: keep every STRIDE-th revision as a rebuild base.
        if let Some(index) = self.all.len().checked_sub(RECENT + 1)
            && index % STRIDE != 0
        {
            self.all[index].project = None;
        }
    }

    pub fn replay(&mut self, records: &[Record]) -> Result<Project, StoreError> {
        let record = records.last().expect("record to replay");
        let action = &record.batch.change;
        let mut next = match action {
            Change::Edit {
                ops,
                mints,
                imports,
            } => {
                let (candidate, _) = edit::stage_recorded(
                    self.tip_project(),
                    ops,
                    record.batch.source,
                    mints,
                    imports,
                    record.engine,
                )?;
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
            _ => self.restore(action, records)?,
        };
        let access = self.tip_project().access;
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
        let current = self.tip_active();
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
            let Change::Edit {
                ops,
                mints,
                imports,
            } = &record.batch.change
            else {
                return Err(invalid_revert());
            };
            match edit::stage_recorded(
                &project,
                ops,
                record.batch.source,
                mints,
                imports,
                record.engine,
            ) {
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
