//! Recorded take events for the renderer: the cursor track and the click-zoom path.
//! Parsed once per composition build, never per frame.
use crate::Error;
use apple_sys::composition::{CursorImage, CursorSample};
use splice_eval::ClickZoom;
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

/// Seconds a zoom takes to move in, out or across.
const RAMP: f64 = 0.45;

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

/// Zoom keyframes on the source clock: (seconds, scale, focus x, focus y), focus normalized to the
/// source. Each click zooms in over `RAMP`, holds, and zooms out; clicks closer than a zoom-out
/// stay zoomed and pan instead (Recorder auto-zoom).
pub(crate) fn zoom_keys(clicks: &[[f64; 3]], zoom: ClickZoom, source_fps: f64) -> Vec<[f64; 4]> {
    let hold = zoom.hold / source_fps;
    let mut keys: Vec<[f64; 4]> = Vec::new();
    let mut index = 0;
    while index < clicks.len() {
        let first = clicks[index];
        let start = (first[0] - RAMP).max(keys.last().map_or(0.0, |k| k[0]));
        keys.push([start, 1.0, first[1], first[2]]);
        keys.push([first[0].max(start), zoom.scale, first[1], first[2]]);
        let mut last = first;
        while index + 1 < clicks.len() && clicks[index + 1][0] - RAMP < last[0] + hold + RAMP {
            let next = clicks[index + 1];
            let pan = (next[0] - RAMP).max(last[0]);
            keys.push([pan, zoom.scale, last[1], last[2]]);
            keys.push([next[0].max(pan), zoom.scale, next[1], next[2]]);
            last = next;
            index += 1;
        }
        keys.push([last[0] + hold, zoom.scale, last[1], last[2]]);
        keys.push([last[0] + hold + RAMP, 1.0, last[1], last[2]]);
        index += 1;
    }
    keys
}

/// Scale and focus at `t`, eased between keyframes.
pub(crate) fn zoom_at(keys: &[[f64; 4]], t: f64) -> [f64; 3] {
    let Some(after) = keys.iter().position(|k| k[0] > t) else {
        return keys.last().map_or([1.0, 0.5, 0.5], |k| [k[1], k[2], k[3]]);
    };
    if after == 0 {
        let k = keys[0];
        return [k[1], k[2], k[3]];
    }
    let (a, b) = (keys[after - 1], keys[after]);
    let x = ((t - a[0]) / (b[0] - a[0])).clamp(0.0, 1.0);
    let e = x * x * (3.0 - 2.0 * x);
    [
        a[1] + (b[1] - a[1]) * e,
        a[2] + (b[2] - a[2]) * e,
        a[3] + (b[3] - a[3]) * e,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keys_merge_close_clicks_and_return_to_one() {
        let zoom = ClickZoom {
            scale: 2.0,
            hold: 30.0,
        };
        let keys = zoom_keys(
            &[[1.0, 0.2, 0.2], [1.5, 0.8, 0.8], [6.0, 0.5, 0.5]],
            zoom,
            30.0,
        );
        // The first two clicks share one zoom that pans; the third zooms on its own.
        assert_eq!(keys.iter().filter(|k| k[1] == 1.0).count(), 4);
        assert!(keys.windows(2).all(|w| w[0][0] <= w[1][0]));
        assert_eq!(zoom_at(&keys, 0.0), [1.0, 0.2, 0.2]);
        assert_eq!(zoom_at(&keys, 1.0), [2.0, 0.2, 0.2]);
        assert_eq!(zoom_at(&keys, 1.5), [2.0, 0.8, 0.8]);
        assert_eq!(zoom_at(&keys, 4.0)[0], 1.0);
        let mid = zoom_at(&keys, 0.775);
        assert!(mid[0] > 1.0 && mid[0] < 2.0);
    }
}
