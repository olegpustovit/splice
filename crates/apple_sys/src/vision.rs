//! Vision analysis, tracking, person mattes (native/analysis.m) and on-device translation
//! (native/translate.swift).
use crate::Error;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
};

unsafe extern "C" {
    fn splice_vision_analyze(
        path: *const c_char,
        kinds: u32,
        every: f64,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn splice_vision_track(
        path: *const c_char,
        from: f64,
        to: f64,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn splice_vision_matte(
        path: *const c_char,
        output: *const c_char,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_translate(
        joined: *const c_char,
        source: *const c_char,
        target: *const c_char,
        out: *mut *mut c_char,
        error: *mut *mut c_char,
    ) -> bool;
    fn free(pointer: *mut c_void);
}

/// Feature prints for shot detection.
pub const SHOTS: u32 = 1;
/// Recognized on-screen text.
pub const TEXT: u32 = 2;
/// Face rectangles.
pub const FACES: u32 = 4;
/// Person rectangles.
pub const PEOPLE: u32 = 8;

/// One analysis result at a sampled frame (`t` in seconds, boxes normalized, top-left origin).
#[derive(Clone, Debug, PartialEq)]
pub enum Finding {
    /// Feature-print distance from the previous sample (0 for the first).
    Print {
        /// Seconds.
        t: f64,
        /// Distance; large jumps are cuts.
        distance: f64,
    },
    /// A recognized text line.
    Text {
        /// Seconds.
        t: f64,
        /// x, y, w, h.
        rect: [f64; 4],
        /// The line.
        text: String,
    },
    /// A face.
    Face {
        /// Seconds.
        t: f64,
        /// x, y, w, h.
        rect: [f64; 4],
    },
    /// A person.
    Person {
        /// Seconds.
        t: f64,
        /// x, y, w, h.
        rect: [f64; 4],
    },
    /// A tracked region.
    Track {
        /// Seconds.
        t: f64,
        /// x, y, w, h.
        rect: [f64; 4],
    },
}

fn path(value: &Path) -> Result<CString, Error> {
    CString::new(value.as_os_str().as_bytes()).map_err(|_| Error::message("A path contains NUL."))
}

fn take(text: *mut c_char, error: *mut c_char, fallback: &'static str) -> Result<String, Error> {
    if text.is_null() {
        // SAFETY: the bridge transfers its diagnostic (or null) to us.
        return Err(unsafe { Error::from_native(error, fallback) });
    }
    // SAFETY: NUL-terminated strdup allocation, freed once here.
    let out = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(text.cast()) };
    Ok(out)
}

fn parse(lines: &str) -> Vec<Finding> {
    lines
        .lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let kind = f.next()?;
            let t: f64 = f.next()?.parse().ok()?;
            if kind == "print" {
                return Some(Finding::Print {
                    t,
                    distance: f.next()?.parse().ok()?,
                });
            }
            let mut rect = [0.0; 4];
            for slot in &mut rect {
                *slot = f.next()?.parse().ok()?;
            }
            Some(match kind {
                "text" => Finding::Text {
                    t,
                    rect,
                    text: f.next().unwrap_or("").to_owned(),
                },
                "face" => Finding::Face { t, rect },
                "person" => Finding::Person { t, rect },
                _ => Finding::Track { t, rect },
            })
        })
        .collect()
}

/// Analyzes video frames sampled every `every` seconds for the `kinds` bits.
pub fn analyze(video: &Path, kinds: u32, every: f64) -> Result<Vec<Finding>, Error> {
    let video = path(video)?;
    let mut error = ptr::null_mut();
    // SAFETY: the path lives for the call.
    let text = unsafe { splice_vision_analyze(video.as_ptr(), kinds, every, &mut error) };
    take(text, error, "Vision analysis failed.").map(|lines| parse(&lines))
}

/// Tracks the region `rect` (normalized, top-left) from `from` to `to` seconds, every frame.
pub fn track(video: &Path, from: f64, to: f64, rect: [f64; 4]) -> Result<Vec<Finding>, Error> {
    let video = path(video)?;
    let mut error = ptr::null_mut();
    let [x, y, w, h] = rect;
    // SAFETY: the path lives for the call.
    let text = unsafe { splice_vision_track(video.as_ptr(), from, to, x, y, w, h, &mut error) };
    take(text, error, "Tracking failed.").map(|lines| parse(&lines))
}

/// Writes a person-segmentation matte movie (white = person) matching the video's frames.
pub fn matte(video: &Path, output: &Path) -> Result<(), Error> {
    let (video, output) = (path(video)?, path(output)?);
    let mut error = ptr::null_mut();
    // SAFETY: both paths live for the call.
    if unsafe { splice_vision_matte(video.as_ptr(), output.as_ptr(), &mut error) } {
        Ok(())
    } else {
        // SAFETY: as in `take`.
        Err(unsafe { Error::from_native(error, "Person segmentation failed.") })
    }
}

/// Translates `texts` on device from `source` to `target` (BCP 47), in order.
pub fn translate(texts: &[String], source: &str, target: &str) -> Result<Vec<String>, Error> {
    let joined = CString::new(texts.join("\u{1e}").replace('\0', ""))
        .map_err(|_| Error::message("Text contains NUL."))?;
    let source = CString::new(source).map_err(|_| Error::message("Bad language."))?;
    let target = CString::new(target).map_err(|_| Error::message("Bad language."))?;
    let (mut out, mut error) = (ptr::null_mut(), ptr::null_mut());
    // SAFETY: strings live for the call; exactly one output is set.
    let ok = unsafe {
        splice_translate(
            joined.as_ptr(),
            source.as_ptr(),
            target.as_ptr(),
            &mut out,
            &mut error,
        )
    };
    let text = if ok {
        take(out, ptr::null_mut(), "Translation failed.")?
    } else {
        return Err(take(ptr::null_mut(), error, "Translation failed.").unwrap_err());
    };
    let result: Vec<String> = text.split('\u{1e}').map(str::to_owned).collect();
    if result.len() != texts.len() {
        return Err(Error::message(
            "Translation returned a different number of texts.",
        ));
    }
    Ok(result)
}
