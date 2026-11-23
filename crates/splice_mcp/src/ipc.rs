use crate::Json;
use apple_sys::ipc::user;
use std::{
    fs::{self, DirBuilder},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, symlink},
        net::UnixStream,
    },
    path::{Path, PathBuf},
};

pub(super) fn send(stream: &mut UnixStream, value: &Json) -> io::Result<()> {
    let mut wire = String::new();
    splice_json::write(value, &mut wire).map_err(io::Error::other)?;
    wire.push('\n');
    stream.write_all(wire.as_bytes())
}

pub(super) fn receive(stream: &UnixStream, max: u64) -> io::Result<Json> {
    let mut line = String::new();
    BufReader::new(stream).take(max + 1).read_line(&mut line)?;
    if line.len() as u64 > max || !line.ends_with('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Incomplete or oversized IPC message",
        ));
    }
    splice_json::parse(&line).map_err(io::Error::other)
}

pub(super) fn socket_path(directory: &Path, name: &str, create: bool) -> io::Result<PathBuf> {
    let mut directory = directory.to_path_buf();
    private_directory(&directory, create)?;
    if directory.join(name).as_os_str().as_bytes().len() >= 104 {
        // Darwin limits sockaddr_un to 104 bytes. Bind through a private short alias
        // so the socket itself remains in the configured state directory.
        let runtime = PathBuf::from(format!("/tmp/splice-{}", user()));
        private_directory(&runtime, true)?;
        let actual = directory.canonicalize()?;
        let id = fs::metadata(&actual)?;
        directory = runtime.join(format!("s-{:x}-{:x}", id.dev(), id.ino()));
        match symlink(&actual, &directory) {
            Ok(()) => (),
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && fs::read_link(&directory)? == actual => {}
            Err(error) => return Err(error),
        }
    }
    Ok(directory.join(name))
}

fn private_directory(path: &Path, create: bool) -> io::Result<()> {
    if create {
        match DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != user() || meta.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Splice socket directory must be private to this user",
        ));
    }
    Ok(())
}
