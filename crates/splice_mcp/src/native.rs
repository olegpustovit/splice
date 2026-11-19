//! macOS implementations behind `media.analyze` and the 0.3 input fills (PLAN.md T18–T20).
use super::analyze::{
    Window, cache_dir, cached, component, label, onsets, rect_json, runs, shots, store_cache,
};
use super::{Json, object, wire::*};
use apple_sys::vision::{self, Finding};
use splice_core::history::StoreError;
use splice_model::*;
use std::path::Path;

fn native(error: apple_sys::Error) -> StoreError {
    failure(
        "NATIVE",
        &error.to_string(),
        "Check the media file is readable; retry once it is local.",
    )
}

const ENVELOPE_RATE: f64 = 100.0;

fn envelope(package: &Path, media: &Media) -> Result<Vec<f32>, StoreError> {
    let file = component(package, media, true);
    let dir = cache_dir(package, media.id);
    if let Some(body) = cached(&dir, "envelope.txt", &file) {
        return Ok(body.split(',').filter_map(|v| v.parse().ok()).collect());
    }
    let values = apple_sys::audio::envelope(&file, 0, ENVELOPE_RATE).map_err(native)?;
    let body: Vec<String> = values.iter().map(|v| format!("{v:.5}")).collect();
    store_cache(&dir, "envelope.txt", &file, &body.join(","));
    Ok(values)
}

fn findings(
    package: &Path,
    media: &Media,
    kinds: u32,
    every: f64,
    name: &str,
) -> Result<Vec<Finding>, StoreError> {
    let file = component(package, media, false);
    let dir = cache_dir(package, media.id);
    let body = match cached(&dir, name, &file) {
        Some(body) => body,
        None => {
            let found = vision::analyze(&file, kinds, every).map_err(native)?;
            let body = found
                .iter()
                .map(|f| match f {
                    Finding::Print { t, distance } => format!("print\t{t}\t{distance}"),
                    Finding::Text { t, rect, text } => format!(
                        "text\t{t}\t{}\t{}\t{}\t{}\t{text}",
                        rect[0], rect[1], rect[2], rect[3]
                    ),
                    Finding::Face { t, rect }
                    | Finding::Person { t, rect }
                    | Finding::Track { t, rect } => {
                        let kind = match f {
                            Finding::Face { .. } => "face",
                            Finding::Person { .. } => "person",
                            _ => "track",
                        };
                        format!(
                            "{kind}\t{t}\t{}\t{}\t{}\t{}",
                            rect[0], rect[1], rect[2], rect[3]
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            store_cache(&dir, name, &file, &body);
            return Ok(found);
        }
    };
    Ok(parse(&body))
}

fn parse(body: &str) -> Vec<Finding> {
    body.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let kind = f.next()?;
            let t: f64 = f.next()?.parse().ok()?;
            if kind == "print" {
                return Some(Finding::Print {
                    t,
                    distance: f.next()?.parse().ok()?,
                });
            }
            let mut rect = [0.0; 4];
            for slot in &mut rect {
                *slot = f.next()?.parse().ok()?;
            }
            Some(match kind {
                "text" => Finding::Text {
                    t,
                    rect,
                    text: f.next().unwrap_or("").to_owned(),
                },
                "face" => Finding::Face { t, rect },
                "person" => Finding::Person { t, rect },
                _ => Finding::Track { t, rect },
            })
        })
        .collect()
}

pub(super) fn analyze(
    package: &Path,
    media: &Media,
    window: &Window,
    kinds: &[&str],
) -> Result<Vec<(&'static str, Json)>, StoreError> {
    let mut out = Vec::new();
    let (from, to) = window.seconds();
    if kinds.iter().any(|k| matches!(*k, "peaks" | "beats")) {
        let values = envelope(package, media)?;
        if kinds.contains(&"peaks") {
            // 10 values per second over the clip, the loudest RMS in each tenth.
            let a = (from * ENVELOPE_RATE) as usize;
            let b = ((to * ENVELOPE_RATE) as usize).min(values.len());
            let peaks: Vec<Json> = values
                .get(a..b)
                .unwrap_or(&[])
                .chunks(10)
                .map(|c| {
                    Json::Float(f64::from(
                        (c.iter().copied().fold(0.0f32, f32::max) * 1000.0).round() / 1000.0,
                    ))
                })
                .collect();
            out.push((
                "peaks",
                object(vec![
                    ("rate", Json::Unsigned(10)),
                    ("values", Json::Array(peaks)),
                ]),
            ));
        }
        if kinds.contains(&"beats") {
            let beats: Vec<Json> = onsets(&values, ENVELOPE_RATE)
                .into_iter()
                .filter_map(|t| window.frame(t))
                .map(frame)
                .collect();
            out.push(("beats", Json::Array(beats)));
        }
    }
    if kinds.contains(&"sounds") {
        let file = component(package, media, true);
        let dir = cache_dir(package, media.id);
        let labels: Vec<(f64, f64, String)> = match cached(&dir, "sounds.txt", &file) {
            Some(body) => body
                .lines()
                .filter_map(|l| {
                    let mut f = l.split('\t');
                    Some((
                        f.next()?.parse().ok()?,
                        f.next()?.parse().ok()?,
                        f.next()?.to_owned(),
                    ))
                })
                .collect(),
            None => {
                std::fs::create_dir_all(&dir)
                    .map_err(|e| failure("IO", &e.to_string(), "Check disk space."))?;
                let sounds = apple_sys::audio::classify(&file, 0, &dir.join("classify.caf"))
                    .map_err(native)?;
                let labels: Vec<(f64, f64, String)> = sounds
                    .into_iter()
                    .map(|s| (s.start, s.duration, s.label))
                    .collect();
                let body: Vec<String> = labels
                    .iter()
                    .map(|(a, b, l)| format!("{a}\t{b}\t{l}"))
                    .collect();
                store_cache(&dir, "sounds.txt", &file, &body.join("\n"));
                labels
            }
        };
        let sounds: Vec<Json> = runs(&labels)
            .into_iter()
            .filter_map(|(a, b, l)| {
                let start = window.frame(a.max(from))?;
                let end = window.frame(b.min(to) - 1e-6).map_or(start + 1, |e| e + 1);
                Some(object(vec![
                    ("from", frame(start)),
                    ("to", frame(end)),
                    ("label", label(&l)),
                ]))
            })
            .collect();
        out.push(("sounds", Json::Array(sounds)));
    }
    let visual: u32 = kinds.iter().fold(0, |bits, k| {
        bits | match *k {
            "shots" => vision::SHOTS,
            "text" => vision::TEXT,
            "faces" => vision::FACES,
            "people" => vision::PEOPLE,
            _ => 0,
        }
    });
    if visual != 0 {
        // Shots sample every 0.25 s; text, faces and people every second.
        let mut found = Vec::new();
        if visual & vision::SHOTS != 0 {
            found.extend(findings(package, media, vision::SHOTS, 0.25, "shots.txt")?);
        }
        let rest = visual & !vision::SHOTS;
        if rest != 0 {
            found.extend(findings(
                package,
                media,
                vision::TEXT | vision::FACES | vision::PEOPLE,
                1.0,
                "frames.txt",
            )?);
        }
        if kinds.contains(&"shots") {
            let prints: Vec<(f64, f64)> = found
                .iter()
                .filter_map(|f| match f {
                    Finding::Print { t, distance } => Some((*t, *distance)),
                    _ => None,
                })
                .collect();
            let cuts: Vec<Json> = shots(&prints, 0.35)
                .into_iter()
                .filter_map(|t| window.frame(t))
                .map(frame)
                .collect();
            out.push(("shots", Json::Array(cuts)));
        }
        let mut seen: Vec<(String, [i32; 2])> = Vec::new();
        let (mut texts, mut faces, mut people) = (Vec::new(), Vec::new(), Vec::new());
        for finding in &found {
            match finding {
                Finding::Text { t, rect, text } if kinds.contains(&"text") => {
                    let key = (
                        text.clone(),
                        [(rect[0] * 50.0) as i32, (rect[1] * 50.0) as i32],
                    );
                    if let Some(at) = window.frame(*t)
                        && !seen.contains(&key)
                    {
                        seen.push(key);
                        texts.push(object(vec![
                            ("frame", frame(at)),
                            ("text", label(text)),
                            ("rect", rect_json(*rect)),
                        ]));
                    }
                }
                Finding::Face { t, rect } if kinds.contains(&"faces") => {
                    if let Some(at) = window.frame(*t) {
                        faces.push(object(vec![
                            ("frame", frame(at)),
                            ("rect", rect_json(*rect)),
                        ]));
                    }
                }
                Finding::Person { t, rect } if kinds.contains(&"people") => {
                    if let Some(at) = window.frame(*t) {
                        people.push(object(vec![
                            ("frame", frame(at)),
                            ("rect", rect_json(*rect)),
                        ]));
                    }
                }
                _ => {}
            }
        }
        for (name, list) in [("text", texts), ("faces", faces), ("people", people)] {
            if kinds.contains(&name) {
                out.push((name, Json::Array(list)));
            }
        }
    }
    if kinds.contains(&"matte") {
        let path = splice_av::matte_path(package, media.id);
        if !path.exists() {
            std::fs::create_dir_all(path.parent().expect("analysis dir"))
                .map_err(|e| failure("IO", &e.to_string(), "Check disk space."))?;
            vision::matte(&component(package, media, false), &path).map_err(native)?;
        }
        // A take's camera gets its own matte for the bubble's background.
        if let Some(camera) = media.take.as_ref().and_then(|take| take.camera.as_deref()) {
            let source = package.join(&media.path);
            let source = if source.is_dir() {
                source.join(camera)
            } else {
                source.with_file_name(camera)
            };
            let camera_path = splice_av::camera_matte_path(package, media.id);
            if source.is_file() && !camera_path.exists() {
                vision::matte(&source, &camera_path).map_err(native)?;
            }
        }
        out.push((
            "matte",
            Json::String(
                path.strip_prefix(package)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            ),
        ));
    }
    Ok(out)
}

fn field(fields: &[(String, Json)], key: &str) -> Option<String> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_str())
        .map(str::to_owned)
}

/// Tracks a mask's region through its owner's media and returns mask centers per owner frame.
pub(super) fn track_mask(
    dir: &Path,
    p: &Project,
    fields: &[(String, Json)],
) -> Result<Json, StoreError> {
    let mask: EffectId = field(fields, "mask")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| bad("mask", "a mask effect ID (fx_…)"))?;
    let (slot, effect) = p
        .clips
        .order()
        .iter()
        .find_map(|&slot| {
            let data = p.clips.data(slot)?;
            data.appearance
                .effects
                .iter()
                .find(|e| e.id == mask)
                .map(|e| (slot, e))
        })
        .ok_or_else(|| {
            failure(
                "NOT_FOUND",
                &format!("No mask {mask} on a spine clip."),
                "Track masks on spine clips.",
            )
        })?;
    let number = |name: &str, default: f64| {
        effect
            .params
            .iter()
            .find(|p| p.name == name)
            .map_or(default, |p| match &p.value {
                EffectValue::Number(n) => *n,
                EffectValue::Keys(k) => k.first().map_or(default, |k| k.value),
                _ => default,
            })
    };
    let rect = [
        number("x", 0.25),
        number("y", 0.25),
        number("w", 0.5),
        number("h", 0.5),
    ];
    let timing = p.clips.timing(slot).expect("live clip");
    let media = p
        .clips
        .data(slot)
        .and_then(|d| d.media)
        .and_then(|m| p.media.iter().find(|x| x.id == m))
        .ok_or_else(|| bad("mask", "a mask on a clip with media"))?;
    let rate = timing::source_rate(p, slot).unwrap_or(1.0);
    let window = Window {
        start: 0,
        length: timing.length,
        source: f64::from(timing.source),
        rate,
        fps: media.probe.fps,
    };
    let (from, to) = window.seconds();
    let found = vision::track(&component(dir, media, false), from, to, rect).map_err(native)?;
    let keys: Vec<Json> = found
        .iter()
        .filter_map(|f| match f {
            Finding::Track { t, rect } => {
                let at = window.frame(*t)?;
                Some(object(vec![
                    ("at", frame(at)),
                    ("x", Json::Float(rect[0] + rect[2] / 2.0)),
                    ("y", Json::Float(rect[1] + rect[3] / 2.0)),
                ]))
            }
            _ => None,
        })
        .collect();
    if keys.is_empty() {
        return Err(failure(
            "NOT_TRACKED",
            "The region could not be tracked.",
            "Pass keys explicitly.",
        ));
    }
    Ok(Json::Array(keys))
}

/// Source frames of `item` that line up with the first source frame of `to`, by envelope
/// cross-correlation over ±30 s.
pub(super) fn sync_offset(
    dir: &Path,
    p: &Project,
    fields: &[(String, Json)],
) -> Result<Frame, StoreError> {
    let item: ItemId = field(fields, "item")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| bad("item", "a sound or overlay ID"))?;
    let to: ClipId = field(fields, "to")
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| bad("to", "a spine clip ID"))?;
    let item_slot = p
        .items
        .find(item)
        .ok_or_else(|| bad("item", "an existing item"))?;
    let item_media = match &p.items.data(item_slot).expect("live item").kind {
        ItemKind::Sound(s) => s.media,
        ItemKind::Media(o) => o.media,
        _ => return Err(bad("item", "a sound or media overlay")),
    };
    let clip_slot = p
        .clips
        .find(to)
        .ok_or_else(|| bad("to", "an existing clip"))?;
    let clip_media = p
        .clips
        .data(clip_slot)
        .and_then(|d| d.media)
        .ok_or_else(|| bad("to", "a clip with media"))?;
    let find = |id: MediaId| {
        p.media
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| bad("media", "a known media"))
    };
    let (a_media, b_media) = (find(item_media)?, find(clip_media)?);
    let (a, b) = (envelope(dir, a_media)?, envelope(dir, b_media)?);
    let lag = super::analyze::best_lag(&a, &b, ENVELOPE_RATE);
    // Item time = clip media time + lag.
    let start = f64::from(p.clips.timing(clip_slot).expect("live clip").source) / b_media.probe.fps;
    let seconds = start + lag as f64 / ENVELOPE_RATE;
    Ok((seconds * a_media.probe.fps).round() as Frame)
}

/// Translates a captions lane on device; the language pack must be installed.
pub(super) fn translate_captions(
    p: &Project,
    fields: &[(String, Json)],
) -> Result<Json, StoreError> {
    let lane = field(fields, "lane").ok_or_else(|| bad("lane", "a captions lane ID"))?;
    let target = field(fields, "language").ok_or_else(|| bad("language", "a BCP 47 tag"))?;
    let cues = super::media::captions(
        p,
        &Json::Object(vec![("lane".into(), Json::String(lane))]),
        "json",
        None,
    )?;
    let texts: Vec<String> = cues
        .field("cues")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(|cue| cue.field("text").and_then(Json::as_str).map(str::to_owned))
        .collect();
    let source = p
        .transcripts
        .first()
        .map_or("en-US", |t| t.language.as_str());
    let translated = vision::translate(&texts, source, &target).map_err(|e| {
        failure(
            "NEEDS_TEXT",
            &format!("On-device translation failed: {e}"),
            "Install the language in System Settings ▸ General ▸ Language & Region ▸ Translation Languages, or pass texts.",
        )
    })?;
    Ok(Json::Array(
        translated.into_iter().map(Json::String).collect(),
    ))
}

/// Crop keys every second centered on the largest face (cached `media.analyze` faces).
pub(super) fn follow_face(
    dir: &Path,
    p: &Project,
    item: &str,
    aspect: Option<&str>,
) -> Result<Json, StoreError> {
    let id: ClipId = item.parse().map_err(|_| bad("item", "a clip"))?;
    let slot = p.clips.find(id).ok_or_else(|| bad("item", "a clip"))?;
    let timing = p.clips.timing(slot).expect("live clip");
    let media = p
        .clips
        .data(slot)
        .and_then(|d| d.media)
        .and_then(|m| p.media.iter().find(|x| x.id == m))
        .ok_or_else(|| bad("item", "a clip with video"))?;
    let (w, h) = super::enrich::crop_size(
        [media.probe.width, media.probe.height],
        super::enrich::aspect_ratio(p, aspect)?,
    );
    let window = Window {
        start: 0,
        length: timing.length,
        source: f64::from(timing.source),
        rate: timing::source_rate(p, slot).unwrap_or(1.0),
        fps: media.probe.fps,
    };
    let found = findings(
        dir,
        media,
        vision::TEXT | vision::FACES | vision::PEOPLE,
        1.0,
        "frames.txt",
    )?;
    let mut keys = Vec::new();
    for finding in &found {
        let Finding::Face { t, rect } = finding else {
            continue;
        };
        let Some(at) = window.frame(*t) else {
            continue;
        };
        let larger = found.iter().any(|other| {
            matches!(other, Finding::Face { t: t2, rect: r2 } if t2 == t && r2[2] * r2[3] > rect[2] * rect[3])
        });
        if larger || keys.iter().any(|k: &(Frame, f64, f64)| k.0 == at) {
            continue;
        }
        let (cx, cy) = (rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0);
        keys.push((
            at,
            (cx - w / 2.0).clamp(0.0, 1.0 - w),
            (cy - h / 2.0).clamp(0.0, 1.0 - h),
        ));
    }
    keys.sort_by_key(|k| k.0);
    if keys.is_empty() {
        return Err(failure(
            "NO_FACES",
            "No face was found in this clip.",
            "Use follow:\"cursor\" or pass keys.",
        ));
    }
    Ok(Json::Array(
        keys.into_iter()
            .map(|(at, x, y)| {
                object(vec![
                    ("at", frame(at)),
                    ("x", Json::Float((x * 1e4).round() / 1e4)),
                    ("y", Json::Float((y * 1e4).round() / 1e4)),
                    ("w", Json::Float((w * 1e4).floor() / 1e4)),
                    ("h", Json::Float((h * 1e4).floor() / 1e4)),
                    ("ease", Json::String("inOut".into())),
                ])
            })
            .collect(),
    ))
}
