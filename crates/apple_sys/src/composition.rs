use crate::Error;
use std::{
    ffi::{CString, c_char, c_void},
    mem::{offset_of, size_of},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{self, NonNull},
};

/// Output codec (the container follows: MP4, QuickTime, M4A or WAV).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// H.264 in MP4.
    H264 = 0,
    /// HEVC in MP4.
    Hevc = 1,
    /// ProRes 422 HQ in QuickTime.
    ProRes422 = 2,
    /// ProRes 4444 with alpha in QuickTime.
    ProRes4444 = 3,
    /// HEVC with alpha in QuickTime.
    HevcAlpha = 4,
    /// Audio only, AAC in M4A.
    Aac = 5,
    /// Audio only, 16-bit PCM WAV.
    Wav = 6,
}

impl Codec {
    /// Whether the codec carries video.
    pub fn video(self) -> bool {
        (self as u32) <= Codec::HevcAlpha as u32
    }

    /// Whether the codec keeps transparency.
    pub fn alpha(self) -> bool {
        matches!(self, Codec::ProRes4444 | Codec::HevcAlpha)
    }

    /// The name export settings report (stable: not the Rust variant's spelling).
    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "H264",
            Codec::Hevc => "HEVC",
            Codec::ProRes422 => "ProRes422",
            Codec::ProRes4444 => "ProRes4444",
            Codec::HevcAlpha => "HEVCAlpha",
            Codec::Aac => "AAC",
            Codec::Wav => "WAV",
        }
    }
}

/// How an export encodes.
#[derive(Clone, Debug, PartialEq)]
pub struct Encoding {
    /// Codec and container.
    pub codec: Codec,
    /// Timeline frames written, half-open.
    pub range: [i32; 2],
    /// Video bits per second; `None` for the default rate.
    pub bitrate: Option<f64>,
    /// Chapter starts (timeline frames) and names.
    pub chapters: Vec<(i32, String)>,
}

impl Encoding {
    /// The whole timeline in `codec` at the default rate, without chapters.
    pub fn new(codec: Codec) -> Self {
        Self {
            codec,
            range: [0, i32::MAX],
            bitrate: None,
            chapters: Vec::new(),
        }
    }
}

#[repr(C)]
struct RawExportSettings {
    codec: u32,
    from: i32,
    to: i32,
    pad: u32,
    bitrate: f64,
    chapters: *const c_char,
}

/// Native track kinds and their underlying inputs.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
pub struct CursorImage {
    /// PNG file.
    pub path: std::path::PathBuf,
    /// Hotspot in PNG pixels.
    pub hot: [f64; 2],
    /// PNG pixels per screen point.
    pub scale: f64,
}

/// A linear volume ramp for a native audio track.
#[repr(C)]
pub struct AudioRamp {
    /// Composition track returned by the builder.
    pub track: i32,
    /// Start and end gains in the native unit range.
    pub gain: [f32; 2],
}

/// A timed layer of title, caption or keystroke text.
#[derive(Clone, Debug)]
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
    /// Font family, weight and decoration.
    pub style: TextStyle,
    /// UTF-16 range that draws; the rest keeps its layout but is transparent (word animators).
    pub visible: Option<[usize; 2]>,
    /// UTF-16 range of the spoken caption word and its RGB.
    pub active: Option<([usize; 2], u32)>,
}

/// Font, weight and decoration of rasterized text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Family; empty or "SF Pro…" uses the system font.
    pub font: String,
    /// 0 regular … 5 black.
    pub weight: u32,
    /// Stroke RGB.
    pub stroke: u32,
    /// Stroke width relative to the font size (0 for none).
    pub stroke_width: f64,
    /// Drop shadow 0…1.
    pub shadow: f64,
    /// Backing box: 0 none, 1 solid, 2 rounded.
    pub boxed: u32,
    /// Backing RGB.
    pub box_color: u32,
    /// 0 left, 1 center, 2 right.
    pub align: u32,
    /// Line height relative to the font size.
    pub line_height: f64,
    /// Wrap lines at the box width; otherwise the font shrinks until each line fits.
    pub wrap: bool,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: String::new(),
            weight: 3,
            stroke: 0,
            stroke_width: 0.0,
            shadow: 0.0,
            boxed: 0,
            box_color: 0,
            align: 1,
            line_height: 1.15,
            wrap: true,
        }
    }
}

#[repr(C)]
struct RawTextStyle {
    font: *const c_char,
    weight: u32,
    color: u32,
    stroke: u32,
    boxed: u32,
    box_color: u32,
    align: u32,
    wrap: u32,
    stroke_width: f64,
    shadow: f64,
    line_height: f64,
}

const _: () = {
    assert!(size_of::<RawSource>() == 64 && offset_of!(RawSource, timeline) == 40);
    assert!(size_of::<FrameSource>() == 24);
    assert!(size_of::<AudioRamp>() == 12 && offset_of!(AudioRamp, gain) == 4);
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
    fn splice_av_frames(
        composition: *mut c_void,
        path: *const c_char,
        width: u32,
        height: u32,
        count: usize,
        seconds: f64,
        fill: extern "C" fn(*mut c_void, usize, *mut u8, usize) -> bool,
        context: *mut c_void,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_source(
        composition: *mut c_void,
        source: *const RawSource,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_range(
        composition: *mut c_void,
        from: i32,
        to: i32,
        tracks: *const i32,
        track_count: usize,
        audio: *const AudioRamp,
        audios: usize,
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
    fn splice_av_player(composition: *mut c_void, error: *mut *mut c_char) -> *mut c_void;
    fn splice_av_player_play(player: *mut c_void, playing: bool);
    fn splice_av_player_ended(player: *mut c_void) -> bool;
    fn splice_av_player_seek(player: *mut c_void, frame: i32) -> bool;
    fn splice_av_grab(
        composition: *mut c_void,
        device: *mut c_void,
        frame: i32,
        sources: *mut FrameSource,
        capacity: usize,
        count: *mut usize,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_reader(
        composition: *mut c_void,
        from: i32,
        to: i32,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_reader_next(
        reader: *mut c_void,
        device: *mut c_void,
        sources: *mut FrameSource,
        capacity: usize,
        count: *mut usize,
        frame: *mut i32,
        error: *mut *mut c_char,
    ) -> i32;
    fn splice_av_reader_release(reader: *mut c_void);
    fn splice_av_writer(
        reader: *mut c_void,
        file: *const c_char,
        settings: *const RawExportSettings,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_writer_target(
        writer: *mut c_void,
        device: *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_av_writer_append(writer: *mut c_void, frame: i32, error: *mut *mut c_char) -> bool;
    fn splice_av_writer_finish(writer: *mut c_void, cancel: bool, error: *mut *mut c_char) -> i32;
    fn splice_av_writer_release(writer: *mut c_void);
    fn splice_text_rgba(
        text: *const c_char,
        width: u32,
        height: u32,
        font: f64,
        style: *const RawTextStyle,
        visible_from: i64,
        visible_to: i64,
        active_from: i64,
        active_to: i64,
        active_color: u32,
        error: *mut *mut c_char,
    ) -> *mut u8;
    fn splice_av_append(
        raw: *mut c_void,
        track: i32,
        spec: *const RawSource,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_av_orientation(raw: *mut c_void, track: i32) -> u32;
    fn free(pointer: *mut c_void);
}

/// Owns a composition during construction.
pub struct Builder(Composition);

/// Prepared AVFoundation asset, video instructions and audio mix.
pub struct Composition(NonNull<c_void>);

/// An AVPlayer and its output for one immutable composition.
pub struct Player(NonNull<c_void>);

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

    /// Adds a composition track for `source` covering `timing`.
    pub fn source(&mut self, source: Source, timing: &Timing) -> Result<i32, Error> {
        let (_path, raw) = raw_source(source, timing)?;
        let mut error = ptr::null_mut();
        // SAFETY: the builder is uniquely borrowed and the descriptors live through the call.
        let track = unsafe { splice_av_source(self.0.0.as_ptr(), &raw, &mut error) };
        if track < 0 {
            Err(diagnostic(error))
        } else {
            Ok(track)
        }
    }

    /// Makes the picture movie that later `Source::Still(path)` tracks play: `count` frames of
    /// `seconds` each at `size`, frame i drawn by `fill(i, rgba)` as straight-alpha RGBA8
    /// (`size[0] * size[1] * 4` bytes). Several frames loop across the item like an animated
    /// GIF (T45.2). `fill` runs on the encoder's queue while this call waits, hence `Send`.
    pub fn frames(
        &mut self,
        path: &Path,
        size: [u32; 2],
        count: usize,
        seconds: f64,
        mut fill: &mut (dyn FnMut(usize, &mut [u8]) -> bool + Send),
    ) -> Result<(), Error> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| Error::message("A picture source path contains NUL."))?;
        let context: *mut &mut (dyn FnMut(usize, &mut [u8]) -> bool + Send) = &mut fill;
        let mut error = ptr::null_mut();
        // SAFETY: the builder is uniquely borrowed; the path and `fill` outlive the call, and the
        // native side never calls `fill_frame` after it returns (it waits out the encoder).
        let made = unsafe {
            splice_av_frames(
                self.0.0.as_ptr(),
                path.as_ptr(),
                size[0],
                size[1],
                count,
                seconds,
                fill_frame,
                context.cast(),
                &mut error,
            )
        };
        if made { Ok(()) } else { Err(diagnostic(error)) }
    }

    /// Inserts another time range of `source` into an existing track (speed-ramp segments).
    pub fn append(&mut self, track: i32, source: Source, timing: &Timing) -> Result<(), Error> {
        let (_path, raw) = raw_source(source, timing)?;
        let mut error = ptr::null_mut();
        // SAFETY: as in `source`.
        if unsafe { splice_av_append(self.0.0.as_ptr(), track, &raw, &mut error) } {
            Ok(())
        } else {
            Err(diagnostic(error))
        }
    }

    /// The track's display orientation: 0 none, 1 90°, 2 180°, 3 270° clockwise.
    pub fn orientation(&self, track: i32) -> u32 {
        // SAFETY: read-only query of a track the builder owns; out-of-range tracks return 0.
        unsafe { splice_av_orientation(self.0.0.as_ptr(), track) }
    }

    /// Appends one contiguous interval: its picture tracks back to front and its audio ramps.
    pub fn range(
        &mut self,
        [from, to]: [i32; 2],
        tracks: &[i32],
        audio: &[AudioRamp],
    ) -> Result<(), Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the slices are valid for their lengths and are read only during this call.
        let ok = unsafe {
            splice_av_range(
                self.0.0.as_ptr(),
                from,
                to,
                tracks.as_ptr(),
                tracks.len(),
                audio.as_ptr(),
                audio.len(),
                &mut error,
            )
        };
        if ok { Ok(()) } else { Err(diagnostic(error)) }
    }

    /// Adds an offline Core Animation text layer.

    /// Publishes the native composition after all ranges have been added.

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
    /// Decodes one frame through the broker into `out` (RGBA8 views on `device`, an
    /// `id<MTLDevice>`), one per layer of its instruction, valid until the next grab.
    pub fn grab(
        &self,
        device: *mut c_void,
        frame: i32,
        out: &mut [FrameSource],
    ) -> Result<usize, Error> {
        let mut count = 0;
        let mut error = ptr::null_mut();
        // SAFETY: `out` is writable for its length; native validates the frame and device.
        let result = unsafe {
            splice_av_grab(
                self.0.as_ptr(),
                device,
                frame,
                out.as_mut_ptr(),
                out.len(),
                &mut count,
                &mut error,
            )
        };
        if result == 1 {
            Ok(count)
        } else {
            Err(diagnostic(error))
        }
    }

    /// Starts decoding the whole composition in order, for export.
    pub fn reader(&self, range: Option<[i32; 2]>) -> Result<Reader, Error> {
        let mut error = ptr::null_mut();
        let [from, to] = range.unwrap_or([0, 0]);
        // SAFETY: the reader retains the composition; the diagnostic storage outlives the call.
        let reader = unsafe { splice_av_reader(self.0.as_ptr(), from, to, &mut error) };
        NonNull::new(reader)
            .map(Reader)
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

/// Decodes a composition in order for export, through the broker compositor.
pub struct Reader(NonNull<c_void>);

// SAFETY: the native reader and its AVFoundation objects may be driven from any one thread at a
// time; this handle is moved to the export thread and used only there.
unsafe impl Send for Reader {}

impl Reader {
    /// The next frame's sources (RGBA8 views on `device`), valid until the next call, or `None`
    /// at the end.
    pub fn next(
        &mut self,
        device: *mut c_void,
        out: &mut [FrameSource],
    ) -> Result<Option<(i32, usize)>, Error> {
        let (mut frame, mut count) = (0, 0);
        let mut error = ptr::null_mut();
        // SAFETY: `out` is writable for its length; the reader is retained by this handle.
        match unsafe {
            splice_av_reader_next(
                self.0.as_ptr(),
                device,
                out.as_mut_ptr(),
                out.len(),
                &mut count,
                &mut frame,
                &mut error,
            )
        } {
            1 => Ok(Some((frame, count))),
            0 => Ok(None),
            _ => Err(diagnostic(error)),
        }
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // SAFETY: the handle owns one native reference, consumed here.
        unsafe { splice_av_reader_release(self.0.as_ptr()) };
    }
}

/// Encodes drawn frames and the reader's audio mix into an MP4.
pub struct Writer(NonNull<c_void>);

// SAFETY: as for `Reader`: used from one thread at a time.
unsafe impl Send for Writer {}

impl Writer {
    /// Opens `file` (which must not exist) for H.264 or HEVC, pumping the reader's audio.
    pub fn new(reader: &Reader, file: &Path, settings: &Encoding) -> Result<Self, Error> {
        let path = CString::new(file.as_os_str().as_bytes())
            .map_err(|_| Error::message("The export path contains NUL."))?;
        let chapters = CString::new(
            settings
                .chapters
                .iter()
                .map(|(at, name)| format!("{at}\t{}", name.replace(['\t', '\n', '\0'], " ")))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .expect("NUL removed");
        let raw = RawExportSettings {
            codec: settings.codec as u32,
            from: settings.range[0],
            to: settings.range[1],
            pad: 0,
            bitrate: settings.bitrate.unwrap_or(0.0),
            chapters: chapters.as_ptr(),
        };
        let mut error = ptr::null_mut();
        // SAFETY: the path, settings and diagnostic storage live through the call; the writer
        // retains the reader.
        let writer =
            unsafe { splice_av_writer(reader.0.as_ptr(), path.as_ptr(), &raw, &mut error) };
        NonNull::new(writer)
            .map(Self)
            .ok_or_else(|| diagnostic(error))
    }

    /// A fresh encoder buffer as an RGBA8 `id<MTLTexture>` on `device` (holding BGRA bytes).
    pub fn target(&mut self, device: *mut c_void) -> Result<*mut c_void, Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the writer is retained by this handle and returns a borrowed texture or null.
        let texture = unsafe { splice_av_writer_target(self.0.as_ptr(), device, &mut error) };
        if texture.is_null() {
            Err(diagnostic(error))
        } else {
            Ok(texture)
        }
    }

    /// Appends the drawn target at `frame`, once the GPU has finished drawing it.
    pub fn append(&mut self, frame: i32) -> Result<(), Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the writer is retained by this handle.
        if unsafe { splice_av_writer_append(self.0.as_ptr(), frame, &mut error) } {
            Ok(())
        } else {
            Err(diagnostic(error))
        }
    }

    /// Finishes the file (`Ok(true)`), or cancels and removes it (`Ok(false)`).
    pub fn finish(&mut self, cancel: bool) -> Result<bool, Error> {
        let mut error = ptr::null_mut();
        // SAFETY: the writer is retained by this handle.
        match unsafe { splice_av_writer_finish(self.0.as_ptr(), cancel, &mut error) } {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(diagnostic(error)),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // SAFETY: the handle owns one native reference, consumed here.
        unsafe { splice_av_writer_release(self.0.as_ptr()) };
    }
}

/// The native descriptor for `source`; the returned path string must outlive the descriptor.
fn raw_source(source: Source, timing: &Timing) -> Result<(Option<CString>, RawSource), Error> {
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
    Ok((path, raw))
}

/// Rasterizes a text layer into straight RGBA8 of `size`.
pub fn text_rgba(layer: &TextLayer, size: [u32; 2]) -> Result<Vec<u8>, Error> {
    let text = CString::new(layer.text.replace('\0', "\u{FFFD}")).expect("NUL replaced");
    let family = CString::new(layer.style.font.replace('\0', "")).expect("NUL removed");
    let style = RawTextStyle {
        font: family.as_ptr(),
        weight: layer.style.weight,
        color: layer.color,
        stroke: layer.style.stroke,
        boxed: layer.style.boxed,
        box_color: layer.style.box_color,
        align: layer.style.align,
        wrap: u32::from(layer.style.wrap),
        stroke_width: layer.style.stroke_width,
        shadow: layer.style.shadow,
        line_height: layer.style.line_height,
    };
    let [from, to] = layer.visible.map_or([-1, -1], |r| r.map(|v| v as i64));
    let ([active_from, active_to], active_color) =
        layer.active.map_or(([-1, -1], 0), |(range, color)| {
            (range.map(|v| v as i64), color)
        });
    let mut error = ptr::null_mut();
    // SAFETY: the strings and style live for the call; native returns a malloc'd
    // width*height*4 buffer.
    let pixels = unsafe {
        splice_text_rgba(
            text.as_ptr(),
            size[0],
            size[1],
            layer.font,
            &style,
            from,
            to,
            active_from,
            active_to,
            active_color,
            &mut error,
        )
    };
    if pixels.is_null() {
        return Err(diagnostic(error));
    }
    let length = size[0] as usize * size[1] as usize * 4;
    // SAFETY: the buffer holds `length` initialized bytes until freed below.
    let bytes = unsafe { std::slice::from_raw_parts(pixels, length) }.to_vec();
    // SAFETY: the allocation came from calloc and is not used again.
    unsafe { free(pixels.cast()) };
    Ok(bytes)
}

/// `Builder::frames`' callback: draws one frame through the borrowed closure.
extern "C" fn fill_frame(context: *mut c_void, index: usize, rgba: *mut u8, bytes: usize) -> bool {
    // SAFETY: `context` is the closure `Builder::frames` borrows for the duration of the native
    // call, which runs this on one thread at a time; `rgba` is a native buffer of `bytes` bytes
    // that nothing else touches until this returns.
    let (fill, pixels) = unsafe {
        (
            &mut *context.cast::<&mut (dyn FnMut(usize, &mut [u8]) -> bool + Send)>(),
            std::slice::from_raw_parts_mut(rgba, bytes),
        )
    };
    // A panic must not unwind into Objective-C: it fails the frame instead.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(index, pixels))).unwrap_or(false)
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
