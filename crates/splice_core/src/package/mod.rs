mod codec;
pub mod journal;
mod media;
mod project;
mod timeline;
mod validate;

use codec::{object, text};
use splice_json::Value as Json;
use splice_model::Project;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A held writer lock and the directory it protects.
#[derive(Debug)]
pub struct Package {
    path: PathBuf,
    lock: File,
    owns_record: bool,
}

/// Typed package failure with a recovery hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageError {
    /// Stable code used by CLI and MCP callers.
    pub code: &'static str,
    /// Details, including the affected path for I/O or load errors.
    pub message: String,
    /// Action that may resolve the error.
    pub fix: &'static str,
}

impl PackageError {
    fn new(code: &'static str, message: String, fix: &'static str) -> Self {
        Self { code, message, fix }
    }

    fn invalid(message: &dyn fmt::Display) -> Self {
        Self::new(
            "BAD_PACKAGE",
            message.to_string(),
            "Check the .splice package against docs/FORMAT.md.",
        )
    }

    fn io(path: &Path, error: io::Error) -> Self {
        Self::new(
            "IO",
            format!("{}: {error}", path.display()),
            "Check the project path, its permissions, and available disk space.",
        )
    }

    fn at(mut self, path: &Path) -> Self {
        self.message = format!("{}: {}", path.display(), self.message);
        self
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PackageError {}

impl Package {
    /// Creates a project directory, syncs its initial snapshot, and holds its writer lock.
    pub fn create(path: &Path, project: &Project) -> Result<Self, PackageError> {
        check_extension(path)?;
        validate::project(project)?;
        if let Err(error) = fs::create_dir(path) {
            if error.kind() == io::ErrorKind::AlreadyExists {
                return Err(PackageError::new(
                    "EXISTS",
                    format!("{} already exists", path.display()),
                    "Open that project or pick a new path.",
                ));
            }
            return Err(PackageError::io(path, error));
        }
        let mut package = Self::acquire(path)?;
        package.save(project)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        sync_dir(parent)?;
        package.record_owner()?;
        Ok(package)
    }

    /// Acquires the writer lock and loads the snapshot and media descriptors.
    pub fn open(path: &Path) -> Result<(Self, Project), PackageError> {
        check_extension(path)?;
        let mut package = Self::acquire(path)?;
        let project = package.load()?;
        package.record_owner()?;
        Ok((package, project))
    }

    /// Canonical package directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Loads the snapshot, media table, and any transcripts from this locked package.
    pub fn load(&self) -> Result<Project, PackageError> {
        let path = self.path.join("project.json");
        let mut project = project::decode(&read_json(&path)?).map_err(|error| error.at(&path))?;
        let path = self.path.join("media.json");
        project.media = media::decode(&read_json(&path)?).map_err(|error| error.at(&path))?;
        let transcripts = self.path.join("transcripts");
        match fs::symlink_metadata(&transcripts) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(
                    PackageError::invalid(&"expected a transcripts directory").at(&transcripts)
                );
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(PackageError::io(&transcripts, error)),
        }
        for item in &project.media {
            let path = transcripts.join(format!("{}.json", item.id));
            if let Some(value) = read_optional_json(&path)? {
                project.transcripts.push(
                    media::decode_transcript(item.id, &value).map_err(|error| error.at(&path))?,
                );
            }
        }
        validate::project(&project).map_err(|error| error.at(&self.path))?;
        Ok(project)
    }

    /// Writes canonical files through fsynced temporary files, then publishes the snapshot.
    pub fn save(&mut self, project: &Project) -> Result<(), PackageError> {
        validate::project(project)?;
        let snapshot = json_bytes(&project::encode(project))?;
        let media = json_bytes(&media::encode(&project.media))?;
        let transcripts = project
            .transcripts
            .iter()
            .map(|transcript| {
                let bytes = json_bytes(&media::encode_transcript(transcript))?;
                Ok((format!("{}.json", transcript.media), bytes))
            })
            .collect::<Result<Vec<_>, PackageError>>()?;
        if !transcripts.is_empty() {
            let directory = self.path.join("transcripts");
            match fs::create_dir(&directory) {
                Ok(()) => sync_dir(&self.path)?,
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        && fs::symlink_metadata(&directory)
                            .is_ok_and(|metadata| metadata.is_dir()) => {}
                Err(error) => return Err(PackageError::io(&directory, error)),
            }
            for (name, bytes) in &transcripts {
                atomic_write(&directory, name, bytes)?;
            }
        }
        atomic_write(&self.path, "media.json", &media)?;
        atomic_write(&self.path, "project.json", &snapshot)
    }

    fn acquire(path: &Path) -> Result<Self, PackageError> {
        let path = path
            .canonicalize()
            .map_err(|error| PackageError::io(path, error))?;
        let lock_path = path.join("lock");
        let options = || {
            let mut options = OpenOptions::new();
            options.read(true).write(true);
            options
        };
        let lock = match options().create_new(true).open(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                check_single_link_file(&lock_path, None)?;
                options()
                    .open(&lock_path)
                    .map_err(|error| PackageError::io(&lock_path, error))?
            }
            Err(error) => return Err(PackageError::io(&lock_path, error)),
        };
        check_single_link_file(&lock_path, Some(&lock))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(PackageError::new(
                    "LOCKED",
                    format!("{} already has a writer", path.display()),
                    "Close the other writer or forward the call to the open viewer.",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(PackageError::io(&lock_path, error)),
        }
        Ok(Self {
            path,
            lock,
            owns_record: false,
        })
    }

    fn record_owner(&mut self) -> Result<(), PackageError> {
        let host = std::process::Command::new("hostname")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .unwrap_or_default();
        let host = host.trim();
        if host.is_empty() {
            return Err(PackageError::invalid(
                &"could not determine the local hostname",
            ));
        }
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let owner = json_bytes(&object(vec![
            ("pid", Json::Unsigned(u64::from(std::process::id()))),
            ("host", text(host)),
            ("since", Json::Unsigned(since)),
        ]))?;
        let path = self.path.join("lock");
        if self
            .lock
            .metadata()
            .map_err(|error| PackageError::io(&path, error))?
            .len()
            != 0
        {
            eprintln!(
                "splice: recovered abandoned writer lock in {}",
                self.path.display()
            );
        }
        self.owns_record = true;
        self.lock
            .set_len(0)
            .and_then(|()| self.lock.seek(SeekFrom::Start(0)))
            .and_then(|_| self.lock.write_all(&owner))
            .and_then(|()| self.lock.sync_all())
            .map_err(|error| PackageError::io(&path, error))
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        if self.owns_record
            && let Err(error) = self.lock.set_len(0).and_then(|()| self.lock.sync_all())
        {
            eprintln!("splice: {}: {error}", self.path.join("lock").display());
        }
    }
}

fn check_extension(path: &Path) -> Result<(), PackageError> {
    if path.extension().is_some_and(|ext| ext == "splice") {
        Ok(())
    } else {
        Err(PackageError::invalid(&"project directory must end with .splice").at(path))
    }
}

fn check_single_link_file(path: &Path, open: Option<&File>) -> Result<(), PackageError> {
    let known = fs::symlink_metadata(path).map_err(|error| PackageError::io(path, error))?;
    if !known.is_file() || known.nlink() != 1 {
        return Err(PackageError::invalid(&"expected a regular file with one link").at(path));
    }
    if let Some(open) = open {
        let held = open
            .metadata()
            .map_err(|error| PackageError::io(path, error))?;
        if (held.dev(), held.ino()) != (known.dev(), known.ino()) {
            return Err(PackageError::invalid(&"file changed while opening it").at(path));
        }
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Json, PackageError> {
    read_optional_json(path)?
        .ok_or_else(|| PackageError::invalid(&"required file is missing").at(path))
}

fn read_optional_json(path: &Path) -> Result<Option<Json>, PackageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(PackageError::io(path, error)),
    };
    if !metadata.is_file() {
        return Err(PackageError::invalid(&"expected a regular package file").at(path));
    }
    let mut file = File::open(path).map_err(|error| PackageError::io(path, error))?;
    let mut bytes = String::new();
    file.read_to_string(&mut bytes)
        .map_err(|error| PackageError::io(path, error))?;
    splice_json::parse(&bytes)
        .map(Some)
        .map_err(|error| PackageError::invalid(&error).at(path))
}

fn json_bytes(value: &Json) -> Result<Vec<u8>, PackageError> {
    let mut bytes = String::new();
    splice_json::write(value, &mut bytes).map_err(|error| PackageError::invalid(&error))?;
    Ok(bytes.into_bytes())
}

fn atomic_write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), PackageError> {
    let target = directory.join(name);
    let temp = directory.join(format!("{name}.tmp"));
    match fs::remove_file(&temp) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(PackageError::io(&temp, error)),
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| PackageError::io(&temp, error))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| PackageError::io(&temp, error))?;
    fs::rename(&temp, &target).map_err(|error| PackageError::io(&target, error))?;
    sync_dir(directory)
}

fn sync_dir(path: &Path) -> Result<(), PackageError> {
    File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|error| PackageError::io(path, error))
}

#[cfg(test)]
mod journal_tests;
