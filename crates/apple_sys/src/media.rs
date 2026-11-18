use crate::Error;
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
    fn splice_media_dispose(probe: *mut RawProbe);
}

impl Drop for RawProbe {
    fn drop(&mut self) {
        // SAFETY: this struct owns the allocation made by splice_media_probe, if present.
        unsafe { splice_media_dispose(self) };
    }
}

/// Loads metadata with AVFoundation. Call from a worker when blocking the caller
/// would interrupt the UI.
pub fn probe(path: &Path) -> Result<Probe, Error> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::message("The media path contains a NUL byte."))?;
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

fn fourcc(value: u32) -> Option<[u8; 4]> {
    (value != 0).then_some(value.to_be_bytes())
}
