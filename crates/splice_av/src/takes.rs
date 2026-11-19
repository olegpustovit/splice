//! Recorded take events for the renderer: the cursor track and the click-zoom path.
//! Parsed once per composition build, never per frame.
use crate::Error;
use apple_sys::composition::{CursorImage, CursorSample};
use splice_json::Value as Json;
use std::{fs, path::Path};

/// Cursor samples and images on the source clock, plus clicks and typed keys.
pub(crate) struct TakeEvents {
    /// Captured source width in screen points.
    pub points: f64,
    pub samples: Vec<CursorSample>,
    pub images: Vec<CursorImage>,
    /// (source seconds, x, y) normalized to the source.
    pub clicks: Vec<[f64; 3]>,
    /// (source seconds, typed text).
    pub keys: Vec<(f64, String)>,
}

pub(crate) fn load(events: &Path, cursors: Option<&Path>) -> Result<TakeEvents, Error> {
    let bad = |what: &str| Error::Input(format!("{}: {what}", events.display()));
    let text = fs::read_to_string(events).map_err(|e| bad(&e.to_string()))?;
    let root = splice_json::parse(&text).map_err(|e| bad(&e.to_string()))?;
    let points = root
        .field("source")
        .and_then(|s| s.field("rect"))
        .and_then(|r| r.field("width"))
        .and_then(Json::as_f64)
        .filter(|w| *w > 0.0)
        .ok_or_else(|| bad("missing source.rect.width"))?;
    let mut out = TakeEvents {
        points,
        samples: Vec::new(),
        images: Vec::new(),
        clicks: Vec::new(),
        keys: Vec::new(),
    };
    let mut ids: Vec<String> = Vec::new();
    let mut image = -1;
    let mut at: Option<[f64; 2]> = None;
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
                    push(&mut out.samples, t, x, y, image, 0);
                }
            }
            Some(kind @ ("move" | "drag" | "down" | "up")) => {
                let (Some(x), Some(y)) = (number("x"), number("y")) else {
                    continue;
                };
                at = Some([x, y]);
                let click = u32::from(kind == "down");
                push(&mut out.samples, t, x, y, image, click);
                if click == 1 {
                    out.clicks.push([t, x, y]);
                }
            }
            Some("key") => {
                if let Some(ch) = event.field("ch").and_then(Json::as_str)
                    && !ch.trim().is_empty()
                {
                    out.keys.push((t, ch.to_owned()));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn push(samples: &mut Vec<CursorSample>, t: f64, x: f64, y: f64, image: i32, click: u32) {
    // Events can arrive with equal or slightly earlier timestamps; keep the table ordered.
    let t = samples.last().map_or(t, |last| t.max(last.t));
    samples.push(CursorSample {
        t,
        x,
        y,
        image,
        click,
    });
}
