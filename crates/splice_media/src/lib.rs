//! Native audio and video import for Splice.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

use apple_sys::media;
use splice_model::{AudioTrack, Frame, Media, MediaId, MediaKind, Probe};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

/// A media import failure and its recovery hint.
#[derive(Debug)]
pub struct Error {
    /// Stable command error code.
    pub code: &'static str,
    /// Failure detail.
    pub message: String,
    /// Suggested correction.
    pub fix: &'static str,
}

/// Opens an absolute path or a path relative to the project package.
/// The store assigns the descriptor its stable ID when the edit commits.
pub fn probe(package: &Path, request: &Path, project_fps: u32) -> Result<Media, Error> {
    let path = package.join(request);
    let absolute = path
        .canonicalize()
        .map_err(|failure| offline(&path, &failure))?;
    let metadata = fs::metadata(&absolute).map_err(|failure| offline(&path, &failure))?;
    if !metadata.is_file() {
        return Err(bad_media("The media path must name a regular file.".into()));
    }
    let package = package
        .canonicalize()
        .map_err(|failure| offline(package, &failure))?;
    let stored = absolute.strip_prefix(&package).unwrap_or(&absolute);
    let stored = stored
        .to_str()
        .ok_or_else(|| bad_media("The resolved media path must be valid UTF-8.".into()))?;
    let asset = media::probe(&absolute).map_err(|failure| bad_media(failure.to_string()))?;
    let fps = asset
        .video
        .and_then(|video| video.nominal_fps)
        .unwrap_or(f64::from(project_fps));
    let frames = (asset.duration_seconds * fps).round();
    if !frames.is_finite()
        || frames > f64::from(Frame::MAX)
        || fps <= 0.0
        || asset.duration_seconds <= 0.0
    {
        return Err(bad_media(
            "The media duration is outside the supported frame range.".into(),
        ));
    }
    let audio = asset
        .audio
        .iter()
        .map(|track| AudioTrack {
            name: format!("track {}", track.track_id),
            channels: track.channels,
        })
        .collect();
    let (kind, width, height, codec) = match asset.video {
        Some(video) => (MediaKind::Video, video.width, video.height, video.codec),
        None => (
            MediaKind::Audio,
            0,
            0,
            asset.audio.first().and_then(|track| track.codec),
        ),
    };
    let bookmark = media::bookmark(&absolute).map_err(|failure| offline(&path, &failure))?;
    Ok(Media {
        id: MediaId::from_serial(0),
        kind,
        path: stored.into(),
        bookmark: Some(bookmark),
        size: metadata.len(),
        mtime: metadata.mtime(),
        probe: Probe {
            fps,
            frames: frames.max(1.0) as Frame,
            width,
            height,
            audio,
            codec: codec.and_then(codec_label),
        },
        take: None,
    })
}

fn codec_label(code: [u8; 4]) -> Option<String> {
    match &code {
        b"avc1" => Some("h264".into()),
        b"hvc1" | b"hev1" => Some("hevc".into()),
        b"mp4a" => Some("aac".into()),
        b"lpcm" => Some("pcm".into()),
        _ if code
            .iter()
            .all(|byte| byte.is_ascii_graphic() || *byte == b' ') =>
        {
            String::from_utf8(code.to_vec()).ok()
        }
        _ => None,
    }
}

fn offline(path: &Path, failure: &dyn std::fmt::Display) -> Error {
    Error {
        code: "MEDIA_OFFLINE",
        message: format!("Could not access {}: {failure}", path.display()),
        fix: "Check that the media is available and readable, then try the path again.",
    }
}

fn bad_media(message: String) -> Error {
    Error {
        code: "BAD_MEDIA",
        message,
        fix: "Use a readable audio or video file with a supported format and duration.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_stereo_audio_on_the_project_clock() {
        let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
        let descriptor = probe(&package, Path::new("stereo.wav"), 30).unwrap();
        assert_eq!(descriptor.path, "stereo.wav");
        assert_eq!(descriptor.kind, MediaKind::Audio);
        assert_eq!(descriptor.probe.fps, 30.0);
        assert_eq!(descriptor.probe.frames, 3);
        assert_eq!((descriptor.probe.width, descriptor.probe.height), (0, 0));
        assert_eq!(descriptor.probe.audio.len(), 1);
        assert_eq!(descriptor.probe.audio[0].channels, Some(2));
        assert!(descriptor.bookmark.is_some_and(|value| !value.is_empty()));
    }
}
