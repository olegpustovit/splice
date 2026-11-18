use crate::Error;
use std::{
    ffi::{c_char, c_void},
    ptr,
};

unsafe extern "C" {
    fn splice_image_png(
        pixels: *const u8,
        length: usize,
        width: u32,
        height: u32,
        max_edge: u32,
        data: *mut *const u8,
        size: *mut usize,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn splice_image_release(data: *mut c_void);
    fn splice_cursor_current(
        previous: *const u8,
        info: *mut CursorInfo,
        bytes: *mut *const u8,
        size: *mut usize,
        error: *mut *mut c_char,
    ) -> *mut c_void;
}

/// Geometry and content hash for a recorded cursor image.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct CursorInfo {
    /// Hotspot relative to the PNG's top-left corner, in pixels.
    pub hot_x: f64,
    /// Vertical hotspot in pixels.
    pub hot_y: f64,
    /// PNG pixels per cursor point.
    pub scale: f64,
    /// SHA-256 of the PNG and its hotspot metadata.
    pub hash: [u8; 32],
}

const _: () = assert!(std::mem::size_of::<CursorInfo>() == 56);

/// A changed cursor image and the metadata needed to place its hotspot.
pub struct CursorImage {
    /// Placement and identity.
    pub info: CursorInfo,
    /// Encoded PNG.
    pub png: Vec<u8>,
}

/// Samples the system cursor on the application thread. Unchanged or unavailable
/// cursors return `None` without allocating a Rust buffer.
pub fn system_cursor(previous: Option<&[u8; 32]>) -> Result<Option<CursorImage>, Error> {
    let mut info = CursorInfo::default();
    let mut bytes = ptr::null();
    let mut length = 0;
    let mut error = ptr::null_mut();
    // SAFETY: all output parameters are writable. The previous hash stays live during the call.
    let owner = unsafe {
        splice_cursor_current(
            previous.map_or(ptr::null(), |hash| hash.as_ptr()),
            &mut info,
            &mut bytes,
            &mut length,
            &mut error,
        )
    };
    if !owner.is_null() {
        // SAFETY: the retained native object owns the nonempty byte range.
        let png = unsafe { take_bytes(owner, bytes, length) };
        return Ok(Some(CursorImage { info, png }));
    }
    if error.is_null() {
        return Ok(None);
    }
    // SAFETY: a failed native capture transfers its diagnostic.
    Err(unsafe { Error::from_native(error, "System cursor capture failed.") })
}

/// Encodes premultiplied sRGB RGBA8. A nonzero limit bounds the longest edge.
pub fn png(pixels: &[u8], [width, height]: [u32; 2], max_edge: u32) -> Result<Vec<u8>, Error> {
    let mut data = ptr::null();
    let mut length = 0;
    let mut error = ptr::null_mut();
    // SAFETY: all inputs live for the call. Native code validates dimensions against the length.
    let result = unsafe {
        splice_image_png(
            pixels.as_ptr(),
            pixels.len(),
            width,
            height,
            max_edge,
            &mut data,
            &mut length,
            &mut error,
        )
    };
    if result.is_null() {
        // SAFETY: the bridge transfers ownership of the optional diagnostic on failure.
        return Err(unsafe { Error::from_native(error, "PNG encoding failed.") });
    }
    // SAFETY: on success the retained native object owns a nonempty byte buffer of this length.
    Ok(unsafe { take_bytes(result, data, length) })
}

// The caller transfers a retained native owner and its borrowed byte range.
pub(crate) unsafe fn take_bytes(owner: *mut c_void, data: *const u8, length: usize) -> Vec<u8> {
    // SAFETY: the native object is alive and owns the byte range.
    let bytes = unsafe { std::slice::from_raw_parts(data, length) }.to_vec();
    // SAFETY: the copy owns its data and the caller transferred the native reference.
    unsafe { splice_image_release(owner) };
    bytes
}
