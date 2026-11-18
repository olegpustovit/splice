use super::{
    Json,
    export_file::{Output, path_error},
    object,
    platform::FrameSelection,
    text, timeline,
    wire::*,
};
use splice_av::{Composition, encode_png};
use splice_core::history::{Store, StoreError};
use splice_model::Frame;
use std::{
    fmt::{Display, Write as _},
    fs, io,
    os::unix::ffi::OsStrExt,
    path::Path,
};

const INLINE_EDGE: u32 = 640;

/// Renders one frame of the current revision to a new PNG and returns its metadata.
pub fn grab_frame(store: Store, at: Frame, file: &Path) -> Result<Json, StoreError> {
    if !file
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    {
        return Err(bad("output", "a filename ending in .png"));
    }
    FrameSelection::Explicit(vec![at]).resolve(timeline::duration(store.project())?)?;
    let output = Output::prepare(file.to_path_buf(), false)?;
    let revision = store.project().rev;
    let composition = Composition::prepare(store.project(), store.path())
        .map_err(|error| render_error(&error))?;
    let mut pixels = buffer(composition.size())?;
    composition
        .read_frame(at, &mut pixels)
        .map_err(|error| render_error(&error))?;
    let png = encode_png(&pixels, composition.size(), 0).map_err(|error| render_error(&error))?;
    save(&output, &png)?;
    Ok(extend(
        info(at, composition.size()),
        vec![
            ("revision", rev(revision)),
            ("output", display(&output.file().display())),
        ],
    ))
}

pub(super) fn render_tool(
    store: &Store,
    selection: FrameSelection,
    content: &mut Vec<Json>,
) -> Result<Json, StoreError> {
    let requested = selection.resolve(timeline::duration(store.project())?)?;
    let revision = store.project().rev;
    let cache = store.path().join("cache");
    directory(&cache)?;
    let directory_path = cache.join("frames");
    directory(&directory_path)?;
    let composition = Composition::prepare(store.project(), store.path())
        .map_err(|error| render_error(&error))?;
    let size = composition.size();
    let mut pixels = buffer(size)?;
    let mut frames = Vec::with_capacity(requested.len());
    for at in requested {
        composition
            .read_frame(at, &mut pixels)
            .map_err(|error| render_error(&error))?;
        let full = encode_png(&pixels, size, 0).map_err(|error| render_error(&error))?;
        let name = format!("Frame {at}");
        let output = Output::prepare(
            directory_path.join(format!(
                "r{revision}-{:016x}-{at}.png",
                store.project().hash
            )),
            true,
        )?;
        save(&output, &full)?;
        let uri = file_uri(output.file());
        frames.push(extend(info(at, size), vec![("uri", text(&uri))]));
        let inline = if size[0].max(size[1]) <= INLINE_EDGE {
            full
        } else {
            encode_png(&pixels, size, INLINE_EDGE).map_err(|error| render_error(&error))?
        };
        content.push(object(vec![
            ("type", text("image")),
            ("mimeType", text("image/png")),
            ("data", Json::String(base64(&inline))),
        ]));
        content.push(object(vec![
            ("type", text("resource_link")),
            ("name", text(&name)),
            ("uri", text(&uri)),
            ("mimeType", text("image/png")),
        ]));
    }
    Ok(object(vec![
        ("revision", rev(revision)),
        ("frames", Json::Array(frames)),
    ]))
}

fn buffer(size: [u32; 2]) -> Result<Vec<u8>, StoreError> {
    let length = size[0] as usize * size[1] as usize * 4;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(length)
        .map_err(|error| render_error(&error))?;
    pixels.resize(length, 0);
    Ok(pixels)
}

fn info(at: Frame, [w, h]: [u32; 2]) -> Json {
    object(vec![
        ("frame", frame(at)),
        ("width", Json::Unsigned(u64::from(w))),
        ("height", Json::Unsigned(u64::from(h))),
        ("mimeType", text("image/png")),
    ])
}

fn directory(path: &Path) -> Result<(), StoreError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(path_error(path, &error)),
    }
    if !fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
        return Err(failure(
            "IO",
            "The package cache must be a directory.",
            "Restore the cache directory in the project package.",
        ));
    }
    Ok(())
}

fn save(output: &Output, png: &[u8]) -> Result<(), StoreError> {
    fs::write(output.staged(), png).map_err(|error| path_error(output.staged(), &error))?;
    output.publish()
}

fn render_error(error: &dyn Display) -> StoreError {
    failure(
        "FRAME_FAILED",
        &error.to_string(),
        "Check the source media and output location, then render the frame again.",
    )
}

fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            write!(uri, "%{byte:02X}").expect("write to string");
        }
    }
    uri
}

fn base64(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for bytes in bytes.chunks(3) {
        let a = bytes[0];
        let b = bytes.get(1).copied().unwrap_or(0);
        let c = bytes.get(2).copied().unwrap_or(0);
        text.push(char::from(DIGITS[(a >> 2) as usize]));
        text.push(char::from(DIGITS[(((a & 3) << 4) | (b >> 4)) as usize]));
        text.push(if bytes.len() > 1 {
            char::from(DIGITS[(((b & 15) << 2) | (c >> 6)) as usize])
        } else {
            '='
        });
        text.push(if bytes.len() > 2 {
            char::from(DIGITS[(c & 63) as usize])
        } else {
            '='
        });
    }
    text
}
