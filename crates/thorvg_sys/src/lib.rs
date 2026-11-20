//! Lottie and SVG playback through a vendored ThorVG (software rasterizer only).
//!
//! [`open`] reads a Lottie (`.json`, `.lot`) or SVG file and reports its [`Info`];
//! [`Animation::render`] draws frame N into a caller-owned straight-alpha RGBA8 buffer. The
//! canvas is reused, so a warm render allocates nothing on the Rust side. ThorVG is built
//! without its thread support, so one process-wide lock serializes every call into it.
#![allow(unsafe_code)]
#![deny(missing_docs)]

use std::ffi::{CString, c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Mutex;

#[cfg(test)]
mod tests;

/// Largest side, in pixels, a file may declare or a frame may be rendered at.
pub const MAX_SIDE: u32 = 16_384;

/// A file's intrinsic geometry and timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Info {
    /// Width in pixels, rounded up.
    pub width: u32,
    /// Height in pixels, rounded up.
    pub height: u32,
    /// Frames in one loop; 1 for a still (SVG).
    pub frames: u32,
    /// Frames per second; 0 for a still.
    pub fps: f64,
}

/// A file ThorVG could not open, or a frame it could not draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// What failed, for a person.
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

fn error(message: &str) -> Error {
    Error {
        message: message.into(),
    }
}

/// An open Lottie or SVG file and the canvas it draws on.
#[derive(Debug)]
pub struct Animation {
    handle: NonNull<c_void>,
    info: Info,
}

// SAFETY: ThorVG objects are not bound to the thread that made them, and every call that
// reaches them holds ENGINE, so moving the handle to another thread is sound.
unsafe impl Send for Animation {}

/// Whether ThorVG is initialized; held across every call into it.
static ENGINE: Mutex<bool> = Mutex::new(false);

fn engine() -> Result<std::sync::MutexGuard<'static, bool>, Error> {
    let mut ready = ENGINE.lock().unwrap_or_else(|poison| poison.into_inner());
    // SAFETY: init is called once, under the lock; the engine is never terminated.
    if !*ready && !unsafe { splice_tvg_init() } {
        return Err(error("ThorVG could not start."));
    }
    *ready = true;
    Ok(ready)
}

/// Opens a Lottie or SVG file. ThorVG picks the loader by extension (`.json`/`.lot`, `.svg`).
pub fn open(path: &Path) -> Result<Animation, Error> {
    let name = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| error("The vector file path contains NUL."))?;
    let (mut width, mut height, mut frames, mut duration) = (0f32, 0f32, 0f32, 0f32);
    let _engine = engine()?;
    // SAFETY: the path is NUL-terminated and the out-pointers live for the call.
    let raw = unsafe {
        splice_tvg_open(
            name.as_ptr(),
            &mut width,
            &mut height,
            &mut frames,
            &mut duration,
        )
    };
    let handle = NonNull::new(raw).ok_or_else(|| {
        error("ThorVG could not read the file as Lottie JSON or SVG (or it has no size).")
    })?;
    match info(width, height, frames, duration) {
        Ok(info) => Ok(Animation { handle, info }),
        Err(failure) => {
            // SAFETY: the handle is live, owned, and never used again.
            unsafe { splice_tvg_close(handle.as_ptr()) };
            Err(failure)
        }
    }
}

/// Validates what ThorVG reported for a file.
fn info(width: f32, height: f32, frames: f32, duration: f32) -> Result<Info, Error> {
    let side = |value: f32| {
        let value = value.ceil();
        (value >= 1.0 && value <= MAX_SIDE as f32).then_some(value as u32)
    };
    let (Some(width), Some(height)) = (side(width), side(height)) else {
        return Err(error(
            "The vector file's size is outside 1–16384 pixels per side.",
        ));
    };
    let frames = frames.round();
    if frames < 2.0 {
        return Ok(Info {
            width,
            height,
            frames: 1,
            fps: 0.0,
        });
    }
    if !(frames <= 1_000_000.0 && duration.is_finite() && duration > 0.0) {
        return Err(error(
            "The animation has no usable frame count or frame rate.",
        ));
    }
    Ok(Info {
        width,
        height,
        frames: frames as u32,
        fps: f64::from(frames) / f64::from(duration),
    })
}

impl Animation {
    /// The file's size, frame count and frame rate.
    pub fn info(&self) -> Info {
        self.info
    }

    /// Draws `frame` (0-based, below `info().frames`) scaled to `size` into `rgba`: straight
    /// alpha, `size[0] * size[1] * 4` bytes, 4-byte aligned. Transparent where nothing draws.
    pub fn render(&mut self, frame: u32, size: [u32; 2], rgba: &mut [u8]) -> Result<(), Error> {
        let [width, height] = size;
        if frame >= self.info.frames {
            return Err(error("The frame is past the animation's last frame."));
        }
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return Err(error("The render size is outside 1–16384 pixels per side."));
        }
        if rgba.len() != width as usize * height as usize * 4
            || !(rgba.as_ptr() as usize).is_multiple_of(4)
        {
            return Err(error(
                "The pixel buffer must be width × height × 4 bytes, 4-byte aligned.",
            ));
        }
        let _engine = engine()?;
        // SAFETY: the handle is live; the buffer is aligned, exactly width*height pixels and
        // borrowed mutably for the call, and ThorVG keeps no access to it after sync().
        let drawn = unsafe {
            splice_tvg_render(
                self.handle.as_ptr(),
                frame as f32,
                rgba.as_mut_ptr().cast(),
                width,
                height,
            )
        };
        if drawn {
            Ok(())
        } else {
            Err(error("ThorVG could not draw the frame."))
        }
    }
}

impl Drop for Animation {
    fn drop(&mut self) {
        let _engine = ENGINE.lock().unwrap_or_else(|poison| poison.into_inner());
        // SAFETY: the handle is live and owned; it is not used again.
        unsafe { splice_tvg_close(self.handle.as_ptr()) };
    }
}

unsafe extern "C" {
    fn splice_tvg_init() -> bool;
    fn splice_tvg_open(
        path: *const c_char,
        width: *mut f32,
        height: *mut f32,
        frames: *mut f32,
        duration: *mut f32,
    ) -> *mut c_void;
    fn splice_tvg_render(
        handle: *mut c_void,
        frame: f32,
        pixels: *mut u32,
        width: u32,
        height: u32,
    ) -> bool;
    fn splice_tvg_close(handle: *mut c_void);
}
