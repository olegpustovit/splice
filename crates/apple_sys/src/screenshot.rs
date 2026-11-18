use crate::{
    capture::{CaptureError, RawTarget, Target},
    image::take_bytes,
};
use std::{
    ffi::{c_char, c_void},
    ptr,
};

unsafe extern "C" {
    fn splice_capture_screenshot(
        target: *const RawTarget,
        data: *mut *const u8,
        size: *mut usize,
        denied: *mut bool,
        error: *mut *mut c_char,
    ) -> *mut c_void;
}

/// Captures an authorized display, window, or area as PNG without including the cursor.
pub fn png(target: Target) -> Result<Vec<u8>, CaptureError> {
    let raw = target.native()?;
    let mut data = ptr::null();
    let mut size = 0;
    let mut denied = false;
    let mut diagnostic = ptr::null_mut();
    // SAFETY: the native bridge owns its callbacks and fills the outputs synchronously.
    let owner = unsafe {
        splice_capture_screenshot(&raw, &mut data, &mut size, &mut denied, &mut diagnostic)
    };
    if owner.is_null() {
        // SAFETY: the bridge transfers the optional failure diagnostic.
        return Err(unsafe { CaptureError::from_native(diagnostic, denied, "Screenshot failed.") });
    }
    // SAFETY: owner retains the immutable, nonempty PNG byte range.
    Ok(unsafe { take_bytes(owner, data, size) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Rect;

    #[test]
    fn screenshot_rejects_invalid_area_before_accessing_the_display() {
        for width in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let error = png(Target::Area {
                display: 0,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width,
                    height: 100.0,
                },
            })
            .unwrap_err();
            assert!(!error.permission_denied);
            assert!(error.to_string().contains("positive dimensions"));
        }
    }
}
