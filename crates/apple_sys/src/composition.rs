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
#[derive(Clone, Copy, Debug)]
pub struct VideoLayer {
    /// Source aperture as normalized x, y, width and height in oriented image coordinates.
    pub crop: [f64; 4],
    /// Placement in top-left canvas coordinates as x, y, width and height.
    pub destination: [f64; 4],
    /// Scale about `focus` at the two ends of the ramp.
    pub scale: [f64; 2],
    /// Opacity at the two ends of the ramp.
    pub opacity: [f64; 2],
    /// Canvas point the scale keeps fixed: x, y at the start, then x, y at the end.
    pub focus: [f64; 4],
    /// Source clock in seconds at the two ends, for cursor lookups.
    pub source: [f64; 2],
    /// Corner radius in canvas pixels before scaling.
    pub corner: f64,
    /// Drop shadow intensity, 0…1.
    pub shadow: f64,
    /// Cursor size relative to the recorded cursor.
    pub cursor_scale: f64,
    /// Composition track returned by the builder.
    pub track: i32,
    /// Center-crop the oriented source to fit the destination.
    pub center_crop: u32,
    /// Cursor table from `Builder::cursor`, or -1.
    pub cursor: i32,
    /// `EASE` and `CLICK_RINGS` bits.
    pub flags: u32,
}

impl VideoLayer {
    /// Ease the scale and focus ramps (smoothstep) instead of interpolating linearly.
    pub const EASE: u32 = 1;
    /// Draw rings at recorded clicks.
    pub const CLICK_RINGS: u32 = 2;

    /// A plain layer: no zoom focus, corners, shadow or cursor.
    pub fn plain(track: i32, crop: [f64; 4], destination: [f64; 4], canvas: [f64; 2]) -> Self {
        Self {
            crop,
            destination,
            scale: [1.0; 2],
            opacity: [1.0; 2],
            focus: [
                canvas[0] / 2.0,
                canvas[1] / 2.0,
                canvas[0] / 2.0,
                canvas[1] / 2.0,
            ],
            source: [0.0; 2],
            corner: 0.0,
            shadow: 0.0,
            cursor_scale: 1.0,
            track,
            center_crop: 0,
            cursor: -1,
            flags: 0,
        }
    }
}

/// One cursor state on a take's source clock, normalized to the source with a top-left origin.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorSample {
    /// Source seconds.
    pub t: f64,
    /// Normalized x.
    pub x: f64,
    /// Normalized y.
    pub y: f64,
    /// Index into the table's images, or -1 when hidden.
    pub image: i32,
    /// 1 at a click.
    pub click: u32,
}

/// One layer's decoded source for the OGPU compositor: an MTLTexture on the caller's device,
/// cached per IOSurface. `texture` is null for fills or a missing frame.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FrameSource {
    /// Borrowed `id<MTLTexture>`, valid while the player lives.
    pub texture: *mut c_void,
    /// IOSurface ID, stable across frames from the same pooled buffer.
    pub surface: u32,
    /// Pixels.
    pub width: u32,
    /// Pixels.
    pub height: u32,
    pad: u32,
}

impl Default for FrameSource {
    fn default() -> Self {
        Self {
            texture: ptr::null_mut(),
            surface: 0,
            width: 0,
            height: 0,
            pad: 0,
        }
    }
}

/// Encodes whole-texture GPU copies (`sources[i]` into `destinations[i]`, `id<MTLTexture>`s of one
/// size and format) into `command_buffer` (an `id<MTLCommandBuffer>`).
pub fn blit(
    command_buffer: *mut c_void,
    sources: &[*mut c_void],
    destinations: &[*mut c_void],
) -> Result<(), Error> {
    if sources.len() != destinations.len() {
        return Err(Error::message(
            "Frame copies need one destination per source.",
        ));
    }
    let mut error = ptr::null_mut();
    // SAFETY: both slices are valid for their length; native validates every texture pair.
    let ok = unsafe {
        splice_av_blit(
            command_buffer,
            sources.as_ptr(),
            destinations.as_ptr(),
            sources.len(),
            &mut error,
        )
    };
    if ok { Ok(()) } else { Err(diagnostic(error)) }
}

/// A recorded cursor image with its hotspot in PNG pixels and its pixels per point.
pub struct CursorImage {
    /// PNG file.
    pub path: std::path::PathBuf,
    /// Hotspot in PNG pixels.
    pub hot: [f64; 2],
    /// PNG pixels per screen point.
    pub scale: f64,
}

#[repr(C)]
struct RawCursorImage {
    path: *const c_char,
    hot: [f64; 2],
    scale: f64,
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
    assert!(size_of::<VideoLayer>() == 184 && offset_of!(VideoLayer, flags) == 180);
    assert!(size_of::<FrameSource>() == 24);
    assert!(size_of::<CursorSample>() == 32 && offset_of!(CursorSample, click) == 28);
    assert!(size_of::<RawCursorImage>() == 32 && offset_of!(RawCursorImage, scale) == 24);
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
    fn splice_av_blit(
        command_buffer: *mut c_void,
        sources: *const *mut c_void,
        destinations: *const *mut c_void,
        count: usize,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_player_sources(
        player: *mut c_void,
        device: *mut c_void,
        slot: u32,
        sources: *mut FrameSource,
        capacity: usize,
        count: *mut usize,
        frame: *mut i32,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_cursor(
        composition: *mut c_void,
        samples: *const CursorSample,
        sample_count: usize,
        images: *const RawCursorImage,
        image_count: usize,
        points: f64,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_read_frame(
        composition: *mut c_void,
        frame: i32,
        rgba: *mut u8,
        length: usize,
        draw_text: bool,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_player(composition: *mut c_void, error: *mut *mut c_char) -> *mut c_void;
    fn splice_av_player_play(player: *mut c_void, playing: bool);
    fn splice_av_player_ended(player: *mut c_void) -> bool;
    fn splice_av_player_seek(player: *mut c_void, frame: i32) -> bool;
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
    /// Registers a take's cursor track. `points` is the captured source width in screen points.
    pub fn cursor(
        &mut self,
        samples: &[CursorSample],
        images: &[CursorImage],
        points: f64,
    ) -> Result<i32, Error> {
        let paths = images
            .iter()
            .map(|image| CString::new(image.path.as_os_str().as_bytes()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::message("A cursor image path contains NUL."))?;
        let raw: Vec<RawCursorImage> = images
            .iter()
            .zip(&paths)
            .map(|(image, path)| RawCursorImage {
                path: path.as_ptr(),
                hot: image.hot,
                scale: image.scale,
            })
            .collect();
        let mut error = ptr::null_mut();
        // SAFETY: the slices and their C strings stay alive for the duration of the call.
        let index = unsafe {
            splice_av_cursor(
                self.0.0.as_ptr(),
                samples.as_ptr(),
                samples.len(),
                raw.as_ptr(),
                raw.len(),
                points,
                &mut error,
            )
        };
        if index < 0 {
            Err(diagnostic(error))
        } else {
            Ok(index)
        }
    }

    /// Completes the composition.
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
        self.read(frame, rgba, true)
    }

    /// Reads the program picture for a caller that draws its own text.
    pub fn read_picture(&self, frame: i32, rgba: &mut [u8]) -> Result<(), Error> {
        self.read(frame, rgba, false)
    }

    fn read(&self, frame: i32, rgba: &mut [u8], draw_text: bool) -> Result<(), Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the C bridge checks the buffer length and writes only during this call.
        let ok = unsafe {
            splice_av_read_frame(
                self.0.as_ptr(),
                frame,
                rgba.as_mut_ptr(),
                rgba.len(),
                draw_text,
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
    /// Whether the native playback clock has reached the end of its composition.
    pub fn ended(&self) -> bool {
        // SAFETY: this handle owns the player and is called on its owning thread.
        unsafe { splice_av_player_ended(self.0.as_ptr()) }
    }

    /// Converts a decoded frame directly into a free shared Metal texture.
    ///
    /// Finish GPU work sampling the texture before writing to it again.
    /// Takes the newest displayed frame's decoded sources, one per layer of its instruction, as
    /// textures on `device` (an `id<MTLDevice>`). `slot` (0…2) keeps the buffers alive until reused:
    /// pass a slot whose GPU work has completed. Returns the frame and source count.
    pub fn sources(
        &mut self,
        device: *mut c_void,
        slot: u32,
        out: &mut [FrameSource],
    ) -> Result<Option<(i32, usize)>, Error> {
        let (mut frame, mut count) = (0, 0);
        let mut error = ptr::null_mut();
        // SAFETY: `out` is writable for its length; native validates the device and slot.
        let result = unsafe {
            splice_av_player_sources(
                self.0.as_ptr(),
                device,
                slot,
                out.as_mut_ptr(),
                out.len(),
                &mut count,
                &mut frame,
                &mut error,
            )
        };
        match result {
            1 => Ok(Some((frame, count))),
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
