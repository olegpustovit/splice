//! Offline audio through AVAudioEngine manual rendering and SoundAnalysis (native/audio.m).
use crate::Error;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
};

/// An item's effect chain (PLAN.md T19.2); `None` stages are bypassed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chain {
    /// Low cut Hz, low/mid/high gain dB, mid frequency Hz.
    pub eq: Option<[f64; 5]>,
    /// Threshold dB, ratio, makeup gain dB.
    pub compressor: Option<[f64; 3]>,
    /// Gate threshold dB.
    pub gate: Option<f64>,
    /// Voice isolation mix 0…1.
    pub isolation: Option<f64>,
    /// Stereo pan −1…1.
    pub pan: f64,
}

#[repr(C)]
struct RawChain {
    eq: u32,
    compressor: u32,
    gate: u32,
    pad: u32,
    low_cut: f64,
    low: f64,
    mid: f64,
    high: f64,
    mid_frequency: f64,
    threshold: f64,
    ratio: f64,
    makeup: f64,
    gate_threshold: f64,
    isolation: f64,
    pan: f64,
}

unsafe extern "C" {
    fn splice_audio_render(
        input: *const c_char,
        track: u32,
        output: *const c_char,
        chain: *const RawChain,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_audio_envelope(
        input: *const c_char,
        track: u32,
        rate: f64,
        count: *mut usize,
        error: *mut *mut c_char,
    ) -> *mut f32;
    fn splice_audio_classify(
        input: *const c_char,
        track: u32,
        scratch: *const c_char,
        error: *mut *mut c_char,
    ) -> *mut c_char;
    fn free(pointer: *mut c_void);
}

fn path(value: &Path) -> Result<CString, Error> {
    CString::new(value.as_os_str().as_bytes()).map_err(|_| Error::message("A path contains NUL."))
}

fn failed(error: *mut c_char, fallback: &'static str) -> Error {
    // SAFETY: the bridge transfers its diagnostic (or null) to us.
    unsafe { Error::from_native(error, fallback) }
}

/// Renders `track` of `input` through `chain` into a 48 kHz float CAF at `output`.
pub fn render(input: &Path, track: u32, output: &Path, chain: &Chain) -> Result<(), Error> {
    let (input, output) = (path(input)?, path(output)?);
    let [low_cut, low, mid, high, mid_frequency] =
        chain.eq.unwrap_or([20.0, 0.0, 0.0, 0.0, 1000.0]);
    let [threshold, ratio, makeup] = chain.compressor.unwrap_or([-20.0, 1.0, 0.0]);
    let raw = RawChain {
        eq: u32::from(chain.eq.is_some()),
        compressor: u32::from(chain.compressor.is_some()),
        gate: u32::from(chain.gate.is_some()),
        pad: 0,
        low_cut,
        low,
        mid,
        high,
        mid_frequency,
        threshold,
        ratio,
        makeup,
        gate_threshold: chain.gate.unwrap_or(-80.0),
        isolation: chain.isolation.unwrap_or(0.0),
        pan: chain.pan.clamp(-1.0, 1.0),
    };
    let mut error = ptr::null_mut();
    // SAFETY: strings and chain live through the call.
    if unsafe { splice_audio_render(input.as_ptr(), track, output.as_ptr(), &raw, &mut error) } {
        Ok(())
    } else {
        Err(failed(error, "Offline audio rendering failed."))
    }
}

/// RMS envelope of the mono mix of `track` at `rate` values per second.
pub fn envelope(input: &Path, track: u32, rate: f64) -> Result<Vec<f32>, Error> {
    let input = path(input)?;
    let (mut count, mut error) = (0, ptr::null_mut());
    // SAFETY: native returns a calloc'd buffer of `count` floats, or null with a diagnostic.
    let values =
        unsafe { splice_audio_envelope(input.as_ptr(), track, rate, &mut count, &mut error) };
    if values.is_null() {
        return Err(failed(error, "Could not read the audio."));
    }
    // SAFETY: `count` initialized floats until freed below.
    let out = unsafe { std::slice::from_raw_parts(values, count) }.to_vec();
    // SAFETY: allocated by calloc, not used again.
    unsafe { free(values.cast()) };
    Ok(out)
}

/// One SoundAnalysis window: start and duration in seconds, label, confidence.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    /// Window start, seconds.
    pub start: f64,
    /// Window length, seconds.
    pub duration: f64,
    /// Top label (e.g. "speech", "music", "laughter").
    pub label: String,
    /// Confidence 0…1.
    pub confidence: f64,
}

/// Classifies `track` in 1 s windows with Apple's built-in sound classifier.
pub fn classify(input: &Path, track: u32, scratch: &Path) -> Result<Vec<Sound>, Error> {
    let (input, scratch) = (path(input)?, path(scratch)?);
    let mut error = ptr::null_mut();
    // SAFETY: native returns a strdup'd text, or null with a diagnostic.
    let text =
        unsafe { splice_audio_classify(input.as_ptr(), track, scratch.as_ptr(), &mut error) };
    if text.is_null() {
        return Err(failed(error, "Sound analysis failed."));
    }
    // SAFETY: NUL-terminated strdup allocation, freed once here.
    let lines = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(text.cast()) };
    Ok(lines
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            Some(Sound {
                start: fields.next()?.parse().ok()?,
                duration: fields.next()?.parse().ok()?,
                label: fields.next()?.to_owned(),
                confidence: fields.next()?.parse().ok()?,
            })
        })
        .collect())
}
