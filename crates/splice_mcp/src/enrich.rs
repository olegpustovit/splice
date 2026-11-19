//! Fills the inputs of 0.3 commands that the core can't read itself (PLAN.md T20, T21): take
//! clicks for intent.autoZoom, probed lengths for media.relink, the file copies behind
//! media.consolidate, and (macOS) tracking, sync offsets and on-device translation. Batches that
//! need none of this pass through untouched.
use super::{Json, object, wire::*};
use splice_core::history::{Store, StoreError};

const FILLED: &[&str] = &[
    "intent.autoZoom",
    "media.relink",
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
                let clicks: Vec<Json> = events
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
                if clicks.is_empty() {
                    return Err(failure(
                        "NO_CLICKS",
                        "The take recorded no clicks inside this clip.",
                        "Pass clicks explicitly, or zoom with param.keys.",
                    ));
                }
                fields.push(("clicks".into(), Json::Array(clicks)));
            }
            "media.relink" if !has("frames") => {
                if let Some(path) = get("path")
                    && let Some(frames) = super::platform::probe_frames(store, &path)?
                {
                    fields.push(("frames".into(), Json::Signed(i64::from(frames))));
                }
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
