//! On-device transcription through the Swift SpeechAnalyzer shim (native/speech.swift).
use crate::Error;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
};

unsafe extern "C" {
    fn splice_speech_transcribe(
        path: *const c_char,
        locale: *const c_char,
        out: *mut *mut c_char,
        error: *mut *mut c_char,
    ) -> bool;
    fn free(pointer: *mut c_void);
}

/// One recognized word with its start and end in seconds on the file's clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Display text.
    pub text: String,
    /// Start, seconds.
    pub start: f64,
    /// End, seconds.
    pub end: f64,
}

/// Transcribes the audio in `path` on device (macOS 26+), downloading the language model once.
/// Blocks until the file is done.
pub fn transcribe(path: &Path, locale: &str) -> Result<Vec<Word>, Error> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::message("The audio path contains NUL."))?;
    let locale = CString::new(locale).map_err(|_| Error::message("The locale contains NUL."))?;
    let (mut out, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: both strings live for the call; native returns strdup'd text in exactly one output.
    let ok =
        unsafe { splice_speech_transcribe(path.as_ptr(), locale.as_ptr(), &mut out, &mut error) };
    let take = |pointer: *mut c_char| {
        // SAFETY: the shim returned a NUL-terminated strdup allocation, freed once here.
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        unsafe { free(pointer.cast()) };
        text
    };
    if !ok {
        let message = if error.is_null() {
            "Transcription failed.".into()
        } else {
            take(error)
        };
        return Err(Error::message(&message));
    }
    let text = take(out);
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let start = parts.next()?.parse().ok()?;
            let end = parts.next()?.parse().ok()?;
            let text = parts.next()?.to_owned();
            (end >= start).then_some(Word { text, start, end })
        })
        .collect())
}
