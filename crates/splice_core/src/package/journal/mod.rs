//! Durable revision log for a writer-locked project.

mod record;
pub use record::{Batch, Change, MediaImport, Record, Source};

use super::{
    Package, PackageError, atomic_write, codec::*, json_bytes, media, project, read_optional_json,
    sync_dir, validate,
};
use splice_json::Value as Json;
use splice_model::Project;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const JOURNAL: &str = "journal.ops";
const ORIGIN: &str = "journal.base.json";
const SNAPSHOT_EVERY: usize = 64;

/// A recovered edit candidate or its replay failure.
pub type ReplayResult = Result<Project, PackageError>;

/// The result of the periodic snapshot after a successful append.
#[derive(Debug)]
pub enum Checkpoint {
    /// The batch was committed and the next snapshot is not due yet.
    Deferred,
    /// The batch and its snapshot were written.
    Written,
    /// The append is durable, and reopening can replay it. The snapshot write failed.
    Failed(PackageError),
}

/// Owns the writer lock and the verified journal prefix.
#[derive(Debug)]
pub struct Journal {
    package: Package,
    file: File,
    path: PathBuf,
    origin: Project,
    // ponytail: verified records stay in memory (a batch's ops run ~200 bytes, so 10,000 batches
    // hold about 2 MB; revisions themselves are bounded in history::versions). Index older
    // segments on disk if a measured journal passes 50 MB.
    records: Vec<Record>,
    problem: Option<PackageError>,
    /// The (rev, hash) `project.json` holds, so an unchanged close writes nothing.
    saved: (u64, u64),
}

impl Journal {
    /// Creates a project with a durable initial state and an empty journal.
    pub fn create(path: &Path, project: &Project) -> Result<(Self, Project), PackageError> {
        let package = Package::create(path, project)?;
        let journal_path = package.path().join(JOURNAL);
        let file = open_file(&journal_path)?;
        write_origin(&package, project)?;
        let journal = Self {
            package,
            file,
            path: journal_path,
            origin: project.clone(),
            records: Vec::new(),
            problem: None,
            saved: (project.rev, project.hash),
        };
        Ok((journal, project.clone()))
    }

    /// Verifies the chain and replays the batches after the checkpoint.
    ///
    /// Replay receives the current state, the journal origin, and the prefix ending with
    /// the record to apply. It returns a validated candidate. The journal
    /// sets the candidate's revision and hash. On corruption, the returned project is
    /// the last verifiable revision, and the journal refuses further writes.
    pub fn open(
        path: &Path,
        replay: &mut dyn FnMut(&Project, &Project, &[Record]) -> ReplayResult,
    ) -> Result<(Self, Project), PackageError> {
        Self::open_inner(path, replay, false)
    }

    pub(crate) fn open_history(
        path: &Path,
        replay: &mut dyn FnMut(&Project, &Project, &[Record]) -> ReplayResult,
    ) -> Result<(Self, Project), PackageError> {
        Self::open_inner(path, replay, true)
    }

    fn open_inner(
        path: &Path,
        replay: &mut dyn FnMut(&Project, &Project, &[Record]) -> ReplayResult,
        from_origin: bool,
    ) -> Result<(Self, Project), PackageError> {
        let (package, checkpoint) = Package::open(path)?;
        let journal_path = package.path().join(JOURNAL);
        let mut file = open_file(&journal_path)?;
        let origin = match read_origin(&package) {
            Ok(Some(origin)) => origin,
            Ok(None)
                if file
                    .metadata()
                    .map_err(|e| PackageError::io(&journal_path, e))?
                    .len()
                    == 0 =>
            {
                write_origin(&package, &checkpoint)?;
                checkpoint.clone()
            }
            other => {
                let reason = other.err().map_or_else(
                    || "missing journal origin".to_owned(),
                    |error| error.to_string(),
                );
                let problem = Some(
                    PackageError::new(
                        "JOURNAL_CORRUPT",
                        format!(
                            "{reason}; showing unverified snapshot at revision {}",
                            checkpoint.rev
                        ),
                        "Restore the journal origin from a verified backup before editing.",
                    )
                    .at(&journal_path),
                );
                let journal = Self {
                    package,
                    file,
                    path: journal_path,
                    origin: checkpoint.clone(),
                    records: Vec::new(),
                    problem,
                    saved: (checkpoint.rev, checkpoint.hash),
                };
                return Ok((journal, checkpoint));
            }
        };
        let prefix = read_prefix(&mut file, &journal_path, &origin)?;
        let recovered = recover(&origin, &checkpoint, prefix, replay, from_origin);
        if recovered.torn_bytes != 0 {
            eprintln!(
                "splice: truncated {} torn journal bytes in {}",
                recovered.torn_bytes,
                journal_path.display()
            );
        }
        let problem = recovered.problem.map(|error| error.at(&journal_path));
        let journal = Self {
            package,
            file,
            path: journal_path,
            origin,
            records: recovered.records,
            problem,
            saved: (checkpoint.rev, checkpoint.hash),
        };
        Ok((journal, recovered.project))
    }

    /// Directory protected by the journal's writer lock.
    pub fn path(&self) -> &Path {
        self.package.path()
    }

    /// Starting state for the retained revision chain.
    pub fn origin(&self) -> &Project {
        &self.origin
    }

    /// Verified records in revision order.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// If present, the reason this journal is read-only.
    pub fn problem(&self) -> Option<&PackageError> {
        self.problem.as_ref()
    }

    pub(crate) fn load_checkpoint(&self) -> Result<Project, PackageError> {
        self.package.load()
    }

    /// Appends and fsyncs a fully validated batch, then updates the candidate's stamp.
    ///
    /// The candidate must be based on the journal's current revision and hash. If the
    /// periodic snapshot fails, the successful append is returned with its warning.
    pub fn commit(
        &mut self,
        project: &mut Project,
        batch: Batch,
    ) -> Result<Checkpoint, PackageError> {
        self.check_tip(project)?;
        let mut record = Record {
            rev: project
                .rev
                .checked_add(1)
                .ok_or_else(|| bad("rev", "a revision with a successor"))?,
            parent: project.hash,
            hash: 0,
            batch,
        };
        record.validate()?;
        record.hash = record.compute_hash()?;
        let mut bytes = json_bytes(&record.encode(true))?;
        bytes.push(b'\n');
        let previous_end = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(|e| PackageError::io(&self.path, e))?;
        if let Err(error) = self
            .file
            .write_all(&bytes)
            .and_then(|()| self.file.sync_all())
        {
            if let Err(rollback) = self
                .file
                .set_len(previous_end)
                .and_then(|()| self.file.sync_all())
            {
                self.problem = Some(PackageError::new(
                    "IO",
                    format!(
                        "{}: journal append failed ({error}); rollback failed ({rollback})",
                        self.path.display()
                    ),
                    "Reopen the project to recover its durable journal prefix before retrying.",
                ));
                return Err(self.problem.as_ref().expect("rollback problem").clone());
            }
            return Err(PackageError::io(&self.path, error));
        }
        project.rev = record.rev;
        project.hash = record.hash;
        self.records.push(record);
        if self.records.len().is_multiple_of(SNAPSHOT_EVERY) {
            return Ok(match self.checkpoint(project) {
                Ok(()) => Checkpoint::Written,
                Err(error) => Checkpoint::Failed(error),
            });
        }
        Ok(Checkpoint::Deferred)
    }

    /// Saves a checkpoint at the verified tip without rewriting the journal.
    pub fn checkpoint(&mut self, project: &Project) -> Result<(), PackageError> {
        self.check_tip(project)?;
        if self.saved == (project.rev, project.hash) {
            return Ok(());
        }
        self.package.save(project)?;
        self.saved = (project.rev, project.hash);
        Ok(())
    }

    /// Checkpoints the live state and releases the writer lock.
    pub fn close(mut self, project: &Project) -> Result<(), PackageError> {
        self.checkpoint(project)
    }

    fn check_tip(&self, project: &Project) -> Result<(), PackageError> {
        if let Some(problem) = &self.problem {
            return Err(problem.clone());
        }
        let tip = self
            .records
            .last()
            .map_or((self.origin.rev, self.origin.hash), |record| {
                (record.rev, record.hash)
            });
        if (project.rev, project.hash) != tip {
            return Err(PackageError::new(
                "STALE_REVISION",
                format!(
                    "journal is at revision {}, candidate is at {}",
                    tip.0, project.rev
                ),
                "Read the current snapshot and rebase the edit.",
            ));
        }
        Ok(())
    }
}

fn open_file(path: &Path) -> Result<File, PackageError> {
    let options = || {
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        options
    };
    match options().create_new(true).open(path) {
        Ok(file) => {
            sync_dir(path.parent().expect("journal parent"))?;
            Ok(file)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            super::check_single_link_file(path, None)?;
            let file = options()
                .open(path)
                .map_err(|e| PackageError::io(path, e))?;
            super::check_single_link_file(path, Some(&file))?;
            Ok(file)
        }
        Err(error) => Err(PackageError::io(path, error)),
    }
}

fn write_origin(package: &Package, origin: &Project) -> Result<(), PackageError> {
    atomic_write(package.path(), ORIGIN, &json_bytes(&encode_state(origin))?)
}

pub(crate) fn encode_state(origin: &Project) -> Json {
    object(vec![
        ("project", project::encode(origin)),
        ("media", media::encode(&origin.media)),
        (
            "transcripts",
            Json::Array(
                origin
                    .transcripts
                    .iter()
                    .map(|transcript| {
                        object(vec![
                            ("media", display(&transcript.media)),
                            ("data", media::encode_transcript(transcript)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn read_origin(package: &Package) -> Result<Option<Project>, PackageError> {
    let path = package.path().join(ORIGIN);
    let Some(data) = read_optional_json(&path)? else {
        return Ok(None);
    };
    let mut origin = project::decode(data.required("project")?)?;
    shape(&data, &["project", "media", "transcripts"])?;
    origin.media = media::decode(data.required("media")?)?;
    for item in data.array("transcripts")? {
        shape(item, &["media", "data"])?;
        let media = item
            .string("media")?
            .parse()
            .map_err(|_| bad_id("transcript.media"))?;
        origin
            .transcripts
            .push(media::decode_transcript(media, item.required("data")?)?);
    }
    validate::migrate_words(&mut origin);
    crate::edit::resolve_word_anchors(&mut origin);
    validate::project(&origin)?;
    Ok(Some(origin))
}

pub(super) struct Prefix {
    records: Vec<Record>,
    torn_bytes: u64,
    problem: Option<PackageError>,
}

pub(super) fn read_prefix(
    file: &mut File,
    path: &Path,
    origin: &Project,
) -> Result<Prefix, PackageError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| PackageError::io(path, e))?;
    let mut input = BufReader::new(&mut *file);
    let mut prefix = Prefix {
        records: Vec::new(),
        torn_bytes: 0,
        problem: None,
    };
    let mut line = Vec::new();
    let mut offset = 0;
    let (mut rev, mut hash) = (origin.rev, origin.hash);
    loop {
        line.clear();
        let count = input
            .read_until(b'\n', &mut line)
            .map_err(|e| PackageError::io(path, e))?;
        if count == 0 {
            break;
        }
        if line.pop() != Some(b'\n') {
            prefix.torn_bytes = count as u64;
            break;
        }
        match verify(&line, rev, hash) {
            Ok(record) => {
                (rev, hash) = (record.rev, record.hash);
                offset += count as u64;
                prefix.records.push(record);
            }
            Err(error) => {
                prefix.problem = Some(corrupt(rev, &format!("byte {offset}: {}", error.message)));
                break;
            }
        }
    }
    if prefix.torn_bytes != 0 {
        file.set_len(offset)
            .and_then(|()| file.sync_all())
            .map_err(|e| PackageError::io(path, e))?;
    }
    Ok(prefix)
}

fn verify(line: &[u8], rev: u64, hash: u64) -> Result<Record, PackageError> {
    let text = str::from_utf8(line).map_err(|e| PackageError::invalid(&e))?;
    let value = splice_json::parse(text).map_err(|e| PackageError::invalid(&e))?;
    let record = Record::decode(&value)?;
    if rev.checked_add(1) != Some(record.rev) || hash != record.parent {
        return Err(PackageError::invalid(
            &"revision or parent hash is out of sequence",
        ));
    }
    if record.compute_hash()? != record.hash {
        return Err(PackageError::invalid(
            &"record hash does not match its contents",
        ));
    }
    if json_bytes(&record.encode(true))? != line {
        return Err(PackageError::invalid(&"record is not canonical JSON"));
    }
    Ok(record)
}

pub(super) struct Recovery {
    pub(super) project: Project,
    records: Vec<Record>,
    pub(super) torn_bytes: u64,
    pub(super) problem: Option<PackageError>,
}

pub(super) fn recover(
    origin: &Project,
    checkpoint: &Project,
    prefix: Prefix,
    replay: &mut dyn FnMut(&Project, &Project, &[Record]) -> ReplayResult,
    from_origin: bool,
) -> Recovery {
    let position = if (checkpoint.rev, checkpoint.hash) == (origin.rev, origin.hash) {
        Some(0)
    } else {
        prefix
            .records
            .iter()
            .position(|record| (record.rev, record.hash) == (checkpoint.rev, checkpoint.hash))
            .map(|index| index + 1)
    };
    let start = if from_origin {
        0
    } else {
        position.unwrap_or(0)
    };
    let mut project = if from_origin || position.is_none() {
        origin.clone()
    } else {
        checkpoint.clone()
    };
    let mut problem = prefix.problem;
    let mut records = prefix.records;
    for (index, record) in records.iter().enumerate().skip(start) {
        match replay(&project, origin, &records[..=index]).and_then(|p| {
            validate::project(&p)?;
            Ok(p)
        }) {
            Ok(mut next) => {
                next.rev = record.rev;
                next.hash = record.hash;
                project = next;
            }
            Err(error) => {
                problem = Some(corrupt(
                    project.rev,
                    &format!(
                        "replay of revision {} failed: {}",
                        record.rev, error.message
                    ),
                ));
                records.truncate(index);
                break;
            }
        }
    }
    if position.is_none() && problem.is_none() {
        problem = Some(corrupt(
            project.rev,
            &format!(
                "checkpoint revision {} is absent from the verified chain",
                checkpoint.rev
            ),
        ));
    }
    Recovery {
        project,
        records,
        torn_bytes: prefix.torn_bytes,
        problem,
    }
}

fn corrupt(rev: u64, reason: &str) -> PackageError {
    PackageError::new(
        "JOURNAL_CORRUPT",
        format!("{reason}; last verified revision {rev}"),
        "Use the project read-only and restore the journal from a verified backup before editing.",
    )
}
