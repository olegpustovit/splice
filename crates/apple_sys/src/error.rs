use std::{
    ffi::{CStr, c_char},
    fmt,
};

/// Failure reported by an Apple service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

unsafe extern "C" {
    fn splice_string_free(value: *mut c_char);
}

// The caller transfers a non-null NUL-terminated string owned by the native bridge.
pub(crate) unsafe fn take_native_string(value: *mut c_char) -> String {
    // SAFETY: the native bridge allocated and terminated this string.
    let copy = unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() };
    // SAFETY: the native string has been copied and is no longer borrowed.
    unsafe { splice_string_free(value) };
    copy
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
        // SAFETY: the non-null diagnostic is owned by the caller.
        Self(unsafe { take_native_string(error) })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}
