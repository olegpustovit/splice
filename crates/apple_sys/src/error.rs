use std::{
    ffi::{CStr, c_char},
    fmt,
};

/// Failure reported by an Apple service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

unsafe extern "C" {
    fn splice_error_free(error: *mut c_char);
}

impl Error {
    pub(crate) fn message(message: &str) -> Self {
        Self(message.into())
    }

    // The caller transfers a null pointer or an owned diagnostic from platform.h.
    pub(crate) unsafe fn from_native(error: *mut c_char, fallback: &str) -> Self {
        if error.is_null() {
            return Self::message(fallback);
        }
        // SAFETY: native diagnostics are NUL-terminated and allocated for this caller.
        let description = unsafe { CStr::from_ptr(error).to_string_lossy().into_owned() };
        // SAFETY: the bridge allocator owns the diagnostic and it has been copied.
        unsafe { splice_error_free(error) };
        Self(description)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}
