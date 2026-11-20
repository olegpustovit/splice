//! Fills the inputs of 0.3 commands that the core can't read itself (PLAN.md T20, T21): take
//! clicks for intent.autoZoom, take activity for intent.steps, intent.speedTyping and the
//! activity basis of the smart cuts (T41), cursor crops, and (macOS) tracking, sync offsets and
//! on-device translation, html.add's rendered movie; the music library's file and credit for
//! music.add (T45.3); and copies the files behind media.consolidate once a batch will commit. Every
//! edit path (MCP, `splice apply`, the viewer) runs it through `history::apply_batch`. Batches
//! that need none of this pass through untouched.
use super::{Json, object, wire::*};
use splice_core::commands::CommandError;
use splice_core::history::{Store, StoreError};
use splice_model::{Media, Project};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

fn cmd(op: &Json) -> &str {
    op.field("cmd").and_then(Json::as_str).unwrap_or("")
}

/// Whether `op` lacks an input `fill` computes.
fn needs(op: &Json) -> bool {
    let missing = |key: &str| op.field(key).is_none();
    match cmd(op) {
        "intent.autoZoom" => missing("clicks"),
        "intent.steps" => missing("activity"),
        "intent.speedTyping" => missing("typing"),
        // Filled when the basis is activity, or auto on a timeline without words (`fill`).
        "cut.pauses" | "intent.pace" | "intent.fitDuration" => {
            missing("activity") && op.field("basis").and_then(Json::as_str) != Some("transcript")
        }
        "crop.keys" => missing("keys") && op.field("follow").is_some(),
        "mask.track" => missing("keys"),
        "audio.sync" => missing("offset"),
        "captions.translate" => missing("texts"),
        "html.add" => missing("movie"),
        "music.add" => missing("path") && !missing("track"),
        "look.apply" => missing("effects") && missing("pack") && !missing("file"),
        _ => false,
    }
}

/// The batch with missing inputs filled, or `None` when nothing needed filling.
///
/// `$refs` in an op that needs filling resolve against the batch staged up to that op (M10), so
/// inputs of an object the batch itself creates (a split's right half, a new mask) are read from
/// the staged project. The filled op keeps its `$refs`: the commit mints its own IDs.
pub(super) fn ops(
    store: &Store,
    ops: &[Json],
    load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
) -> Result<Option<Vec<Json>>, StoreError> {
    if !ops.iter().any(needs) {
        return Ok(None);
    }
    let mut filled = ops.to_vec();
    for index in 0..filled.len() {
        if !needs(&filled[index]) {
            continue;
        }
        if cmd(&filled[index]) == "music.add" {
            let extra = super::music::fill(&super::music::library()?, &filled[index])?;
            if let Json::Object(fields) = &mut filled[index] {
                fields.retain(|(key, _)| !extra.iter().any(|(added, _)| added == key));
                fields.extend(extra);
            }
            continue;
        }
        if cmd(&filled[index]) == "look.apply" {
            let file = filled[index].field("file").and_then(Json::as_str);
            let effects = read_look(store.path(), file.unwrap_or(""))?;
            if let Json::Object(fields) = &mut filled[index] {
                fields.push(("effects".into(), effects));
            }
            continue;
        }
        let Json::Object(fields) = &filled[index] else {
            continue;
        };
        let refs: Vec<&str> = fields
            .iter()
            .filter(|(key, _)| key != "cmd" && key != "ref")
            .filter_map(|(_, value)| value.as_str())
            .filter(|value| value.starts_with('$'))
            .collect();
        let (project, targets) = if refs.is_empty() {
            (None, Vec::new())
        } else {
            let (project, targets) = store.preview(&filled[..index], &refs, load)?;
            (Some(project), targets)
        };
        // The op as the core will see it: each `$ref` replaced by the ID it resolves to.
        let resolved: Vec<(String, Json)> = fields
            .iter()
            .map(|(key, value)| {
                let target = refs
                    .iter()
                    .position(|name| value.as_str() == Some(name))
                    .filter(|_| key != "cmd" && key != "ref")
                    .map(|at| super::history::target(&targets[at]));
                (key.clone(), target.unwrap_or_else(|| value.clone()))
            })
            .collect();
        let project = project.as_ref().unwrap_or(store.project());
        let Some((key, value)) = fill(store.path(), project, &resolved)? else {
            continue;
        };
        let Json::Object(fields) = &mut filled[index] else {
            unreachable!("checked object")
        };
        if key == "keys" {
            fields.retain(|(name, _)| name != "follow" && name != "aspect");
        }
        fields.push((key.into(), value));
    }
    Ok(Some(filled))
}

/// The missing input of one op whose `$refs` are resolved, read from `project`; `None` when the
/// op needs none after all (an `auto` basis on a timeline with words).
fn fill(
    dir: &Path,
    project: &Project,
    fields: &[(String, Json)],
) -> Result<Option<(&'static str, Json)>, StoreError> {
    let get = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.as_str())
    };
    let command = get("cmd").unwrap_or("");
    match command {
        "intent.steps" | "intent.speedTyping" => {
            let fix = if command == "intent.steps" {
                "Pass activity:[{from, to}] busy source frames explicitly."
            } else {
                "Pass typing:[{from, to}] source frames explicitly."
            };
            let item = take_clip(get("item"), fix)?;
            let media = clip_media(project, item)?;
            let found = super::media::activity(dir, project, media, whole(media))?;
            let ranges = if command == "intent.steps" {
                &found.busy
            } else {
                &found.typing
            };
            let ranges = source_ranges(media, ranges, None);
            return Ok(Some((
                if command == "intent.steps" {
                    "activity"
                } else {
                    "typing"
                },
                Json::Array(ranges),
            )));
        }
        "cut.pauses" | "intent.pace" | "intent.fitDuration" => {
            let spoken = project.clips.order().iter().any(|&slot| {
                let media = project.clips.data(slot).and_then(|data| data.media);
                project.transcripts.iter().any(|transcript| {
                    Some(transcript.media) == media && !transcript.words.is_empty()
                })
            });
            if get("basis") != Some("activity") && spoken {
                return Ok(None);
            }
            // Every take on the spine, once.
            let mut ranges = Vec::new();
            let mut seen = Vec::new();
            for &slot in project.clips.order() {
                let Some(media) = project
                    .clips
                    .data(slot)
                    .and_then(|data| data.media)
                    .and_then(|id| project.media.iter().find(|media| media.id == id))
                    .filter(|media| media.kind == splice_model::MediaKind::Take)
                else {
                    continue;
                };
                if seen.contains(&media.id) {
                    continue;
                }
                seen.push(media.id);
                let found = super::media::activity(dir, project, media, whole(media))?;
                ranges.extend(source_ranges(media, &found.busy, Some(media.id)));
            }
            return Ok((!ranges.is_empty()).then_some(("activity", Json::Array(ranges))));
        }
        _ => {}
    }
    fill_input(dir, project, fields, command).map(Some)
}

/// A take's whole clock, in seconds.
fn whole(media: &Media) -> [f64; 2] {
    [
        0.0,
        f64::from(media.probe.frames) / media.probe.fps.max(1.0),
    ]
}

/// Second ranges as `[{media?, from, to}]` source frames, widened to whole frames.
fn source_ranges(
    media: &Media,
    ranges: &[[f64; 2]],
    id: Option<splice_model::MediaId>,
) -> Vec<Json> {
    let fps = media.probe.fps.max(1.0);
    ranges
        .iter()
        .map(|&[from, to]| {
            let mut fields = Vec::with_capacity(3);
            if let Some(id) = id {
                fields.push(("media", Json::String(id.to_string())));
            }
            let from = (from * fps).floor() as i64;
            fields.push(("from", Json::Signed(from)));
            fields.push(("to", Json::Signed(((to * fps).ceil() as i64).max(from + 1))));
            object(fields)
        })
        .collect()
}

/// The take behind a spine clip ID.
fn clip_media(project: &Project, item: String) -> Result<&Media, StoreError> {
    let wrong = || {
        StoreError::from(CommandError::rule(
            "WRONG_KIND",
            "item",
            Some(Json::String(item.clone())),
            "a spine clip (clp_…) of a recorded take",
            "Pass a take clip ID from timeline.snapshot.",
        ))
    };
    let id: splice_model::ClipId = item.parse().map_err(|_| wrong())?;
    project
        .clips
        .find(id)
        .and_then(|slot| project.clips.data(slot))
        .and_then(|data| data.media)
        .and_then(|id| project.media.iter().find(|media| media.id == id))
        .filter(|media| media.kind == splice_model::MediaKind::Take)
        .ok_or_else(wrong)
}

/// The inputs of the 0.3 rows: clicks, crop keys, tracking, sync offsets and translations.
fn fill_input(
    dir: &Path,
    project: &Project,
    fields: &[(String, Json)],
    command: &str,
) -> Result<(&'static str, Json), StoreError> {
    let get = |key: &str| {
        fields
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.as_str())
    };
    match command {
        "intent.autoZoom" => {
            let item = take_clip(
                get("item"),
                "Pass clicks explicitly, or place a zoom yourself: zoom.add {at, length, amount, focus:{x, y}} on what matters (view.frames shows where).",
            )?;
            let events =
                super::media::events(dir, project, &object(vec![("clip", Json::String(item))]))?;
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
                    "Pass clicks explicitly, or place a zoom yourself: zoom.add {at, length, amount, focus:{x, y}} on what matters (view.frames shows where).",
                ));
            }
            Ok(("clicks", Json::Array(clicks)))
        }
        "crop.keys" => {
            let item = take_clip(get("item"), "Pass keys explicitly to animate this crop.")?;
            let keys = match get("follow") {
                Some("cursor") => follow_cursor(dir, project, &item, get("aspect"))?,
                Some("face") => super::platform::follow_face(dir, project, &item, get("aspect"))?,
                _ => return Err(bad("follow", "cursor or face")),
            };
            Ok(("keys", keys))
        }
        "mask.track" => Ok(("keys", super::platform::track_mask(dir, project, fields)?)),
        "audio.sync" => {
            let offset = super::platform::sync_offset(dir, project, fields)?;
            Ok(("offset", Json::Signed(i64::from(offset))))
        }
        "captions.translate" => Ok((
            "texts",
            super::platform::translate_captions(project, fields)?,
        )),
        "html.add" => Ok(("movie", super::platform::render_html(dir, project, fields)?)),
        _ => unreachable!("needs() selects the filled commands"),
    }
}

/// Clicks, cursor and faces come from a spine clip's media: an overlay or anything else is
/// refused with the rule it broke instead of being skipped.
fn take_clip(item: Option<&str>, fix: &str) -> Result<String, StoreError> {
    match item {
        Some(id) if id.starts_with("clp_") => Ok(id.into()),
        _ => Err(CommandError::rule(
            "WRONG_KIND",
            "item",
            item.map(|id| Json::String(id.into())),
            "a spine clip (clp_…) with recorded events",
            fix,
        )
        .into()),
    }
}

/// `media.consolidate` (M13): copies external media into `<package>/media/` where the staged
/// batch points it. Run only for a batch that will commit. An existing destination is kept
/// only when its bytes match the source; otherwise a fresh copy replaces it atomically.
pub(super) fn consolidate(
    store: &Store,
    ops: &[Json],
    load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
) -> Result<(), StoreError> {
    let (staged, _) = store.preview(ops, &[], load)?;
    for before in &store.project().media {
        let Some(after) = staged.media.iter().find(|media| media.id == before.id) else {
            continue;
        };
        if !before.path.starts_with('/')
            || after.path != splice_core::edit::consolidated_path(before.id, &before.path)
        {
            continue;
        }
        let to = store.path().join(&after.path);
        let dir = to.parent().unwrap_or(store.path());
        fs::create_dir_all(dir).map_err(|error| io_failure(dir, &error))?;
        copy_verified(Path::new(&before.path), &to)?;
    }
    Ok(())
}

/// A look file's path: relative paths resolve in the package.
fn look_path(package: &Path, file: &str) -> std::path::PathBuf {
    let path = Path::new(file);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        package.join(path)
    }
}

/// The `effects` of a look file `look.save` wrote (T46.3), for `look.apply {file}`.
fn read_look(package: &Path, file: &str) -> Result<Json, StoreError> {
    let path = look_path(package, file);
    let unreadable = |why: &str| {
        StoreError::from(CommandError::rule(
            "BAD_LOOK",
            "file",
            Some(Json::String(file.into())),
            "a look file look.save wrote ({\"splice\":\"look\",\"version\":1,\"effects\":{…}})",
            &format!(
                "{} {why}. Save one with look.save {{target, file}}, or apply a pack.",
                path.display()
            ),
        ))
    };
    let text = fs::read_to_string(&path).map_err(|error| unreadable(&error.to_string()))?;
    let value = splice_json::parse(&text).map_err(|_| unreadable("is not JSON"))?;
    if value.field("splice").and_then(Json::as_str) != Some("look")
        || value.field("version").and_then(Json::as_i64) != Some(1)
    {
        return Err(unreadable("is not a version 1 look"));
    }
    value
        .field("effects")
        .cloned()
        .ok_or_else(|| unreadable("has no effects"))
}

/// `look.save` (T46.3): writes each op's look file from the batch staged up to it. Run only for
/// a batch that will commit.
pub(super) fn save_looks(
    store: &Store,
    ops: &[Json],
    load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
) -> Result<(), StoreError> {
    for (index, op) in ops.iter().enumerate() {
        if cmd(op) != "look.save" {
            continue;
        }
        let target = op.field("target").and_then(Json::as_str).unwrap_or("");
        let refs: Vec<&str> = [target]
            .into_iter()
            .filter(|t| t.starts_with('$'))
            .collect();
        let (staged, targets) = store.preview(&ops[..=index], &refs, load)?;
        let target = match targets.into_iter().next() {
            Some(target) => target,
            None => match splice_core::commands::decode(op)?.op {
                splice_core::commands::Op::LookSave { target, .. } => target,
                _ => unreachable!("a look.save op"),
            },
        };
        let look = splice_core::edit::saved_look(&staged, &target, store.path())?;
        let file = op.field("file").and_then(Json::as_str).unwrap_or("");
        let to = look_path(store.path(), file);
        let mut text = String::new();
        splice_json::write(&look, &mut text).expect("finite look");
        text.push('\n');
        let dir = to.parent().unwrap_or(store.path());
        fs::create_dir_all(dir).map_err(|error| write_failure(dir, &error))?;
        let mut partial = to.as_os_str().to_owned();
        partial.push(".partial");
        let partial = Path::new(&partial);
        fs::write(partial, text)
            .and_then(|()| fs::rename(partial, &to))
            .map_err(|error| {
                let _ = fs::remove_file(partial);
                write_failure(&to, &error)
            })?;
    }
    Ok(())
}

fn write_failure(path: &Path, error: &io::Error) -> StoreError {
    failure(
        "IO",
        &format!("Could not write {}: {error}.", path.display()),
        "Pick a writable path for file (relative paths land in the package).",
    )
}

fn copy_verified(from: &Path, to: &Path) -> Result<(), StoreError> {
    if to.exists() && same_bytes(from, to).map_err(|error| io_failure(from, &error))? {
        return Ok(());
    }
    // Copy beside the destination, then rename: a crash never leaves a partial file at the
    // name the project trusts.
    let mut partial = to.as_os_str().to_owned();
    partial.push(".partial");
    let partial = Path::new(&partial);
    fs::copy(from, partial)
        .and_then(|_| File::open(partial)?.sync_all())
        .and_then(|()| fs::rename(partial, to))
        .map_err(|error| {
            let _ = fs::remove_file(partial);
            io_failure(to, &error)
        })
}

fn same_bytes(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (File::open(a)?, File::open(b)?);
    let (mut x, mut y) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = a.read(&mut x)?;
        if n == 0 {
            return Ok(true);
        }
        b.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

fn io_failure(path: &Path, error: &io::Error) -> StoreError {
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
        let frame = |c: &Json| c.field("sourceFrame").and_then(Json::as_i64);
        let latest = |kinds: &[&str]| {
            kinds
                .iter()
                .flat_map(|kind| list(kind))
                .filter(|c| frame(c).is_some_and(|f| f <= at))
                .max_by_key(|c| frame(c))
        };
        // Typing lands at the text caret (T45.4), unless a click since its last sample moved
        // the focus; without caret samples, where the pointer last was.
        let click = latest(&["clicks"]).and_then(frame);
        let caret = latest(&["caret"]).filter(|caret| frame(caret) >= click);
        let point = match caret {
            Some(caret) => {
                let number = |key: &str| caret.field(key).and_then(Json::as_f64);
                match (number("x"), number("y"), number("w"), number("h")) {
                    (Some(x), Some(y), Some(w), Some(h)) => Some((
                        Json::Float((x + w / 2.0).clamp(0.0, 1.0)),
                        Json::Float((y + h / 2.0).clamp(0.0, 1.0)),
                    )),
                    _ => None,
                }
            }
            None => latest(&["cursor", "clicks"]).and_then(|pointer| {
                Some((pointer.field("x")?.clone(), pointer.field("y")?.clone()))
            }),
        };
        if let Some((x, y)) = point {
            out.push(object(vec![("at", Json::Signed(at)), ("x", x), ("y", y)]));
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

pub(super) fn aspect_ratio(project: &Project, aspect: Option<&str>) -> Result<f64, StoreError> {
    Ok(match aspect {
        None => {
            let [w, h] = project.size;
            f64::from(w) / f64::from(h)
        }
        Some(name) => {
            let size = super::presets::canvas([1000, 1000], name)?;
            f64::from(size[0]) / f64::from(size[1])
        }
    })
}

/// Crop keys every half second that keep a smoothed pointer inside the frame (T20.2).
fn follow_cursor(
    dir: &Path,
    p: &Project,
    item: &str,
    aspect: Option<&str>,
) -> Result<Json, StoreError> {
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
        aspect_ratio(p, aspect)?,
    );
    let events = super::media::events(
        dir,
        p,
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

#[cfg(test)]
#[path = "look_tests.rs"]
mod look_tests;
