use crate::{Error, error::take_native_string};
use std::{
    ffi::{CString, c_char},
    mem::{align_of, offset_of, size_of},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
};

/// Metadata loaded from a local audio or video file.
#[derive(Debug, PartialEq)]
pub struct Probe {
    /// Asset duration in seconds.
    pub duration_seconds: f64,
    /// Display information for the first video track, if present.
    pub video: Option<VideoTrack>,
    /// Number of video tracks.
    pub video_tracks: u32,
    /// Audio tracks in the order returned by AVFoundation.
    pub audio: Vec<AudioTrack>,
}

/// Display geometry and nominal timing of a video track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoTrack {
    /// Display width after the track's preferred transform.
    pub width: u32,
    /// Display height after the track's preferred transform.
    pub height: u32,
    /// Nominal frames per second, if the track declares it.
    pub nominal_fps: Option<f64>,
    /// CoreMedia media subtype as a four-character code.
    pub codec: Option<[u8; 4]>,
}

/// A native audio track and its first format description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Persistent track ID within the asset.
    pub track_id: i32,
    /// Reported channel count, when available.
    pub channels: Option<u32>,
    /// CoreMedia media subtype as a four-character code.
    pub codec: Option<[u8; 4]>,
}

#[repr(C)]
struct RawAudioTrack {
    track_id: i32,
    codec: u32,
    channels: u32,
}

#[repr(C)]
struct RawProbe {
    duration_seconds: f64,
    nominal_fps: f64,
    width: u32,
    height: u32,
    video_codec: u32,
    video_tracks: u32,
    audio_count: usize,
    audio: *mut RawAudioTrack,
}

const _: () = {
    assert!(size_of::<RawAudioTrack>() == 12);
    assert!(align_of::<RawAudioTrack>() == 4);
    assert!(offset_of!(RawAudioTrack, channels) == 8);
    assert!(size_of::<RawProbe>() == 48);
    assert!(align_of::<RawProbe>() == 8);
    assert!(offset_of!(RawProbe, width) == 16);
    assert!(offset_of!(RawProbe, audio_count) == 32);
    assert!(offset_of!(RawProbe, audio) == 40);
};

unsafe extern "C" {
    fn splice_media_probe(path: *const c_char, out: *mut RawProbe, error: *mut *mut c_char)
    -> bool;
    fn splice_media_bookmark(path: *const c_char, error: *mut *mut c_char) -> *mut c_char;
    fn splice_media_loudness(path: *const c_char, error: *mut *mut c_char) -> f64;
    fn splice_media_dispose(probe: *mut RawProbe);
}

impl Drop for RawProbe {
    fn drop(&mut self) {
        // SAFETY: this struct owns the allocation made by splice_media_probe, if present.
        unsafe { splice_media_dispose(self) };
    }
}

/// Integrated loudness of the file's audio in LUFS (ITU-R BS.1770-4); `None` without audio.
pub fn loudness(path: &Path) -> Result<Option<f64>, Error> {
    let path = native_path(path)?;
    let mut error = ptr::null_mut();
    // SAFETY: the path lives for the call; the diagnostic is transferred on failure.
    let lufs = unsafe { splice_media_loudness(path.as_ptr(), &mut error) };
    if !error.is_null() {
        // SAFETY: native transferred ownership of the diagnostic.
        return Err(unsafe { Error::from_native(error, "Measuring loudness failed.") });
    }
    Ok(lufs.is_finite().then_some(lufs))
}

/// Loads metadata with AVFoundation. Call from a worker when blocking the caller
/// would interrupt the UI.
pub fn probe(path: &Path) -> Result<Probe, Error> {
    let path = native_path(path)?;
    let mut raw = RawProbe {
        duration_seconds: 0.0,
        nominal_fps: 0.0,
        width: 0,
        height: 0,
        video_codec: 0,
        video_tracks: 0,
        audio_count: 0,
        audio: ptr::null_mut(),
    };
    let mut diagnostic = ptr::null_mut();
    // SAFETY: the path is a C string, both outputs are writable, and the native
    // function does not keep pointers to caller memory.
    if !unsafe { splice_media_probe(path.as_ptr(), &mut raw, &mut diagnostic) } {
        // SAFETY: on failure the bridge transfers ownership of the optional diagnostic.
        return Err(unsafe { Error::from_native(diagnostic, "AVFoundation media probe failed.") });
    }

    let audio = if raw.audio_count == 0 {
        Vec::new()
    } else {
        if raw.audio.is_null() || raw.audio_count > isize::MAX as usize / size_of::<RawAudioTrack>()
        {
            return Err(Error::message(
                "The native audio track array has an invalid layout.",
            ));
        }
        // SAFETY: the bridge initialized this many contiguous records and raw retains them
        // until they have been copied to Rust-owned values.
        unsafe { std::slice::from_raw_parts(raw.audio, raw.audio_count) }
            .iter()
            .map(|track| AudioTrack {
                track_id: track.track_id,
                channels: (track.channels != 0).then_some(track.channels),
                codec: fourcc(track.codec),
            })
            .collect()
    };
    Ok(Probe {
        duration_seconds: raw.duration_seconds,
        video: (raw.video_tracks != 0).then_some(VideoTrack {
            width: raw.width,
            height: raw.height,
            nominal_fps: (raw.nominal_fps > 0.0).then_some(raw.nominal_fps),
            codec: fourcc(raw.video_codec),
        }),
        video_tracks: raw.video_tracks,
        audio,
    })
}

/// Creates a base64-encoded Foundation bookmark for a local file.
pub fn bookmark(path: &Path) -> Result<String, Error> {
    let path = native_path(path)?;
    let mut diagnostic = ptr::null_mut();
    // SAFETY: the path is a C string and the diagnostic output is writable.
    let bookmark = unsafe { splice_media_bookmark(path.as_ptr(), &mut diagnostic) };
    if bookmark.is_null() {
        // SAFETY: on failure the bridge transfers ownership of the optional diagnostic.
        return Err(unsafe { Error::from_native(diagnostic, "Could not create a file bookmark.") });
    }
    // SAFETY: on success the bridge transfers ownership of the bookmark string.
    Ok(unsafe { take_native_string(bookmark) })
}

fn native_path(path: &Path) -> Result<CString, Error> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::message("The media path contains a NUL byte."))
}

fn fourcc(value: u32) -> Option<[u8; 4]> {
    (value != 0).then_some(value.to_be_bytes())
}

unsafe extern "C" {
    fn splice_free_space(path: *const std::ffi::c_char, bytes: *mut u64) -> bool;
    fn splice_media_frame_at(
        path: *const std::ffi::c_char,
        seconds: f64,
        oriented: bool,
        width: *mut u32,
        height: *mut u32,
        error: *mut *mut std::ffi::c_char,
    ) -> *mut u8;
    fn splice_media_chapters(
        path: *const std::ffi::c_char,
        error: *mut *mut std::ffi::c_char,
    ) -> *mut std::ffi::c_char;
    fn free(pointer: *mut std::ffi::c_void);
}

/// The first frame of a movie as straight RGBA with its size (alpha preserved).
pub fn first_frame(path: &Path) -> Result<(Vec<u8>, [u32; 2]), Error> {
    movie_frame(path, 0.0, false)
}

/// An uncropped RGBA movie frame at asset time, with the video track's preferred orientation.
pub fn frame_at(path: &Path, seconds: f64) -> Result<(Vec<u8>, [u32; 2]), Error> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(Error::message(
            "The source frame time must be finite and nonnegative.",
        ));
    }
    movie_frame(path, seconds, true)
}

fn movie_frame(path: &Path, seconds: f64, oriented: bool) -> Result<(Vec<u8>, [u32; 2]), Error> {
    let path = native_path(path)?;
    let (mut w, mut h, mut error) = (0, 0, std::ptr::null_mut());
    // SAFETY: out pointers are valid; native returns calloc'd w*h*4 bytes or null.
    let pixels = unsafe {
        splice_media_frame_at(path.as_ptr(), seconds, oriented, &mut w, &mut h, &mut error)
    };
    if pixels.is_null() {
        // SAFETY: the bridge transferred its diagnostic.
        return Err(unsafe { Error::from_native(error, "No frame.") });
    }
    // SAFETY: w*h*4 initialized bytes until freed below.
    let out = unsafe { std::slice::from_raw_parts(pixels, w as usize * h as usize * 4) }.to_vec();
    // SAFETY: calloc allocation, not used again.
    unsafe { free(pixels.cast()) };
    Ok((out, [w, h]))
}

/// Chapter starts (seconds) and titles as AVFoundation reads them.
pub fn chapters(path: &Path) -> Result<Vec<(f64, String)>, Error> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::message("The path contains NUL."))?;
    let mut error = std::ptr::null_mut();
    // SAFETY: native returns a strdup'd string.
    let text = unsafe { splice_media_chapters(path.as_ptr(), &mut error) };
    if text.is_null() {
        // SAFETY: the bridge transferred its diagnostic.
        return Err(unsafe { Error::from_native(error, "No chapters.") });
    }
    // SAFETY: NUL-terminated strdup allocation, freed once here.
    let lines = unsafe { std::ffi::CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(text.cast()) };
    Ok(lines
        .lines()
        .filter_map(|l| {
            let (t, name) = l.split_once('\t')?;
            Some((t.parse().ok()?, name.to_owned()))
        })
        .collect())
}

/// Free bytes on the volume holding `path`, when the system reports it.
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut bytes = 0;
    // SAFETY: the path lives for the call; `bytes` is a valid out pointer.
    unsafe { splice_free_space(path.as_ptr(), &mut bytes) }.then_some(bytes)
}
