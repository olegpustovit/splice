//! Export requests (PLAN.md T21.1–T21.2): presets, ranges, bitrate, chapters, caption sidecars
//! and aspect variants. Portable: the macOS job encodes what this resolves; Linux tests it.
use super::{Json, wire::*};
use splice_core::{commands::CommandError, history::StoreError};
use splice_model::*;

/// Output kinds the native writer knows (mirrors `apple_sys::composition::Codec`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    H264,
    Hevc,
    ProRes422,
    ProRes4444,
    HevcAlpha,
    Aac,
    Wav,
    Gif,
    Poster,
}

/// A named export preset.
#[derive(Clone, Copy, Debug)]
pub(super) struct Preset {
    pub name: &'static str,
    pub kind: Kind,
    pub extension: &'static str,
    /// Canvas size; `None` keeps the project's.
    pub size: Option<[u32; 2]>,
    /// Video Mbps; `None` for the encoder default.
    pub mbps: Option<f64>,
}

pub(super) const PRESETS: &[Preset] = &[
    Preset {
        name: "social",
        kind: Kind::H264,
        extension: "mp4",
        size: None,
        mbps: None,
    },
    Preset {
        name: "social-h264",
        kind: Kind::H264,
        extension: "mp4",
        size: None,
        mbps: None,
    },
    Preset {
        name: "social-hevc",
        kind: Kind::Hevc,
        extension: "mp4",
        size: None,
        mbps: None,
    },
    Preset {
        name: "youtube-1080",
        kind: Kind::H264,
        extension: "mp4",
        size: Some([1920, 1080]),
        mbps: Some(16.0),
    },
    Preset {
        name: "youtube-4k",
        kind: Kind::Hevc,
        extension: "mp4",
        size: Some([3840, 2160]),
        mbps: Some(45.0),
    },
    Preset {
        name: "shorts",
        kind: Kind::H264,
        extension: "mp4",
        size: Some([1080, 1920]),
        mbps: Some(12.0),
    },
    Preset {
        name: "reels",
        kind: Kind::H264,
        extension: "mp4",
        size: Some([1080, 1920]),
        mbps: Some(12.0),
    },
    Preset {
        name: "tiktok",
        kind: Kind::H264,
        extension: "mp4",
        size: Some([1080, 1920]),
        mbps: Some(12.0),
    },
    Preset {
        name: "linkedin",
        kind: Kind::H264,
        extension: "mp4",
        size: Some([1080, 1080]),
        mbps: Some(10.0),
    },
    Preset {
        name: "prores-422",
        kind: Kind::ProRes422,
        extension: "mov",
        size: None,
        mbps: None,
    },
    Preset {
        name: "prores-4444",
        kind: Kind::ProRes4444,
        extension: "mov",
        size: None,
        mbps: None,
    },
    Preset {
        name: "hevc-alpha",
        kind: Kind::HevcAlpha,
        extension: "mov",
        size: None,
        mbps: None,
    },
    Preset {
        name: "audio-m4a",
        kind: Kind::Aac,
        extension: "m4a",
        size: None,
        mbps: None,
    },
    Preset {
        name: "audio-wav",
        kind: Kind::Wav,
        extension: "wav",
        size: None,
        mbps: None,
    },
    Preset {
        name: "gif",
        kind: Kind::Gif,
        extension: "gif",
        size: None,
        mbps: None,
    },
    Preset {
        name: "poster",
        kind: Kind::Poster,
        extension: "png",
        size: None,
        mbps: None,
    },
];

/// One resolved output.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Output {
    pub preset: &'static str,
    pub kind: Kind,
    pub extension: &'static str,
    /// Canvas size to render at (the project's unless a preset or variant changes it).
    pub size: [u32; 2],
    /// Frames written, half-open.
    pub range: [Frame; 2],
    /// Video bits per second.
    pub bitrate: Option<f64>,
    /// Output frames per second: the project's, or `fps` (frame sampling of the timeline
    /// clock, `splice_eval::resample`).
    pub fps: u32,
    /// A GIF's width in pixels (its height keeps the canvas shape); `None` for other kinds.
    pub gif_width: Option<u32>,
    /// Chapter starts and names (markers inside the range).
    pub chapters: Vec<(Frame, String)>,
    /// SRT or WebVTT sidecar.
    pub sidecar: Option<&'static str>,
}

fn preset(name: &str) -> Result<Preset, StoreError> {
    PRESETS
        .iter()
        .find(|p| p.name == name)
        .copied()
        .ok_or_else(|| {
            bad(
                "preset",
                &PRESETS
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        })
}

/// `resolution` choices: the output's shorter edge in pixels (Recorder's 720p, 1080p, 4K).
const RESOLUTIONS: [u32; 4] = [720, 1080, 1440, 2160];

/// `quality` levels: H.264 Mbps at 1080p30 (Recorder's export sheet).
const QUALITY: [(&str, f64); 4] = [
    ("web", 4.0),
    ("social", 8.0),
    ("high", 16.0),
    ("studio", 40.0),
];

/// `size` scaled so its shorter edge is `edge` pixels, keeping its shape (even sizes; upscaling
/// is allowed).
fn short_edge(size: [u32; 2], edge: u32) -> [u32; 2] {
    let short = f64::from(size[0].min(size[1]).max(1));
    let even = |v: u32| ((f64::from(v) * f64::from(edge) / short / 2.0).round() as u32 * 2).max(2);
    [even(size[0]), even(size[1])]
}

/// Bits per second for a `quality` level: its 1080p30 H.264 rate scaled by pixels and by the
/// square root of the frame rate; HEVC needs 0.6 of it.
fn quality_bitrate(mbps: f64, size: [u32; 2], fps: u32, kind: Kind) -> f64 {
    let pixels = f64::from(size[0]) * f64::from(size[1]) / (1920.0 * 1080.0);
    let codec = if kind == Kind::Hevc { 0.6 } else { 1.0 };
    mbps * 1e6 * pixels * (f64::from(fps) / 30.0).sqrt() * codec
}

/// The canvas for an aspect, keeping the project's longer edge.
pub(super) fn canvas(size: [u32; 2], aspect: &str) -> Result<[u32; 2], StoreError> {
    let (w, h) = match aspect {
        "9:16" => (9.0, 16.0),
        "4:5" => (4.0, 5.0),
        "1:1" => (1.0, 1.0),
        "4:3" => (4.0, 3.0),
        "16:9" => (16.0, 9.0),
        "16:10" => (16.0, 10.0),
        "3:2" => (3.0, 2.0),
        _ => return Err(bad("aspect", "9:16, 4:5, 1:1, 4:3, 16:9, 16:10 or 3:2")),
    };
    let long = f64::from(size[0].max(size[1]));
    let even = |v: f64| (v / 2.0).round() as u32 * 2;
    Ok(if w >= h {
        [even(long), even(long * h / w)]
    } else {
        [even(long * w / h), even(long)]
    })
}

/// Resolves `export.start` arguments into outputs (one per variant).
pub(super) fn resolve(
    project: &Project,
    end: Frame,
    args: &Json,
) -> Result<Vec<Output>, StoreError> {
    let range = match (
        args.field("from"),
        args.field("to"),
        args.optional_string("marker")?,
    ) {
        (_, _, Some(marker)) => {
            let id: MarkerId = marker.parse().map_err(|_| bad("marker", "a marker ID"))?;
            let index = project
                .markers
                .iter()
                .position(|m| m.id == id)
                .ok_or_else(|| bad("marker", "a marker from timeline.snapshot"))?;
            let to = project.markers.get(index + 1).map_or(end, |m| m.at);
            [project.markers[index].at, to]
        }
        (from, to, None) => [
            range_frame("from", from, 0, end)?,
            range_frame("to", to, end, end)?,
        ],
    };
    if range[1] <= range[0] {
        return Err(bad("to", "a frame after from, inside the timeline"));
    }
    let bitrate = match args.field("bitrate") {
        None => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|mbps| (0.5..=400.0).contains(mbps))
                .ok_or_else(|| bad("bitrate", "Mbps from 0.5 to 400"))?
                * 1e6,
        ),
    };
    let resolution = match args.field("resolution") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|edge| RESOLUTIONS.iter().find(|&&r| u64::from(r) == edge))
                .copied()
                .ok_or_else(|| {
                    bad(
                        "resolution",
                        "720, 1080, 1440 or 2160 (the shorter edge in pixels)",
                    )
                })?,
        ),
    };
    let quality = match args.optional_string("quality")? {
        None => None,
        Some(name) => Some(
            QUALITY
                .iter()
                .find(|(q, _)| *q == name)
                .map(|&(_, mbps)| mbps)
                .ok_or_else(|| bad("quality", "web, social, high or studio"))?,
        ),
    };
    if quality.is_some() && bitrate.is_some() {
        return Err(bad(
            "quality",
            "quality or bitrate, not both (bitrate sets the rate itself)",
        ));
    }
    let fps = match args.field("fps") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|fps| u32::try_from(fps).ok())
                .filter(|fps| VIDEO_FPS.contains(fps) || GIF_FPS.contains(fps))
                .ok_or_else(|| {
                    bad(
                        "fps",
                        "24, 25, 30, 50 or 60 for a video; 10, 15 or 24 for a GIF",
                    )
                })?,
        ),
    };
    let gif_width = match args.field("width") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|width| u32::try_from(width).ok())
                .filter(|width| (GIF_MIN_WIDTH..=GIF_MAX_WIDTH).contains(width))
                .ok_or_else(|| {
                    CommandError::rule(
                        "BAD_ARGS",
                        "width",
                        Some(value.clone()),
                        &format!("a GIF width from {GIF_MIN_WIDTH} to {GIF_MAX_WIDTH} pixels"),
                        &format!(
                            "Pass width:{GIF_MAX_WIDTH} or less (a GIF is at most {GIF_MAX_WIDTH} px wide; export a video preset for a larger picture)."
                        ),
                    )
                })?,
        ),
    };
    let sidecar = match args.optional_string("captions")? {
        None => None,
        Some("srt") => Some("srt"),
        Some("vtt") => Some("vtt"),
        Some(_) => return Err(bad("captions", "srt or vtt")),
    };
    let chapters = if args.field("chapters").and_then(Json::as_bool) == Some(false) {
        Vec::new()
    } else {
        project
            .markers
            .iter()
            .filter(|m| m.at >= range[0] && m.at < range[1])
            .map(|m| (m.at, m.name.clone()))
            .collect()
    };
    let output = |name: &str, aspect: Option<&str>| -> Result<Output, StoreError> {
        let Preset {
            name: preset,
            kind,
            extension,
            size: fixed,
            mbps,
        } = preset(name)?;
        let fps = match (kind, fps) {
            (Kind::Gif, None) => gif_rate(project.fps),
            (Kind::Gif, Some(fps)) if GIF_FPS.contains(&fps) => fps,
            (Kind::Gif, Some(_)) => return Err(bad("fps", "10, 15 or 24 for a GIF")),
            (Kind::Aac | Kind::Wav | Kind::Poster, Some(_)) => {
                return Err(bad(
                    "fps",
                    "omitted for audio presets and posters (they have no frame rate)",
                ));
            }
            (_, None) => project.fps,
            (_, Some(fps)) if VIDEO_FPS.contains(&fps) => fps,
            (_, Some(_)) => return Err(bad("fps", "24, 25, 30, 50 or 60 for a video")),
        };
        let gif_width = match (kind, gif_width) {
            (Kind::Gif, width) => Some(width.unwrap_or(GIF_WIDTH)),
            (_, None) => None,
            (_, Some(_)) => {
                return Err(bad(
                    "width",
                    "omitted for presets other than gif (resolution sizes a video)",
                ));
            }
        };
        let size = match (aspect, fixed) {
            (Some(aspect), _) => canvas(fixed.unwrap_or(project.size), aspect)?,
            (None, Some(size)) => size,
            (None, None) => project.size,
        };
        let size = match resolution {
            None => size,
            Some(_) if matches!(kind, Kind::Aac | Kind::Wav | Kind::Gif) => {
                return Err(bad(
                    "resolution",
                    "omitted for audio and GIF presets (a GIF takes width, at most 960 px)",
                ));
            }
            Some(edge) => short_edge(size, edge),
        };
        let quality = match quality {
            None => None,
            Some(_) if !matches!(kind, Kind::H264 | Kind::Hevc) => {
                return Err(bad(
                    "quality",
                    "omitted for presets that are not H.264 or HEVC (ProRes, audio, GIF and poster set no bitrate)",
                ));
            }
            Some(mbps) => Some(quality_bitrate(mbps, size, fps, kind)),
        };
        Ok(Output {
            preset,
            kind,
            extension,
            size,
            range,
            bitrate: bitrate.or(quality).or(mbps.map(|m| m * 1e6)),
            fps,
            gif_width,
            chapters: chapters.clone(),
            sidecar,
        })
    };
    match args.field("variants") {
        None => Ok(vec![output(
            args.optional_string("preset")?.unwrap_or("social-hevc"),
            None,
        )?]),
        Some(value) => {
            let list = value
                .as_array()
                .filter(|l| (1..=6).contains(&l.len()))
                .ok_or_else(|| bad("variants", "1 to 6 {aspect, preset} objects"))?;
            let outputs = list
                .iter()
                .map(|v| {
                    let preset = v.optional_string("preset")?.unwrap_or("social-h264");
                    output(preset, Some(v.string("aspect")?))
                })
                .collect::<Result<Vec<_>, _>>()?;
            // Files are named by size and container: two variants that share both would
            // write the same file (M12).
            for (index, a) in outputs.iter().enumerate() {
                if outputs[..index]
                    .iter()
                    .any(|b| a.size == b.size && a.extension == b.extension)
                {
                    return Err(CommandError::rule(
                        "BAD_ARGS",
                        "variants",
                        Some(value.clone()),
                        "variants that each make a different file (size and container)",
                        "Give each variant a different aspect, or export the other preset in a second export.start.",
                    )
                    .into());
                }
            }
            Ok(outputs)
        }
    }
}

/// Frame rates a video export can resample to (`export.start {fps}`).
const VIDEO_FPS: [u32; 5] = [24, 25, 30, 50, 60];
/// GIF frame rates (Recorder's 10, 15 and 24).
const GIF_FPS: [u32; 3] = [10, 15, 24];
/// Default GIF width in pixels; `width` picks another up to [`GIF_MAX_WIDTH`].
pub(super) const GIF_WIDTH: u32 = 480;
/// Widest GIF (Recorder's limit): a GIF's palette and size make anything larger a video's job.
pub(super) const GIF_MAX_WIDTH: u32 = 960;
const GIF_MIN_WIDTH: u32 = 16;

/// A GIF's default rate keeps evenly spaced frames: 15 fps at 30 or 60, every frame at 24 or
/// 25, every second at 50.
fn gif_rate(fps: u32) -> u32 {
    if fps.is_multiple_of(15) {
        15
    } else {
        fps / (fps / 25).max(1)
    }
}

/// An export bound: a whole frame inside the timeline, never clamped or wrapped.
fn range_frame(
    key: &str,
    value: Option<&Json>,
    default: Frame,
    end: Frame,
) -> Result<Frame, StoreError> {
    let Some(value) = value else {
        return Ok(default);
    };
    value
        .as_i64()
        .and_then(|frame| Frame::try_from(frame).ok())
        .filter(|frame| (0..=end).contains(frame))
        .ok_or_else(|| {
            CommandError::rule(
                "BAD_ARGS",
                key,
                Some(value.clone()),
                &format!("a whole frame from 0 to {end}"),
                &format!("Pass {key} as an integer frame inside the timeline (0 to {end})."),
            )
            .into()
        })
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
/// The project as rendered for `size`: the same timeline on a new canvas, spine clips and media
/// overlays reframed to fill it (largest centered crop) when the shape changes.
pub(super) fn reframed(project: &Project, size: [u32; 2]) -> Result<Option<Project>, StoreError> {
    if size == project.size {
        return Ok(None);
    }
    let mut copy = project.clone();
    let same_shape = u64::from(size[0]) * u64::from(project.size[1])
        == u64::from(size[1]) * u64::from(project.size[0]);
    copy.size = size;
    if same_shape {
        return Ok(Some(copy));
    }
    let mut items: Vec<Json> = copy
        .clips
        .order()
        .iter()
        .filter(|&&slot| {
            copy.clips
                .data(slot)
                .and_then(|d| d.media)
                .and_then(|m| copy.media.iter().find(|x| x.id == m))
                .is_some_and(|m| !matches!(m.kind, MediaKind::Audio | MediaKind::Compound))
        })
        .map(|&slot| Json::String(copy.clips.id(slot).expect("live clip").to_string()))
        .collect();
    items.extend(
        copy.items
            .order()
            .iter()
            .filter(|&&slot| {
                matches!(
                    copy.items.data(slot).map(|d| &d.kind),
                    Some(ItemKind::Media(_))
                )
            })
            .map(|&slot| Json::String(copy.items.id(slot).expect("live item").to_string())),
    );
    if items.is_empty() {
        return Ok(Some(copy));
    }
    copy.access = Access::Live;
    let op = Json::Object(vec![
        ("cmd".into(), Json::String("clip.crop".into())),
        ("items".into(), Json::Array(items)),
        ("aspect".into(), Json::String("canvas".into())),
    ]);
    splice_core::edit::apply(
        &mut copy,
        std::slice::from_ref(&op),
        splice_core::package::journal::Source::Human,
        false,
    )
    .map_err(|error| failure(error.error.code, &error.error.message, &error.error.fix))?;
    copy.access = project.access;
    Ok(Some(copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(args: &str) -> Box<splice_core::commands::Rule> {
        let args = splice_json::parse(args).unwrap();
        resolve(&Project::default(), 120, &args)
            .err()
            .and_then(|error| error.rule)
            .expect("a refusal with its rule")
    }

    #[test]
    fn m12_variants_that_would_share_a_file_are_refused() {
        let rule =
            refused(r#"{"variants":[{"aspect":"9:16"},{"aspect":"9:16","preset":"social-hevc"}]}"#);
        assert_eq!(rule.param, "variants");
    }

    #[test]
    fn low_export_bounds_are_whole_frames_inside_the_timeline() {
        assert_eq!(refused(r#"{"from":1.5}"#).param, "from");
        assert_eq!(refused(r#"{"to":4294967336}"#).param, "to");
        assert_eq!(refused(r#"{"to":121}"#).param, "to");
        assert_eq!(refused(r#"{"from":-1}"#).param, "from");
        let args = splice_json::parse(r#"{"from":0,"to":120}"#).unwrap();
        assert_eq!(
            resolve(&Project::default(), 120, &args).unwrap()[0].range,
            [0, 120]
        );
    }

    /// Recorder's GIF limits: 10, 15 or 24 fps, up to 960 px wide, any length (A4: nothing is
    /// cut short or refused for its length). Only what cannot be made is refused, with the
    /// limit in the fix.
    #[test]
    fn gifs_take_any_length_a_rate_and_a_width_up_to_960() {
        let project = Project::default();
        let resolved = |args: &str, end| resolve(&project, end, &splice_json::parse(args).unwrap());
        // Ten minutes at 30 fps.
        let out = resolved(r#"{"preset":"gif"}"#, 18_000).unwrap().remove(0);
        assert_eq!(
            (out.range, out.fps, out.gif_width),
            ([0, 18_000], 15, Some(GIF_WIDTH))
        );
        let out = resolved(r#"{"preset":"gif","fps":24,"width":960}"#, 600)
            .unwrap()
            .remove(0);
        assert_eq!((out.fps, out.gif_width), (24, Some(960)));
        for fps in [24, 25, 30, 50, 60] {
            let project = Project {
                fps,
                ..Project::default()
            };
            let rate = resolve(
                &project,
                600,
                &splice_json::parse(r#"{"preset":"gif"}"#).unwrap(),
            )
            .unwrap()[0]
                .fps;
            assert_eq!(
                fps % rate,
                0,
                "{fps} fps keeps evenly spaced frames ({rate})"
            );
        }
        let error = resolved(r#"{"preset":"gif","width":1920}"#, 600).unwrap_err();
        assert!(error.fix.contains("960"), "{}", error.fix);
        assert_eq!(error.rule.unwrap().param, "width");
        assert_eq!(refused(r#"{"preset":"gif","fps":30}"#).param, "fps");
        assert_eq!(refused(r#"{"preset":"social","width":480}"#).param, "width");
    }

    /// `export.start {fps}` resamples a video to 24, 25, 30, 50 or 60 fps on a project that
    /// already has clips; the bitrate follows the output rate. Audio and posters have no rate.
    #[test]
    fn video_exports_pick_an_output_frame_rate() {
        let project = Project::default();
        let out = |args: &str| {
            resolve(&project, 120, &splice_json::parse(args).unwrap())
                .unwrap()
                .remove(0)
        };
        assert_eq!(out(r#"{}"#).fps, project.fps);
        for fps in [24, 25, 30, 50, 60] {
            assert_eq!(out(&format!(r#"{{"fps":{fps}}}"#)).fps, fps);
        }
        let at = |fps: u32| {
            out(&format!(
                r#"{{"preset":"social-h264","quality":"social","fps":{fps}}}"#
            ))
            .bitrate
            .unwrap()
        };
        assert!((at(60) / at(30) - 2f64.sqrt()).abs() < 1e-9);
        assert_eq!(refused(r#"{"fps":29.97}"#).param, "fps");
        assert_eq!(refused(r#"{"fps":120}"#).param, "fps");
        assert_eq!(refused(r#"{"fps":10}"#).param, "fps");
        assert_eq!(refused(r#"{"preset":"audio-m4a","fps":30}"#).param, "fps");
        assert_eq!(refused(r#"{"preset":"poster","fps":30}"#).param, "fps");
    }

    #[test]
    fn presets_ranges_variants_and_reframing_resolve() {
        let mut project = Project::default();
        project.markers.push(Marker {
            id: MarkerId::from_serial(1),
            at: 30,
            name: "Intro".into(),
            color: None,
        });
        project.markers.push(Marker {
            id: MarkerId::from_serial(2),
            at: 90,
            name: "Demo".into(),
            color: None,
        });
        let args =
            splice_json::parse(r#"{"preset":"youtube-1080","from":10,"to":100,"captions":"srt"}"#)
                .unwrap();
        let out = resolve(&project, 120, &args).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].kind, out[0].size, out[0].range),
            (Kind::H264, [1920, 1080], [10, 100])
        );
        assert_eq!(out[0].bitrate, Some(16e6));
        assert_eq!(out[0].chapters, [(30, "Intro".into()), (90, "Demo".into())]);
        assert_eq!(out[0].sidecar, Some("srt"));
        let args = splice_json::parse(r#"{"marker":"mrk_1"}"#).unwrap();
        assert_eq!(resolve(&project, 120, &args).unwrap()[0].range, [30, 90]);
        let args = splice_json::parse(
            r#"{"variants":[{"aspect":"9:16"},{"aspect":"1:1","preset":"social-hevc"},{"aspect":"16:9"}]}"#,
        )
        .unwrap();
        let sizes: Vec<_> = resolve(&project, 120, &args)
            .unwrap()
            .iter()
            .map(|o| o.size)
            .collect();
        assert_eq!(sizes, [[1080, 1920], [1920, 1920], [1920, 1080]]);
        assert!(
            resolve(
                &project,
                120,
                &splice_json::parse(r#"{"preset":"vhs"}"#).unwrap()
            )
            .is_err()
        );
        assert_eq!(canvas([1080, 1920], "4:5").unwrap(), [1536, 1920]);
    }

    /// Recorder's export sheet: resolution is the shorter edge and keeps the output's shape;
    /// quality picks a bitrate for the size, rate and codec. Conflicts are refused, never ignored.
    #[test]
    fn resolution_keeps_the_shape_and_quality_scales_the_bitrate() {
        let project = Project {
            size: [1080, 1920],
            fps: 60,
            ..Project::default()
        };
        let resolved = |args: &str| resolve(&project, 120, &splice_json::parse(args).unwrap());
        let out = |args: &str| resolved(args).unwrap().remove(0);
        assert_eq!(out(r#"{"resolution":720}"#).size, [720, 1280]);
        assert_eq!(out(r#"{"resolution":2160}"#).size, [2160, 3840]);
        assert_eq!(
            out(r#"{"preset":"youtube-1080","resolution":720}"#).size,
            [1280, 720]
        );
        let sizes: Vec<_> =
            resolved(r#"{"resolution":1080,"variants":[{"aspect":"1:1"},{"aspect":"16:9"}]}"#)
                .unwrap()
                .iter()
                .map(|o| o.size)
                .collect();
        assert_eq!(sizes, [[1080, 1080], [1920, 1080]]);
        // 1080x1920 at 60 fps: 8 Mbps x 1 x sqrt(2); HEVC x 0.6.
        let h264 = out(r#"{"preset":"social-h264","quality":"social"}"#)
            .bitrate
            .unwrap();
        assert!((h264 - 8e6 * 2f64.sqrt()).abs() < 1.0, "{h264}");
        let hevc = out(r#"{"quality":"studio","resolution":720}"#)
            .bitrate
            .unwrap();
        let pixels = 720.0 * 1280.0 / (1920.0 * 1080.0);
        assert!(
            (hevc - 40e6 * pixels * 2f64.sqrt() * 0.6).abs() < 1.0,
            "{hevc}"
        );
        assert_eq!(refused(r#"{"resolution":1000}"#).param, "resolution");
        assert_eq!(refused(r#"{"resolution":"1080p"}"#).param, "resolution");
        assert_eq!(
            refused(r#"{"preset":"audio-wav","resolution":720}"#).param,
            "resolution"
        );
        assert_eq!(
            refused(r#"{"preset":"gif","resolution":720}"#).param,
            "resolution"
        );
        assert_eq!(refused(r#"{"quality":"best"}"#).param, "quality");
        assert_eq!(refused(r#"{"quality":"web","bitrate":4}"#).param, "quality");
        assert_eq!(
            refused(r#"{"preset":"prores-422","quality":"high"}"#).param,
            "quality"
        );
    }

    #[test]
    fn variants_reframe_pictures_to_fill_the_new_canvas() {
        let project = crate::tests::sample_project();
        assert!(reframed(&project, project.size).unwrap().is_none());
        let wide = reframed(&project, [1920, 1080]).unwrap().unwrap();
        assert_eq!(wide.size, [1920, 1080]);
        // Each reframed picture fills: its crop has the canvas's shape in source pixels.
        for (copy, ratio) in [
            (&wide, 16.0 / 9.0),
            (&reframed(&project, [1080, 1080]).unwrap().unwrap(), 1.0),
        ] {
            let slot = copy.clips.order()[1];
            let data = copy.clips.data(slot).unwrap();
            let media = copy
                .media
                .iter()
                .find(|m| Some(m.id) == data.media)
                .unwrap();
            let crop = data.framing.crop.unwrap();
            assert_eq!(data.framing.fit, Fit::Fill);
            let shape =
                crop.w * f64::from(media.probe.width) / (crop.h * f64::from(media.probe.height));
            assert!((shape - ratio).abs() < 0.01, "{shape} vs {ratio}");
        }
        // The original is untouched.
        assert_eq!(project.size, [1080, 1920]);
    }
}
