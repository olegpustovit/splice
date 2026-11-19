use super::wire::failure;
use splice_core::history::StoreError;
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

/// Private staging on the destination volume, removed on success, error or cancellation.
pub(super) struct Output {
    file: PathBuf,
    directory: PathBuf,
    staged: PathBuf,
    overwrite: bool,
}

impl Output {
    pub(super) fn prepare(file: PathBuf, overwrite: bool) -> Result<Self, StoreError> {
        let name = file.file_name().ok_or_else(|| {
            failure(
                "BAD_ARGS",
                "The export needs a filename.",
                "Pass an MP4 file path.",
            )
        })?;
        let parent = file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = parent
            .canonicalize()
            .map_err(|error| path_error(parent, &error))?;
        let file = directory.join(name);
        check_destination(&file, overwrite)?;
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        for _ in 0..64 {
            let token = NEXT_STAGE.fetch_add(1, Ordering::Relaxed);
            let stage = directory.join(format!(".splice-export-{}-{token}", std::process::id()));
            match builder.create(&stage) {
                Ok(()) => {
                    let staged = stage.join(name);
                    return Ok(Self {
                        file,
                        directory: stage,
                        staged,
                        overwrite,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(path_error(&directory, &error)),
            }
        }
        Err(failure(
            "IO",
            "Could not reserve a private export directory.",
            "Use another output directory.",
        ))
    }

    pub(super) fn file(&self) -> &Path {
        &self.file
    }

    pub(super) fn staged(&self) -> &Path {
        &self.staged
    }

    pub(super) fn publish(&self) -> Result<(), StoreError> {
        File::open(&self.staged)
            .and_then(|file| file.sync_all())
            .map_err(|error| path_error(&self.staged, &error))?;
        // Both operations publish whole-file results. The link also claims an unused name atomically.
        let result = if self.overwrite {
            check_destination(self.file(), true)?;
            fs::rename(&self.staged, self.file())
        } else {
            fs::hard_link(&self.staged, self.file())
        };
        result.map_err(|error| path_error(self.file(), &error))
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

/// Writes a caption sidecar beside the export; an existing file is replaced only with
/// `overwrite` (M11).
pub(super) fn write_sidecar(path: &Path, text: &str, overwrite: bool) -> Result<(), StoreError> {
    check_destination(path, overwrite)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    options
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|error| path_error(path, &error))
}

fn check_destination(path: &Path, overwrite: bool) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(_) if !overwrite => Err(exists(path)),
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(failure(
            "BAD_ARGS",
            &format!("{} is not a regular file.", path.display()),
            "Choose a regular MP4 output file.",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(path_error(path, &error)),
    }
}

fn exists(path: &Path) -> StoreError {
    failure(
        "EXISTS",
        &format!("{} already exists.", path.display()),
        "Choose a different path or set overwrite:true to replace the file.",
    )
}

pub(super) fn path_error(path: &Path, error: &io::Error) -> StoreError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        exists(path)
    } else {
        failure(
            "IO",
            &format!("{}: {error}", path.display()),
            "Check the export directory, permissions and available disk space.",
        )
    }
}
