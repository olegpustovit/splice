use std::{
    ffi::{CStr, CString, OsStr, c_char},
    io,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    time::Duration,
};

unsafe extern "C" {
    fn splice_app_cancel_open_panel();
    fn splice_app_start_accessory() -> bool;
    fn splice_app_wait_accessory(seconds: f64) -> bool;
    fn splice_app_alert(title: *const c_char, message: *const c_char);
    fn splice_app_user_home() -> *mut c_char;
    fn splice_string_free(value: *mut c_char);
    fn getppid() -> i32;
}

/// Dismisses the attached open panel, if called on the application thread.
pub fn cancel_open_panel() {
    // SAFETY: the shim checks the thread and owns its AppKit references.
    unsafe { splice_app_cancel_open_panel() };
}

/// Sets up an accessory AppKit process on the application thread.
pub fn start_accessory() -> bool {
    // SAFETY: the native function checks the thread and owns the AppKit state.
    unsafe { splice_app_start_accessory() }
}

/// Pumps AppKit on its application thread until an event or a timeout. Returns false on quit.
pub fn wait_accessory(timeout: Duration) -> bool {
    // SAFETY: Duration supplies finite nonnegative seconds and the shim checks the thread.
    unsafe { splice_app_wait_accessory(timeout.as_secs_f64()) }
}

/// Shows a modal warning on the application thread; does nothing on another thread.
pub fn alert(title: &str, message: &str) {
    let title = CString::new(title.replace('\0', "\u{FFFD}")).expect("NUL replaced");
    let message = CString::new(message.replace('\0', "\u{FFFD}")).expect("NUL replaced");
    // SAFETY: both strings outlive the call and the shim checks the thread.
    unsafe { splice_app_alert(title.as_ptr(), message.as_ptr()) };
}

/// Indicates whether the process has launchd as its parent.
pub fn has_launchd_parent() -> bool {
    // SAFETY: this OS call takes no arguments and accesses no caller memory.
    unsafe { getppid() == 1 }
}

/// Returns the effective user's home from the system account record.
pub fn user_home() -> io::Result<PathBuf> {
    // SAFETY: the shim returns a newly allocated NUL-terminated string or null.
    let value = unsafe { splice_app_user_home() };
    if value.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "macOS account home unavailable",
        ));
    }
    // SAFETY: the string is valid until freed, and to_path_buf copies its bytes.
    let path = unsafe { PathBuf::from(OsStr::from_bytes(CStr::from_ptr(value).to_bytes())) };
    // SAFETY: the original allocation belongs to this caller.
    unsafe { splice_string_free(value) };
    Ok(path)
}
