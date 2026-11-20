//! Recorded take events for the renderer: cursor samples, images and keys.
//! Parsed once per composition build, never per frame; splice_eval::cursor smooths them.
use crate::Error;
use apple_sys::composition::CursorImage;
use splice_eval::cursor::Sample;
use splice_json::Value as Json;
use std::{fs, path::Path};

/// Cursor samples and images on the source clock, plus typed keys.
pub(crate) struct TakeEvents {
    /// Captured source width and height in screen points.
    pub size: [f64; 2],
    pub samples: Vec<Sample>,
    pub images: Vec<CursorImage>,
    /// Keys that can make a chip, and modifier taps, in time order.
    pub keys: Vec<Key>,
}

/// A recorded key (or a modifier pressed alone) on the source clock.
pub(crate) struct Key {
    /// Source seconds (a tap: when the modifier went down).
    pub t: f64,
    /// Index among the log's key events (`keys.hide`); `None` for a modifier tap.
    pub index: Option<u32>,
    /// Typed text as recorded.
    pub text: String,
    /// macOS virtual key code, when the log has it.
    pub code: Option<u64>,
    /// Modifier flags (a tap: the modifiers pressed).
    pub mods: u64,
}

pub(crate) fn load(events: &Path, cursors: Option<&Path>) -> Result<TakeEvents, Error> {
    let bad = |what: &str| Error::Input(format!("{}: {what}", events.display()));
    let text = fs::read_to_string(events).map_err(|e| bad(&e.to_string()))?;
    let root = splice_json::parse(&text).map_err(|e| bad(&e.to_string()))?;
    let rect = |key| {
        root.field("source")
            .and_then(|s| s.field("rect"))
            .and_then(|r| r.field(key))
            .and_then(Json::as_f64)
            .filter(|v| *v > 0.0)
    };
    let width = rect("width").ok_or_else(|| bad("missing source.rect.width"))?;
    let mut out = TakeEvents {
        // Older logs carry only the width; assume a 16:10 display for the path math.
        size: [width, rect("height").unwrap_or(width / 1.6)],
        samples: Vec::new(),
        images: Vec::new(),
        keys: Vec::new(),
    };
    let mut ids: Vec<String> = Vec::new();
    let mut image = -1;
    let mut at: Option<[f64; 2]> = None;
    let mut key_index = 0;
    let mut taps = splice_eval::chips::ModifierTaps::default();
    for event in root.field("events").and_then(Json::as_array).unwrap_or(&[]) {
        let t = event.field("t").and_then(Json::as_f64).unwrap_or(-1.0);
        if !t.is_finite() || t < 0.0 {
            continue;
        }
        let number = |key| {
            event
                .field(key)
                .and_then(Json::as_f64)
                .filter(|v| v.is_finite())
        };
        match event.field("k").and_then(Json::as_str) {
            Some("cursor") => {
                let (Some(id), Some(dir)) = (event.field("id").and_then(Json::as_str), cursors)
                else {
                    continue;
                };
                image = match ids.iter().position(|known| known == id) {
                    Some(index) => index as i32,
                    None => {
                        let meta = fs::read_to_string(dir.join(format!("{id}.json")))
                            .ok()
                            .and_then(|meta| splice_json::parse(&meta).ok());
                        let get = |key| {
                            meta.as_ref()
                                .and_then(|m| m.field(key))
                                .and_then(Json::as_f64)
                        };
                        let png = dir.join(format!("{id}.png"));
                        if !png.is_file() {
                            continue;
                        }
                        ids.push(id.to_owned());
                        out.images.push(CursorImage {
                            path: png,
                            hot: [get("hotX").unwrap_or(0.0), get("hotY").unwrap_or(0.0)],
                            scale: get("scale").filter(|s| *s > 0.0).unwrap_or(1.0),
                        });
                        ids.len() as i32 - 1
                    }
                };
                if let Some([x, y]) = at {
                    push(&mut out.samples, t, x, y, image, false);
                }
            }
            Some(kind @ ("move" | "drag" | "down" | "up")) => {
                let (Some(x), Some(y)) = (number("x"), number("y")) else {
                    continue;
                };
                at = Some([x, y]);
                let click = kind == "down";
                push(&mut out.samples, t, x, y, image, click);
            }
            Some("key") => {
                let index = key_index;
                key_index += 1;
                taps.key();
                let ch = event.field("ch").and_then(Json::as_str).unwrap_or_default();
                let code = event.field("keyCode").and_then(Json::as_u64);
                if splice_eval::chips::visible(code, ch) {
                    out.keys.push(Key {
                        t,
                        index: Some(index),
                        text: ch.to_owned(),
                        code,
                        mods: event.field("mods").and_then(Json::as_u64).unwrap_or(0),
                    });
                }
            }
            Some("flags") => {
                let mods = event.field("mods").and_then(Json::as_u64).unwrap_or(0);
                if let Some((pressed, mods)) = taps.flags(t, mods) {
                    out.keys.push(Key {
                        t: pressed,
                        index: None,
                        text: String::new(),
                        code: None,
                        mods,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn push(samples: &mut Vec<Sample>, t: f64, x: f64, y: f64, image: i32, click: bool) {
    // Events can arrive with equal or slightly earlier timestamps; keep the table ordered.
    let t = samples.last().map_or(t, |last| t.max(last.t));
    samples.push(Sample {
        t,
        x,
        y,
        image,
        click,
    });
}
