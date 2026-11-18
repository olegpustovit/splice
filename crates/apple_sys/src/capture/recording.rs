use super::{CaptureError, RawTarget, Target};
use crate::Error;
use std::{
    ffi::{CString, c_char, c_void},
    mem::size_of,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::{self, NonNull},
};

/// Live or finalized screen recording progress.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct RecordingStats {
    /// Seconds since the first complete screen frame with paused time removed.
    pub elapsed_seconds: f64,
    /// Complete screen frames accepted by the writer.
    pub video_frames: u64,
    /// Complete screen frames dropped by the writer.
    pub dropped_frames: u64,
}

const _: () = assert!(size_of::<RecordingStats>() == 24);

/// Owns ScreenCaptureKit outputs and their real-time writers.
pub struct Recording {
    handle: NonNull<c_void>,
}

unsafe extern "C" {
    fn splice_recording_start(
        target: *const RawTarget,
        directory: *const c_char,
        microphone: *const c_char,
        system_audio: bool,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_recording_status(
        handle: *mut c_void,
        out: *mut RecordingStats,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_recording_pause(handle: *mut c_void, pause: bool);
    fn splice_recording_finish(
        handle: *mut c_void,
        out: *mut RecordingStats,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_recording_release(handle: *mut c_void);
}

impl Recording {
    /// Starts an authorized recording in an existing staging directory.
    /// Pass an empty microphone ID for the system default, or `None` to omit it.
    pub fn start(
        directory: &Path,
        target: Target,
        microphone: Option<&str>,
        system_audio: bool,
    ) -> Result<Self, CaptureError> {
        let raw = target.native()?;
        let directory = c_string(directory.as_os_str().as_bytes())?;
        let microphone = microphone
            .map(|device| c_string(device.as_bytes()))
            .transpose()?;
        let mut denied = false;
        let mut diagnostic = ptr::null_mut();
        // SAFETY: the native bridge copies target and strings before returning and only
        // writes through the two live out parameters.
        let handle = unsafe {
            splice_recording_start(
                &raw,
                directory.as_ptr(),
                microphone
                    .as_ref()
                    .map_or(ptr::null(), |device| device.as_ptr()),
                system_audio,
                &mut denied,
                &mut diagnostic,
            )
        };
        NonNull::new(handle)
            .map(|handle| Self { handle })
            .ok_or_else(|| {
                // SAFETY: the failed start transferred its optional diagnostic.
                unsafe {
                    CaptureError::from_native(diagnostic, denied, "Could not start recording.")
                }
            })
    }

    /// Reports progress and asynchronous stream or writer failures.
    pub fn status(&self) -> Result<RecordingStats, CaptureError> {
        self.stats(splice_recording_status)
    }

    /// Pauses or resumes writing without resetting the media clock.
    pub fn set_paused(&mut self, pause: bool) {
        // SAFETY: the handle is alive and the call serializes on the capture queue.
        unsafe { splice_recording_pause(self.handle.as_ptr(), pause) };
    }

    /// Stops capture and seals each output file.
    pub fn finish(self) -> Result<RecordingStats, CaptureError> {
        self.stats(splice_recording_finish)
    }

    fn stats(
        &self,
        call: unsafe extern "C" fn(
            *mut c_void,
            *mut RecordingStats,
            *mut bool,
            *mut *mut c_char,
        ) -> bool,
    ) -> Result<RecordingStats, CaptureError> {
        let mut stats = RecordingStats::default();
        let mut denied = false;
        let mut diagnostic = ptr::null_mut();
        // SAFETY: the handle remains live and outputs are writable throughout the call.
        if unsafe {
            call(
                self.handle.as_ptr(),
                &mut stats,
                &mut denied,
                &mut diagnostic,
            )
        } {
            Ok(stats)
        } else {
            // SAFETY: the failing native call transferred its optional diagnostic.
            Err(unsafe { CaptureError::from_native(diagnostic, denied, "Recording failed.") })
        }
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        // SAFETY: this owner holds the only retained bridge reference.
        unsafe { splice_recording_release(self.handle.as_ptr()) };
    }
}

fn c_string(bytes: &[u8]) -> Result<CString, CaptureError> {
    CString::new(bytes).map_err(|_| CaptureError {
        permission_denied: false,
        error: Error::message("A recording path or device ID contains NUL."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media;
    use std::{fs, process};

    unsafe extern "C" {
        fn splice_recording_fixture(
            directory: *const c_char,
            stats: *mut RecordingStats,
            error: *mut *mut c_char,
        ) -> bool;
    }

    #[test]
    fn native_recording_writes_hevc_and_stereo_aac_on_the_paused_clock() {
        let directory =
            std::env::temp_dir().join(format!("splice-recording-fixture-{}", process::id()));
        fs::create_dir(&directory).unwrap();
        let root = c_string(directory.as_os_str().as_bytes()).unwrap();
        let mut stats = RecordingStats::default();
        let mut diagnostic = ptr::null_mut();
        // SAFETY: all outputs live through this synchronous fixture call.
        if !unsafe { splice_recording_fixture(root.as_ptr(), &mut stats, &mut diagnostic) } {
            // SAFETY: the fixture transfers an owned diagnostic on failure.
            panic!("{}", unsafe {
                Error::from_native(diagnostic, "Capture fixture failed.")
            });
        }
        assert_eq!(
            stats,
            RecordingStats {
                elapsed_seconds: 1.0,
                video_frames: 8,
                dropped_frames: 0,
            }
        );
        let screen = media::probe(&directory.join("screen.mov")).unwrap();
        let video = screen.video.unwrap();
        assert_eq!(
            (video.width, video.height, video.codec),
            (96, 64, Some(*b"hvc1"))
        );
        assert!(
            (screen.duration_seconds - 1.0).abs() <= 1.0 / 60.0,
            "{screen:?}"
        );
        for name in ["mic.m4a", "system.m4a"] {
            let audio = media::probe(&directory.join(name)).unwrap();
            assert_eq!(audio.audio[0].channels, Some(2));
            assert_eq!(audio.audio[0].codec, Some(*b"aac "));
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
