use crate::Error;
use std::{
    ffi::{CStr, CString, c_char},
    fmt,
    mem::size_of,
    ptr,
};

mod recording;
pub use recording::{InputSourceKind, InputState, Recording, RecordingStats};

/// Bounds in points. Area origins are local to their display.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Rect {
    /// Horizontal origin.
    pub x: f64,
    /// Vertical origin.
    pub y: f64,
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

/// A capturable screen source.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// A display ID, or zero for the main display.
    Display(u32),
    /// A window ID from the source list.
    Window(u32),
    /// An area in points within the selected display.
    Area {
        /// Display ID, or zero for the main display.
        display: u32,
        /// Display-local rectangle.
        rect: Rect,
    },
}

/// A source that macOS has made available to the capture agent.
#[derive(Debug, PartialEq)]
pub enum Source {
    /// An active display.
    Display {
        /// Native display identifier.
        id: u32,
        /// Whether this display is the system's main display.
        main: bool,
        /// Global bounds in points.
        rect: Rect,
    },
    /// A window from another application.
    Window {
        /// Native window identifier.
        id: u32,
        /// Global bounds in points.
        rect: Rect,
        /// Process that owns the window.
        pid: u32,
        /// Application bundle identifier, when available.
        app: String,
        /// Window title, when available.
        title: String,
    },
    /// A running application with shareable content.
    Application {
        /// Application process identifier.
        pid: u32,
        /// Application bundle identifier.
        bundle: String,
        /// Display name.
        name: String,
    },
}

/// Capture failure, with the system authorization decision kept separate from diagnostics.
#[derive(Debug)]
pub struct CaptureError {
    /// macOS refused screen recording access.
    pub permission_denied: bool,
    error: Error,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(output)
    }
}

impl std::error::Error for CaptureError {}

impl CaptureError {
    // The caller transfers a null pointer or an owned diagnostic as defined in platform.h.
    pub(crate) unsafe fn from_native(error: *mut c_char, denied: bool, fallback: &str) -> Self {
        Self {
            permission_denied: denied,
            // SAFETY: the caller transfers the diagnostic's ownership here.
            error: unsafe { Error::from_native(error, fallback) },
        }
    }
}

#[repr(C)]
struct RawSource {
    rect: Rect,
    kind: u32,
    id: u32,
    main_display: u32,
    owner_pid: u32,
    app: *const c_char,
    name: *const c_char,
}

#[repr(C)]
struct RawSources {
    items: *mut RawSource,
    count: usize,
}

#[repr(C)]
pub(crate) struct RawTarget {
    rect: Rect,
    kind: u32,
    id: u32,
}

const _: () = {
    assert!(size_of::<RawSource>() == 64);
    assert!(size_of::<RawTarget>() == 40);
};

impl Target {
    pub(crate) fn native(self) -> Result<RawTarget, CaptureError> {
        let (kind, id, rect) = match self {
            Self::Display(id) => (0, id, Rect::default()),
            Self::Window(id) => (1, id, Rect::default()),
            Self::Area { display, rect }
                if [rect.x, rect.y, rect.width, rect.height]
                    .into_iter()
                    .all(f64::is_finite)
                    && rect.x >= 0.0
                    && rect.y >= 0.0
                    && rect.width > 0.0
                    && rect.height > 0.0 =>
            {
                (2, display, rect)
            }
            Self::Area { .. } => {
                return Err(CaptureError {
                    permission_denied: false,
                    error: Error::message(
                        "Capture area needs a finite origin and positive dimensions.",
                    ),
                });
            }
        };
        Ok(RawTarget { rect, kind, id })
    }
}

unsafe extern "C" {
    fn splice_capture_sources(
        include_windows: bool,
        out: *mut RawSources,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_capture_sources_dispose(sources: *mut RawSources);
}

impl Drop for RawSources {
    fn drop(&mut self) {
        // SAFETY: this struct owns the native source table, including partial results.
        unsafe { splice_capture_sources_dispose(self) };
    }
}

/// Lists active displays. Include windows and apps only with Screen Recording authorization.
pub fn sources(include_windows: bool) -> Result<Vec<Source>, CaptureError> {
    let mut raw = RawSources {
        items: ptr::null_mut(),
        count: 0,
    };
    let mut denied = false;
    let mut diagnostic = ptr::null_mut();
    // SAFETY: the bridge fills writable outputs and does not retain them.
    if !unsafe { splice_capture_sources(include_windows, &mut raw, &mut denied, &mut diagnostic) } {
        // SAFETY: the bridge transfers the optional failure diagnostic.
        return Err(unsafe {
            CaptureError::from_native(diagnostic, denied, "Could not discover capture sources.")
        });
    }
    if raw.count == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: the live source table owns count populated records.
    let items = unsafe { std::slice::from_raw_parts(raw.items, raw.count) };
    Ok(items.iter().filter_map(RawSource::to_source).collect())
}

impl RawSource {
    fn to_source(&self) -> Option<Source> {
        let copy = |value: *const c_char| {
            if value.is_null() {
                String::new()
            } else {
                // SAFETY: these strings belong to the table, which outlives this call.
                unsafe { CStr::from_ptr(value) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        match self.kind {
            0 => Some(Source::Display {
                id: self.id,
                main: self.main_display != 0,
                rect: self.rect,
            }),
            1 => Some(Source::Window {
                id: self.id,
                rect: self.rect,
                pid: self.owner_pid,
                app: copy(self.app),
                title: copy(self.name),
            }),
            3 => Some(Source::Application {
                pid: self.id,
                bundle: copy(self.app),
                name: copy(self.name),
            }),
            _ => None,
        }
    }
}

unsafe extern "C" {
    fn splice_capture_exclude(names: *const c_char, desktop: bool);
    fn splice_capture_devices() -> *mut c_char;
    fn splice_capture_geometry(
        target: *const RawTarget,
        out: *mut Geometry,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> bool;
    fn splice_window_resize(pid: u32, frame: Rect, width: f64, height: f64, out: *mut Rect) -> u32;
    fn free(pointer: *mut std::ffi::c_void);
}

/// Apps (names or bundle IDs) the next display or area capture leaves out besides Splice, and
/// whether the Finder's desktop icons go too.
pub fn exclude(apps: &[String], desktop: bool) {
    let list = CString::new(apps.join("\n").replace('\0', "")).expect("NUL removed");
    // SAFETY: the string lives for the call; native copies it.
    unsafe { splice_capture_exclude(list.as_ptr(), desktop) };
}

/// What a capture of a target covers right now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Geometry {
    /// Captured bounds in global points.
    pub rect: Rect,
    /// Pixels per point.
    pub scale: f64,
    /// Encoded width in pixels (even).
    pub width: u32,
    /// Encoded height in pixels (even).
    pub height: u32,
}

const _: () = assert!(size_of::<Geometry>() == 48);

/// The bounds, scale and pixel size a capture of `target` would have now (needs Screen
/// Recording permission).
pub fn geometry(target: Target) -> Result<Geometry, CaptureError> {
    let raw = target.native()?;
    let mut out = Geometry::default();
    let mut denied = false;
    let mut diagnostic = ptr::null_mut();
    // SAFETY: inputs and outputs are live for this synchronous call.
    if unsafe { splice_capture_geometry(&raw, &mut out, &mut denied, &mut diagnostic) } {
        Ok(out)
    } else {
        // SAFETY: the failing call transferred its optional diagnostic.
        Err(unsafe {
            CaptureError::from_native(diagnostic, denied, "Could not measure the capture source.")
        })
    }
}

/// Why a window resize did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeError {
    /// Splice lacks the Accessibility permission.
    Untrusted,
    /// No Accessibility window of that process has the given frame.
    NotFound,
    /// The app refused the new size.
    Refused,
}

/// Resizes window `frame` (global points) of process `pid` to `size` points with Accessibility,
/// keeping its top-left corner; returns the frame the window then has (apps clamp sizes).
pub fn resize_window(pid: u32, frame: Rect, size: [f64; 2]) -> Result<Rect, ResizeError> {
    let mut out = Rect::default();
    // SAFETY: by-value inputs and one writable output for this synchronous call.
    match unsafe { splice_window_resize(pid, frame, size[0], size[1], &mut out) } {
        0 => Ok(out),
        1 => Err(ResizeError::Untrusted),
        2 => Err(ResizeError::NotFound),
        _ => Err(ResizeError::Refused),
    }
}

/// A microphone or camera present on this Mac.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    /// `mic` or `camera`.
    pub kind: String,
    /// AVFoundation unique ID, stable for the device across launches.
    pub id: String,
    /// Display name.
    pub name: String,
    /// The system default for its kind.
    pub default: bool,
}

/// Microphones and cameras present now.
pub fn devices() -> Vec<Device> {
    // SAFETY: native returns a strdup'd string.
    let text = unsafe { splice_capture_devices() };
    if text.is_null() {
        return Vec::new();
    }
    // SAFETY: NUL-terminated strdup allocation, freed once here.
    let lines = unsafe { std::ffi::CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    unsafe { free(text.cast()) };
    lines
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(4, '\t');
            Some(Device {
                kind: f.next()?.into(),
                id: f.next()?.into(),
                default: f.next()? == "1",
                name: f.next()?.into(),
            })
        })
        .collect()
}
