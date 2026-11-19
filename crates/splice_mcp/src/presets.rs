//! Export requests (PLAN.md T21.1–T21.2): presets, ranges, bitrate, chapters, caption sidecars
//! and aspect variants. Portable: the macOS job encodes what this resolves; Linux tests it.
use super::{Json, wire::*};
use splice_core::history::StoreError;
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

/// The canvas for an aspect, keeping the project's longer edge.
pub(super) fn canvas(size: [u32; 2], aspect: &str) -> Result<[u32; 2], StoreError> {
    let (w, h) = match aspect {
        "9:16" => (9.0, 16.0),
        "4:5" => (4.0, 5.0),
        "1:1" => (1.0, 1.0),
        "4:3" => (4.0, 3.0),
        "16:9" => (16.0, 9.0),
        _ => return Err(bad("aspect", "9:16, 4:5, 1:1, 4:3 or 16:9")),
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
            from.and_then(Json::as_i64)
                .map_or(0, |v| v as Frame)
                .clamp(0, end),
            to.and_then(Json::as_i64)
                .map_or(end, |v| v as Frame)
                .clamp(0, end),
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
        let size = match (aspect, fixed) {
            (Some(aspect), _) => canvas(fixed.unwrap_or(project.size), aspect)?,
            (None, Some(size)) => size,
            (None, None) => project.size,
        };
        Ok(Output {
            preset,
            kind,
            extension,
            size,
            range,
            bitrate: bitrate.or(mbps.map(|m| m * 1e6)),
            chapters: chapters.clone(),
            sidecar,
        })
    };
    match args.field("variants") {
        None => Ok(vec![output(
            args.optional_string("preset")?.unwrap_or("social-hevc"),
            None,
        )?]),
        Some(list) => {
            let list = list
                .as_array()
                .filter(|l| (1..=6).contains(&l.len()))
                .ok_or_else(|| bad("variants", "1 to 6 {aspect, preset} objects"))?;
            list.iter()
                .map(|v| {
                    let preset = v.optional_string("preset")?.unwrap_or("social-h264");
                    output(preset, Some(v.string("aspect")?))
                })
                .collect()
        }
    }
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
