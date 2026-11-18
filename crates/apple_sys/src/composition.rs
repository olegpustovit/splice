use crate::Error;
use std::{
    ffi::{CString, c_char, c_void},
    mem::{offset_of, size_of},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{self, NonNull},
};

/// Native track kinds and their underlying inputs.
pub enum Source {
    /// Primary picture of a movie.
    Video(PathBuf),
    /// An audio track by its position in the asset.
    Audio(PathBuf, u32),
    /// An image held for the entire interval.
    Still(PathBuf),
    /// A diagonal gradient, or a solid color when both RGB values match.
    Color(u32, u32),
}

/// Mapping of one timeline item to its source in seconds.
pub struct Timing {
    /// Inclusive first and exclusive last timeline frames.
    pub timeline: [i32; 2],
    /// Asset time at the first timeline frame.
    pub source_start: f64,
    /// End of usable source on the descriptor clock.
    pub source_limit: f64,
    /// Source seconds per timeline second.
    pub speed: f64,
    /// Native source frame duration, used for a final held video frame.
    pub source_frame_duration: f64,
}

#[repr(C)]
struct RawSource {
    path: *const c_char,
    source_start: f64,
    source_limit: f64,
    speed: f64,
    source_frame_duration: f64,
    timeline: [i32; 2],
    kind: u32,
    index: u32,
    colors: [u32; 2],
}

/// A native picture layer, with ramp values sampled over a composition interval.
#[repr(C)]
pub struct VideoLayer {
    /// Source aperture as normalized x, y, width and height in oriented image coordinates.
    pub crop: [f64; 4],
    /// Placement in top-left canvas coordinates as x, y, width and height.
    pub destination: [f64; 4],
    /// Scale about the canvas center at the two ends of the ramp.
    pub scale: [f64; 2],
    /// Opacity at the two ends of the ramp.
    pub opacity: [f64; 2],
    /// Composition track returned by the builder.
    pub track: i32,
    /// Center-crop the oriented source to fit the destination.
    pub center_crop: u32,
}

/// A linear volume ramp for a native audio track.
#[repr(C)]
pub struct AudioRamp {
    /// Composition track returned by the builder.
    pub track: i32,
    /// Start and end gains in the native unit range.
    pub gain: [f32; 2],
}

/// A timed layer of caption or title text for offline rendering.
pub struct TextLayer {
    /// Content for the current interval.
    pub text: String,
    /// Bounds in top-left canvas coordinates.
    pub bounds: [f64; 4],
    /// Font size in canvas pixels.
    pub font: f64,
    /// Opacity at the two ends of the ramp.
    pub opacity: [f32; 2],
    /// Text RGB.
    pub color: u32,
}

#[repr(C)]
struct RawText {
    text: *const c_char,
    bounds: [f64; 4],
    font: f64,
    opacity: [f32; 2],
    color: u32,
}

const _: () = {
    assert!(size_of::<RawSource>() == 64 && offset_of!(RawSource, timeline) == 40);
    assert!(size_of::<VideoLayer>() == 104 && offset_of!(VideoLayer, center_crop) == 100);
    assert!(size_of::<AudioRamp>() == 12 && offset_of!(AudioRamp, gain) == 4);
    assert!(size_of::<RawText>() == 64 && offset_of!(RawText, color) == 56);
};

unsafe extern "C" {
    fn splice_av_create(
        width: u32,
        height: u32,
        fps: u32,
        end: i32,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_release(composition: *mut c_void);
    fn splice_av_source(
        composition: *mut c_void,
        source: *const RawSource,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_range(
        composition: *mut c_void,
        from: i32,
        to: i32,
        video: *const VideoLayer,
        videos: usize,
        audio: *const AudioRamp,
        audios: usize,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_text(
        composition: *mut c_void,
        from: i32,
        to: i32,
        text: *const RawText,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_finish(composition: *mut c_void, error: *mut *mut c_char) -> bool;
    fn splice_av_read_frame(
        composition: *mut c_void,
        frame: i32,
        rgba: *mut u8,
        length: usize,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_player(composition: *mut c_void, error: *mut *mut c_char) -> *mut c_void;
    fn splice_av_player_play(player: *mut c_void, playing: bool);
    fn splice_av_player_seek(player: *mut c_void, frame: i32) -> bool;
    fn splice_av_player_copy(
        player: *mut c_void,
        rgba: *mut u8,
        length: usize,
        frame: *mut i32,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_texture(
        width: u32,
        height: u32,
        device: *mut *mut c_void,
        image: *mut *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_player_into_texture(
        player: *mut c_void,
        texture: *mut c_void,
        frame: *mut i32,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_export(
        composition: *mut c_void,
        file: *const c_char,
        hevc: bool,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_export_status(
        job: *mut c_void,
        progress: *mut f32,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_export_cancel(job: *mut c_void);
    fn splice_av_export_release(job: *mut c_void);
}

/// Owns a composition during construction.
pub struct Builder(Composition);

/// Prepared AVFoundation asset, video instructions and audio mix.
pub struct Composition(NonNull<c_void>);

/// An AVPlayer and its output for one immutable composition.
pub struct Player(NonNull<c_void>);

/// An RGBA texture backed by shared Metal memory on the default device.
pub struct ProgramTexture {
    storage: NonNull<c_void>,
    device: *mut c_void,
    image: *mut c_void,
}

/// A native encoder job retaining the composition until encoding completes.
pub struct Export(NonNull<c_void>);

/// Latest observation of a native export.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportState {
    /// The encoder is running or queued, with fractional progress.
    Running(f32),
    /// A finished file is available.
    Complete,
    /// Cancellation completed.
    Cancelled,
}

impl Drop for Composition {
    fn drop(&mut self) {
        // SAFETY: this is the last Rust owner of the retained native object.
        unsafe { splice_av_release(self.0.as_ptr()) };
    }
}

impl Builder {
    /// Starts lowering a nonempty timeline.
    pub fn new(size: [u32; 2], fps: u32, end: i32) -> Result<Self, Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the diagnostic points to writable storage.
        let raw = unsafe { splice_av_create(size[0], size[1], fps, end, &mut error) };
        let raw = NonNull::new(raw).ok_or_else(|| diagnostic(error))?;
        Ok(Self(Composition(raw)))
    }

    /// Adds a native track with one contiguous source mapping.
    pub fn source(&mut self, source: Source, timing: &Timing) -> Result<i32, Error> {
        let (path, kind, index, colors) = match source {
            Source::Video(path) => (Some(path), 0, 0, [0; 2]),
            Source::Audio(path, index) => (Some(path), 1, index, [0; 2]),
            Source::Still(path) => (Some(path), 2, 0, [0; 2]),
            Source::Color(a, b) => (None, 3, 0, [a, b]),
        };
        let path = path
            .map(|p| CString::new(p.as_os_str().as_bytes()))
            .transpose()
            .map_err(|_| Error::message("A composition source path contains NUL."))?;
        let raw = RawSource {
            path: path.as_ref().map_or(ptr::null(), |value| value.as_ptr()),
            source_start: timing.source_start,
            source_limit: timing.source_limit,
            speed: timing.speed,
            source_frame_duration: timing.source_frame_duration,
            timeline: timing.timeline,
            kind,
            index,
            colors,
        };
        let mut error = ptr::null_mut();
        // SAFETY: the builder is uniquely borrowed and the descriptors live through the call.
        let track = unsafe { splice_av_source(self.0.0.as_ptr(), &raw, &mut error) };
        if track < 0 {
            Err(diagnostic(error))
        } else {
            Ok(track)
        }
    }

    /// Appends the ordered video layers and audio ramps for one contiguous interval.
    pub fn range(
        &mut self,
        [from, to]: [i32; 2],
        video: &[VideoLayer],
        audio: &[AudioRamp],
    ) -> Result<(), Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the slices are valid for their lengths and are read only during this call.
        let ok = unsafe {
            splice_av_range(
                self.0.0.as_ptr(),
                from,
                to,
                video.as_ptr(),
                video.len(),
                audio.as_ptr(),
                audio.len(),
                &mut error,
            )
        };
        if ok { Ok(()) } else { Err(diagnostic(error)) }
    }

    /// Adds an offline Core Animation text layer.
    pub fn text(&mut self, [from, to]: [i32; 2], layer: &TextLayer) -> Result<(), Error> {
        // A native display string ends at NUL. Replace embedded NULs, preserving the rest.
        let text = CString::new(layer.text.replace('\0', "�")).unwrap();
        let raw = RawText {
            text: text.as_ptr(),
            bounds: layer.bounds,
            font: layer.font,
            opacity: layer.opacity,
            color: layer.color,
        };
        let mut error = ptr::null_mut();
        // SAFETY: both the descriptor and its text are borrowed until the call returns.
        let ok = unsafe { splice_av_text(self.0.0.as_ptr(), from, to, &raw, &mut error) };
        if ok { Ok(()) } else { Err(diagnostic(error)) }
    }

    /// Publishes the native composition after all ranges have been added.
    pub fn finish(self) -> Result<Composition, Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the builder owns a valid native object and provides a diagnostic output.
        let ok = unsafe { splice_av_finish(self.0.0.as_ptr(), &mut error) };
        if ok {
            Ok(self.0)
        } else {
            Err(diagnostic(error))
        }
    }
}

impl Composition {
    /// Creates a paused AVPlayer with a pixel-buffer output on the composition clock.
    pub fn player(&self) -> Result<Player, Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the native output retains the composition and only borrows error storage.
        let player = unsafe { splice_av_player(self.0.as_ptr(), &mut error) };
        NonNull::new(player)
            .map(Player)
            .ok_or_else(|| diagnostic(error))
    }
    /// Reads an offscreen program frame into an exact-size RGBA8 buffer.
    pub fn read_frame(&self, frame: i32, rgba: &mut [u8]) -> Result<(), Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the C bridge checks the buffer length and writes only during this call.
        let ok = unsafe {
            splice_av_read_frame(
                self.0.as_ptr(),
                frame,
                rgba.as_mut_ptr(),
                rgba.len(),
                &mut error,
            )
        };
        if ok { Ok(()) } else { Err(diagnostic(error)) }
    }

    /// Encodes the same composition and audio mix into a new MP4 at the supplied path.
    pub fn start_export(&self, file: &Path, hevc: bool) -> Result<Export, Error> {
        let path = CString::new(file.as_os_str().as_bytes())
            .map_err(|_| Error::message("The export path contains NUL."))?;
        let mut error = ptr::null_mut();
        // SAFETY: both the path and diagnostic storage live through the call.
        let job = unsafe { splice_av_export(self.0.as_ptr(), path.as_ptr(), hevc, &mut error) };
        NonNull::new(job)
            .map(Export)
            .ok_or_else(|| diagnostic(error))
    }
}

impl Player {
    /// Copies the newest sample to the caller's exact-size RGBA buffer if one is ready.
    pub fn copy_frame(&mut self, rgba: &mut [u8]) -> Result<Option<i32>, Error> {
        let mut frame = 0;
        let mut error = ptr::null_mut();
        // SAFETY: the bridge checks buffer length and does not retain caller memory.
        let result = unsafe {
            splice_av_player_copy(
                self.0.as_ptr(),
                rgba.as_mut_ptr(),
                rgba.len(),
                &mut frame,
                &mut error,
            )
        };
        match result {
            1 => Ok(Some(frame)),
            0 => Ok(None),
            _ => Err(diagnostic(error)),
        }
    }

    /// Converts a decoded frame directly into a free shared Metal texture.
    ///
    /// Finish GPU work sampling the texture before writing to it again.
    pub fn copy_into(&mut self, texture: &mut ProgramTexture) -> Result<Option<i32>, Error> {
        let mut frame = 0;
        let mut error = ptr::null_mut();
        // SAFETY: both native objects are owned for the call and native checks their dimensions.
        let result = unsafe {
            splice_av_player_into_texture(
                self.0.as_ptr(),
                texture.storage.as_ptr(),
                &mut frame,
                &mut error,
            )
        };
        match result {
            1 => Ok(Some(frame)),
            0 => Ok(None),
            _ => Err(diagnostic(error)),
        }
    }

    /// Starts or pauses playback.
    pub fn set_playing(&mut self, playing: bool) {
        // SAFETY: this handle owns the player and is called on its owning thread.
        unsafe { splice_av_player_play(self.0.as_ptr(), playing) };
    }

    /// Seeks to a frame on the composition clock.
    pub fn seek(&mut self, frame: i32) -> Result<(), Error> {
        // SAFETY: this handle owns the player and native code validates the frame.
        if unsafe { splice_av_player_seek(self.0.as_ptr(), frame) } {
            Ok(())
        } else {
            Err(Error::message("The seek frame is outside the timeline."))
        }
    }
}

impl ProgramTexture {
    /// Allocates the shared storage and its sampled image on the default Metal device.
    pub fn new([width, height]: [u32; 2]) -> Result<Self, Error> {
        let mut image = ptr::null_mut();
        let mut device = ptr::null_mut();
        let mut error = ptr::null_mut();
        // SAFETY: the native constructor validates dimensions and writes only to these outputs.
        let storage =
            unsafe { splice_av_texture(width, height, &mut device, &mut image, &mut error) };
        let storage = NonNull::new(storage).ok_or_else(|| diagnostic(error))?;
        Ok(Self {
            storage,
            device,
            image,
        })
    }

    /// Returns the texture pointer for a renderer on the same Metal device.
    /// The caller must keep this texture alive until all its consumers finish.
    pub fn image_for_device(&self, device: *mut c_void) -> Option<NonNull<c_void>> {
        if self.device == device {
            NonNull::new(self.image)
        } else {
            None
        }
    }
}

impl Drop for ProgramTexture {
    fn drop(&mut self) {
        // SAFETY: this is the retained native owner of the texture and storage.
        unsafe { splice_av_release(self.storage.as_ptr()) };
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        // SAFETY: the native object is retained by this handle.
        unsafe { splice_av_release(self.0.as_ptr()) };
    }
}

impl Export {
    /// Polls encoder progress or returns the terminal result.
    pub fn status(&self) -> Result<ExportState, Error> {
        let mut error = ptr::null_mut();
        let mut progress = 0.0;
        // SAFETY: this handle retains the session and the diagnostic outputs are writable.
        match unsafe { splice_av_export_status(self.0.as_ptr(), &mut progress, &mut error) } {
            0 => Ok(ExportState::Running(progress.clamp(0.0, 1.0))),
            1 => Ok(ExportState::Complete),
            2 => Ok(ExportState::Cancelled),
            _ => Err(diagnostic(error)),
        }
    }

    /// Requests cancellation of an active encoding.
    pub fn cancel(&mut self) {
        // SAFETY: this handle retains the session through the call.
        unsafe { splice_av_export_cancel(self.0.as_ptr()) };
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        // SAFETY: the handle owns one native reference, which is consumed by this call.
        unsafe { splice_av_export_release(self.0.as_ptr()) };
    }
}

fn diagnostic(value: *mut c_char) -> Error {
    // SAFETY: callers pass only the transferred diagnostic (or null) from the native bridge.
    unsafe {
        Error::from_native(
            value,
            "AVFoundation could not build or read the composition.",
        )
    }
}
