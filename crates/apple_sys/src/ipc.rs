use std::{
    io,
    os::unix::{io::AsRawFd, net::UnixStream},
};

unsafe extern "C" {
    fn geteuid() -> u32;
    fn getpeereid(socket: i32, effective_uid: *mut u32, effective_gid: *mut u32) -> i32;
}

/// Effective user ID of this process.
pub fn user() -> u32 {
    // SAFETY: no arguments or caller-owned storage.
    unsafe { geteuid() }
}

/// Verifies the effective user of a connected Unix peer.
pub fn verify_peer(stream: &UnixStream) -> io::Result<()> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: the stream is borrowed and both outputs live for this call.
    if unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if uid != user() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Viewer IPC requires the same user",
        ));
    }
    Ok(())
}
