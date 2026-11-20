use crate::Error;
use std::ffi::{CString, c_char};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

unsafe extern "C" {
    fn splice_html_render(
        html: *const c_char,
        base: *const c_char,
        width: u32,
        height: u32,
        fps: u32,
        frames: u32,
        out: *const c_char,
        error: *mut *mut c_char,
    ) -> bool;
}

/// Renders `html` (relative URLs resolve against `base`) as `frames` frames at `fps` into a
/// ProRes 4444 movie with alpha at `out`, driven by a virtual clock. Fails when the page does
/// not draw the same frame twice (it must be deterministic). Runs on the main thread.
pub fn render(
    html: &str,
    base: Option<&Path>,
    size: [u32; 2],
    fps: u32,
    frames: u32,
    out: &Path,
) -> Result<(), Error> {
    let text =
        |s: &[u8]| CString::new(s).map_err(|_| Error::message("The page or path contains NUL."));
    let html = text(html.as_bytes())?;
    let base = base.map(|b| text(b.as_os_str().as_bytes())).transpose()?;
    let out = text(out.as_os_str().as_bytes())?;
    let mut error = std::ptr::null_mut();
    // SAFETY: the strings live through the call; the bridge owns nothing after it returns.
    let ok = unsafe {
        splice_html_render(
            html.as_ptr(),
            base.as_ref().map_or(std::ptr::null(), |b| b.as_ptr()),
            size[0],
            size[1],
            fps,
            frames,
            out.as_ptr(),
            &mut error,
        )
    };
    if ok {
        Ok(())
    } else {
        // SAFETY: the bridge transferred its diagnostic.
        Err(unsafe { Error::from_native(error, "Could not render the page.") })
    }
}
