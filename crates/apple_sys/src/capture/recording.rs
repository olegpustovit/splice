use super::{CaptureError, RawTarget, Rect, Target};
use crate::{Error, events::Event};
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

/// The geometry and media clock of an input event stream.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputState {
    /// Capture shape.
    pub kind: InputSourceKind,
    /// Initial capture bounds in global screen points.
    pub rect: Rect,
    /// Source pixels per point.
    pub scale: f64,
    /// Absolute host timestamp of the first complete screen frame.
    pub origin_ns: u64,
    /// Take-relative nanoseconds after removing paused intervals.
    pub elapsed_ns: u64,
    /// Whether new events and cursor changes may be written.
    pub active: bool,
}

/// The capture source underlying an input log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputSourceKind {
    /// A full display.
    Display,
    /// A single window.
    Window,
    /// A subregion of a display.
    Area,
}

#[derive(Default)]
#[repr(C)]
struct NativeInputState {
    rect: Rect,
    scale: f64,
    origin_ns: u64,
    elapsed_ns: u64,
    kind: u32,
    active: bool,
}

const _: () = assert!(size_of::<NativeInputState>() == 64);

/// Owns ScreenCaptureKit outputs and their real-time writers.
pub struct Recording {
    handle: NonNull<c_void>,
    finished: bool,
}

unsafe extern "C" {
    fn splice_recording_start(
        target: *const RawTarget,
        directory: *const c_char,
        microphone: *const c_char,
        system_audio: bool,
        audio_apps: *const c_char,
        camera: *const c_char,
        events: bool,
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
    fn splice_recording_read_input(handle: *mut c_void, out: *mut Event, capacity: usize) -> usize;
    fn splice_recording_dropped_input(handle: *mut c_void) -> u64;
    fn splice_recording_input_state(handle: *mut c_void, out: *mut NativeInputState) -> bool;
}

impl Recording {
    /// Starts an authorized recording in an existing staging directory.
    /// Microphone and camera take a device unique ID, an empty ID for the system default, or
    /// `None` to omit them; the camera records into `camera.mov`. System audio is every app's
    /// sound, or only `audio_apps` (bundle IDs or names) when that is not empty.
    /// Input defaults to the caller's explicit selection and requires Input Monitoring permission.
    pub fn start(
        directory: &Path,
        target: Target,
        microphone: Option<&str>,
        system_audio: bool,
        audio_apps: &[String],
        camera: Option<&str>,
        events: bool,
    ) -> Result<Self, CaptureError> {
        let raw = target.native()?;
        let directory = c_string(directory.as_os_str().as_bytes())?;
        let optional =
            |value: Option<&str>| value.map(|value| c_string(value.as_bytes())).transpose();
        let microphone = optional(microphone)?;
        let camera = optional(camera)?;
        let apps = (!audio_apps.is_empty())
            .then(|| c_string(audio_apps.join("\n").as_bytes()))
            .transpose()?;
        let pointer = |value: &Option<CString>| value.as_ref().map_or(ptr::null(), |v| v.as_ptr());
        let mut denied = false;
        let mut diagnostic = ptr::null_mut();
        // SAFETY: the native bridge copies target and strings before returning and only
        // writes through the two live out parameters.
        let handle = unsafe {
            splice_recording_start(
                &raw,
                directory.as_ptr(),
                pointer(&microphone),
                system_audio,
                pointer(&apps),
                pointer(&camera),
                events,
                &mut denied,
                &mut diagnostic,
            )
        };
        NonNull::new(handle)
            .map(|handle| Self {
                handle,
                finished: false,
            })
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

    /// Stops capture and seals each output file. Queued input can still be drained.
    pub fn finish(&mut self) -> Result<RecordingStats, CaptureError> {
        if self.finished {
            return Err(CaptureError {
                permission_denied: false,
                error: Error::message("The recording is already finished."),
            });
        }
        self.finished = true;
        self.stats(splice_recording_finish)
    }

    /// Drains at most the capacity of the caller's fixed buffer without allocating.
    pub fn read_input(&mut self, out: &mut [Event]) -> usize {
        // SAFETY: the output range is writable and the mutable receiver is the only consumer.
        unsafe { splice_recording_read_input(self.handle.as_ptr(), out.as_mut_ptr(), out.len()) }
    }

    /// Returns how many input records could not be queued.
    pub fn dropped_input(&self) -> u64 {
        // SAFETY: the bridge reference remains alive and the native counter is synchronized.
        unsafe { splice_recording_dropped_input(self.handle.as_ptr()) }
    }

    /// Reports the input clock after capture has reached its first screen frame.
    pub fn input_state(&self) -> Option<InputState> {
        let mut state = NativeInputState::default();
        // SAFETY: out points to a writable record and the native call is synchronous.
        if !unsafe { splice_recording_input_state(self.handle.as_ptr(), &mut state) } {
            return None;
        }
        let kind = match state.kind {
            0 => InputSourceKind::Display,
            1 => InputSourceKind::Window,
            2 => InputSourceKind::Area,
            _ => return None,
        };
        Some(InputState {
            kind,
            rect: state.rect,
            scale: state.scale,
            origin_ns: state.origin_ns,
            elapsed_ns: state.elapsed_ns,
            active: state.active,
        })
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
        let camera = media::probe(&directory.join("camera.mov")).unwrap();
        let camera_track = camera.video.unwrap();
        assert_eq!((camera_track.width, camera_track.height), (96, 64));
        assert_eq!(camera_track.codec, Some(*b"hvc1"));
        assert!((camera.duration_seconds - screen.duration_seconds).abs() <= 1.0 / 60.0);
        for name in ["mic.m4a", "system.m4a"] {
            let audio = media::probe(&directory.join(name)).unwrap();
            assert_eq!(audio.audio[0].channels, Some(2));
            assert_eq!(audio.audio[0].codec, Some(*b"aac "));
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
