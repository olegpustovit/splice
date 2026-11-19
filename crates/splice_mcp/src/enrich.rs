//! Fills the inputs of 0.3 commands that the core can't read itself (PLAN.md T20, T21): take
//! clicks for intent.autoZoom, the file copies behind
//! media.consolidate, and (macOS) tracking, sync offsets and on-device translation. Batches that
//! need none of this pass through untouched.
use super::{Json, object, wire::*};
use splice_core::history::{Store, StoreError};

const FILLED: &[&str] = &[
    "crop.keys",
    "intent.autoZoom",
    "media.consolidate",
    "mask.track",
    "audio.sync",
    "captions.translate",
];

fn cmd(op: &Json) -> &str {
    op.field("cmd").and_then(Json::as_str).unwrap_or("")
}

/// The batch with missing inputs filled, or `None` when nothing needed filling.
pub(super) fn ops(store: &Store, ops: &[Json]) -> Result<Option<Vec<Json>>, StoreError> {
    if !ops.iter().any(|op| FILLED.contains(&cmd(op))) {
        return Ok(None);
    }
    let mut filled = ops.to_vec();
    for op in &mut filled {
        let Json::Object(fields) = op else { continue };
        let has = |key: &str| fields.iter().any(|(name, _)| name == key);
        let get = |key: &str| {
            fields
                .iter()
                .find(|(name, _)| name == key)
                .and_then(|(_, value)| value.as_str())
                .map(str::to_owned)
        };
        let command = fields
            .iter()
            .find(|(name, _)| name == "cmd")
            .and_then(|(_, value)| value.as_str())
            .unwrap_or("")
            .to_owned();
        match command.as_str() {
            "intent.autoZoom" if !has("clicks") => {
                let Some(item) = get("item").filter(|id| id.starts_with("clp_")) else {
                    continue;
                };
                let events = super::media::events(
                    store,
                    &Json::Object(vec![("clip".into(), Json::String(item))]),
                )?;
                let mut clicks: Vec<Json> = events
                    .field("clicks")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|click| {
                        Some(object(vec![
                            ("at", click.field("sourceFrame")?.clone()),
                            ("x", click.field("x")?.clone()),
                            ("y", click.field("y")?.clone()),
                        ]))
                    })
                    .collect();
                // Typing bursts zoom too, where the pointer last was (T12.3 open item).
                clicks.extend(typing_bursts(&events));
                clicks.sort_by_key(|c| c.field("at").and_then(Json::as_i64).unwrap_or(0));
                if clicks.is_empty() {
                    return Err(failure(
                        "NO_CLICKS",
                        "The take recorded no clicks inside this clip.",
                        "Pass clicks explicitly, or zoom with param.keys.",
                    ));
                }
                fields.push(("clicks".into(), Json::Array(clicks)));
            }
            "crop.keys" if !has("keys") => {
                let Some(item) = get("item").filter(|id| id.starts_with("clp_")) else {
                    continue;
                };
                let keys = match get("follow").as_deref() {
                    Some("cursor") => follow_cursor(store, &item, get("aspect").as_deref())?,
                    Some("face") => {
                        super::platform::follow_face(store, &item, get("aspect").as_deref())?
                    }
                    _ => continue,
                };
                fields.retain(|(name, _)| name != "follow" && name != "aspect");
                fields.push(("keys".into(), keys));
            }
            "media.consolidate" => consolidate(store, fields)?,
            "mask.track" if !has("keys") => {
                let keys = super::platform::track_mask(store, fields)?;
                fields.push(("keys".into(), keys));
            }
            "audio.sync" if !has("offset") => {
                let offset = super::platform::sync_offset(store, fields)?;
                fields.push(("offset".into(), Json::Signed(i64::from(offset))));
            }
            "captions.translate" if !has("texts") => {
                let texts = super::platform::translate_captions(store, fields)?;
                fields.push(("texts".into(), texts));
            }
            _ => {}
        }
    }
    Ok(Some(filled))
}

/// Copies external media into `<package>/media/` where the core's media.consolidate points it.
fn consolidate(store: &Store, fields: &[(String, Json)]) -> Result<(), StoreError> {
    let wanted: Vec<&str> = fields
        .iter()
        .find(|(name, _)| name == "media")
        .map(|(_, value)| match value {
            Json::Array(list) => list.iter().filter_map(Json::as_str).collect(),
            Json::String(one) => vec![one.as_str()],
            _ => Vec::new(),
        })
        .unwrap_or_default();
    let dir = store.path().join("media");
    for media in &store.project().media {
        let id = media.id.to_string();
        if (!wanted.is_empty() && !wanted.contains(&id.as_str())) || !media.path.starts_with('/') {
            continue;
        }
        std::fs::create_dir_all(&dir).map_err(|error| io_failure(&dir, &error))?;
        let to = store
            .path()
            .join(splice_core::edit::consolidated_path(media.id, &media.path));
        if !to.exists() {
            std::fs::copy(&media.path, &to).map_err(|error| io_failure(&to, &error))?;
        }
    }
    Ok(())
}

fn io_failure(path: &std::path::Path, error: &std::io::Error) -> StoreError {
    failure(
        "IO",
        &format!("Could not copy media to {}: {error}.", path.display()),
        "Check that the source file is readable and the disk has room.",
    )
}

/// Key presses that start after a 0.6 s lull, as clicks at the pointer's last position.
fn typing_bursts(events: &Json) -> Vec<Json> {
    let list = |key: &str| -> &[Json] { events.field(key).and_then(Json::as_array).unwrap_or(&[]) };
    let mut out = Vec::new();
    let mut last_key: Option<i64> = None;
    for key in list("keys") {
        let Some(at) = key.field("sourceFrame").and_then(Json::as_i64) else {
            continue;
        };
        let lull = last_key.is_none_or(|previous| at - previous > 36);
        last_key = Some(at);
        if !lull {
            continue;
        }
        let pointer = list("cursor")
            .iter()
            .chain(list("clicks"))
            .filter(|c| {
                c.field("sourceFrame")
                    .and_then(Json::as_i64)
                    .is_some_and(|f| f <= at)
            })
            .max_by_key(|c| c.field("sourceFrame").and_then(Json::as_i64));
        if let Some(pointer) = pointer
            && let (Some(x), Some(y)) = (pointer.field("x"), pointer.field("y"))
        {
            out.push(object(vec![
                ("at", Json::Signed(at)),
                ("x", x.clone()),
                ("y", y.clone()),
            ]));
        }
    }
    out
}

/// The crop's normalized size inside a `source` of the given shape for a target `aspect`.
pub(super) fn crop_size(source: [u32; 2], aspect: f64) -> (f64, f64) {
    let ratio = f64::from(source[0]) / f64::from(source[1].max(1));
    if ratio > aspect {
        (aspect / ratio, 1.0)
    } else {
        (1.0, ratio / aspect)
    }
}

pub(super) fn aspect_ratio(store: &Store, aspect: Option<&str>) -> Result<f64, StoreError> {
    Ok(match aspect {
        None => {
            let [w, h] = store.project().size;
            f64::from(w) / f64::from(h)
        }
        Some(name) => {
            let size = super::presets::canvas([1000, 1000], name)?;
            f64::from(size[0]) / f64::from(size[1])
        }
    })
}

/// Crop keys every half second that keep a smoothed pointer inside the frame (T20.2).
fn follow_cursor(store: &Store, item: &str, aspect: Option<&str>) -> Result<Json, StoreError> {
    let p = store.project();
    let id: splice_model::ClipId = item.parse().map_err(|_| bad("item", "a take clip"))?;
    let slot = p.clips.find(id).ok_or_else(|| bad("item", "a take clip"))?;
    let media = p
        .clips
        .data(slot)
        .and_then(|d| d.media)
        .and_then(|m| p.media.iter().find(|x| x.id == m))
        .ok_or_else(|| bad("item", "a take clip"))?;
    let (w, h) = crop_size(
        [media.probe.width, media.probe.height],
        aspect_ratio(store, aspect)?,
    );
    let events = super::media::events(
        store,
        &Json::Object(vec![("clip".into(), Json::String(item.into()))]),
    )?;
    let start = p.clips.timing(slot).expect("live clip").start;
    let length = p.clips.timing(slot).expect("live clip").length;
    let mut points: Vec<(i64, f64, f64)> = events
        .field("cursor")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .chain(
            events
                .field("clicks")
                .and_then(Json::as_array)
                .into_iter()
                .flatten(),
        )
        .filter_map(|c| {
            Some((
                c.field("frame")?.as_i64()? - i64::from(start),
                c.field("x")?.as_f64()?,
                c.field("y")?.as_f64()?,
            ))
        })
        .collect();
    points.sort_by_key(|p| p.0);
    let mut keys = Vec::new();
    let (mut cx, mut cy) = (0.5, 0.5);
    let mut index = 0;
    let step = (p.fps / 2).max(1) as i64;
    let mut at = 0i64;
    while at < i64::from(length) {
        while index + 1 < points.len() && points[index + 1].0 <= at {
            index += 1;
        }
        if let Some(&(_, x, y)) = points.get(index).filter(|p| p.0 <= at) {
            // Move only when the pointer nears the crop edge; then glide halfway there.
            if (x - cx).abs() > w * 0.3 {
                cx += (x - cx) * 0.5;
            }
            if (y - cy).abs() > h * 0.3 {
                cy += (y - cy) * 0.5;
            }
        }
        keys.push(object(vec![
            ("at", Json::Signed(at)),
            (
                "x",
                Json::Float(((cx - w / 2.0).clamp(0.0, 1.0 - w) * 1e4).round() / 1e4),
            ),
            (
                "y",
                Json::Float(((cy - h / 2.0).clamp(0.0, 1.0 - h) * 1e4).round() / 1e4),
            ),
            ("w", Json::Float((w * 1e4).floor() / 1e4)),
            ("h", Json::Float((h * 1e4).floor() / 1e4)),
            ("ease", Json::String("inOut".into())),
        ]));
        at += step;
    }
    Ok(Json::Array(keys))
}
