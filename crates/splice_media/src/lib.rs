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
    if let Some(codec) = still_codec(&absolute) {
        return still(stored, &absolute, &metadata, codec, project_fps);
    }
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
    let audio: Vec<AudioTrack> = asset
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
    // Measured once at import so `audio.set {normalize}` can level it headlessly.
    let loudness = if audio.is_empty() {
        None
    } else {
        media::loudness(&absolute).map_err(|failure| bad_media(failure.to_string()))?
    };
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
            loudness,
        },
        take: None,
    })
}

/// Image files ImageIO decodes, by extension.
fn still_codec(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpeg",
        "heic" | "heif" => "heic",
        "gif" => "gif",
        "tif" | "tiff" => "tiff",
        "bmp" => "bmp",
        "webp" => "webp",
        _ => return None,
    })
}

/// A still image: no duration of its own, so it spans any length on the project clock.
// ponytail: animated GIFs import as their first frame; read every frame if users ask for them.
fn still(
    stored: &str,
    absolute: &Path,
    metadata: &fs::Metadata,
    codec: &str,
    project_fps: u32,
) -> Result<Media, Error> {
    let (_, [width, height]) = apple_sys::image::decode_rgba(absolute)
        .map_err(|failure| bad_media(failure.to_string()))?;
    let bookmark = media::bookmark(absolute).map_err(|failure| offline(absolute, &failure))?;
    Ok(Media {
        id: MediaId::from_serial(0),
        kind: MediaKind::Image,
        path: stored.into(),
        bookmark: Some(bookmark),
        size: metadata.len(),
        mtime: metadata.mtime(),
        probe: Probe {
            fps: f64::from(project_fps),
            frames: 1_000_000_000,
            width,
            height,
            audio: Vec::new(),
            codec: Some(codec.into()),
            loudness: None,
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
        fix: "Use a readable audio, video or image file (PNG, JPEG, HEIC, GIF, TIFF) with a supported format.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_a_png_as_a_still_on_the_project_clock() {
        let package = std::env::temp_dir().join(format!("splice-still-{}", std::process::id()));
        fs::create_dir_all(&package).unwrap();
        let pixels: Vec<u8> = (0..48 * 32).flat_map(|i| [i as u8, 90, 200, 255]).collect();
        fs::write(
            package.join("photo.png"),
            apple_sys::image::png(&pixels, [48, 32], 0).unwrap(),
        )
        .unwrap();
        let descriptor = probe(&package, Path::new("photo.png"), 30).unwrap();
        assert_eq!(descriptor.kind, MediaKind::Image);
        assert_eq!([descriptor.probe.width, descriptor.probe.height], [48, 32]);
        assert_eq!(descriptor.probe.codec.as_deref(), Some("png"));
        fs::remove_dir_all(&package).unwrap();
    }

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

    /// BS.1770's reference: a 997 Hz sine at -20 dBFS in both channels measures -20 LUFS.
    #[test]
    fn imports_measure_integrated_loudness() {
        let dir = std::env::temp_dir().join(format!("splice-lufs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rate = 48_000u32;
        let mut samples = Vec::with_capacity(rate as usize * 4 * 2);
        let peak = 10f64.powf(-20.0 / 20.0) * 32767.0;
        for n in 0..rate * 2 {
            let value = (peak
                * (2.0 * std::f64::consts::PI * 997.0 * f64::from(n) / f64::from(rate)).sin())
                as i16;
            samples.extend(value.to_le_bytes());
            samples.extend(value.to_le_bytes());
        }
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + samples.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 4).to_le_bytes());
        wav.extend(4u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((samples.len() as u32).to_le_bytes());
        wav.extend(&samples);
        std::fs::write(dir.join("tone.wav"), wav).unwrap();
        let descriptor = probe(&dir, Path::new("tone.wav"), 30).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let lufs = descriptor.probe.loudness.unwrap();
        assert!((lufs + 20.0).abs() < 0.3, "measured {lufs} LUFS");
    }
}
