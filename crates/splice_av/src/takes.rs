//! Recorded take events for the renderer: cursor samples, images and keys.
//! Parsed once per composition build, never per frame; splice_eval::cursor smooths them.
use crate::Error;
use apple_sys::composition::CursorImage;
use splice_eval::{Keys, SourceSpan, chips, cursor::Sample};
use splice_json::Value as Json;
use splice_model::Frame;
use std::{collections::BTreeMap, fs, path::Path};

/// Recorded input events for the editor timeline, sorted by source seconds.
#[derive(Default)]
pub struct TimelineEvents {
    /// Source frames per second, used to convert the clip's source trim.
    pub fps: f64,
    /// Timestamps of recorded pointer presses.
    pub clicks: Vec<f64>,
    /// Displayable keys and modifier taps.
    pub keys: Vec<TimelineKey>,
    /// Longest authored key dwell in source seconds for timeline range queries.
    pub key_lookback: f64,
}

/// One redacted key or modifier tap from a recorded take.
pub struct TimelineKey {
    /// Source seconds when the key was pressed.
    pub t: f64,
    /// Event index used by the clip's hidden-key list.
    pub index: Option<u32>,
    /// Category mask bit used by the clip's keystroke settings.
    pub category: u8,
    /// Display text with typed characters redacted.
    pub label: String,
    /// Exclusive source timestamp of an authored chip.
    pub chip_until: Option<f64>,
}

impl TimelineKey {
    /// Frames where this authored chip intersects the clip's sampled source clock.
    pub fn authored_frames(
        &self,
        span: SourceSpan,
        speed_map: &[f64],
        fps: f64,
    ) -> Option<[Frame; 2]> {
        let end = self.chip_until?;
        Some(chip_frames(span, speed_map, fps, [self.t, end]))
    }
}

/// Decode timeline input from the same recorded log used by native playback.
pub fn load_timeline_events(path: &Path, fps: f64) -> Result<TimelineEvents, Error> {
    let take = load(path, None)?;
    let clicks = take
        .samples
        .iter()
        .filter(|sample| sample.click)
        .map(|sample| sample.t)
        .collect();
    let mut keys = take
        .keys
        .into_iter()
        .filter_map(|key| {
            let tap = key.index.is_none();
            let label = key.chip_text(true);
            let category = if tap {
                chips::MODIFIERS
            } else {
                chips::category(key.code, key.mods, &key.text)
            };
            (!label.trim().is_empty()).then_some(TimelineKey {
                t: key.t,
                index: key.index,
                category,
                label,
                chip_until: key.chip_until,
            })
        })
        .collect::<Vec<_>>();
    keys.sort_by(|left, right| left.t.total_cmp(&right.t));
    let key_lookback = keys
        .iter()
        .filter_map(|key| key.chip_until.map(|end| end - key.t))
        .fold(0.0_f64, f64::max);
    Ok(TimelineEvents {
        fps,
        clicks,
        keys,
        key_lookback,
    })
}

/// Resolve whether click rings are active and which key categories are visible.
/// Scope order is project, spine, then clip. Call once per project revision.
pub fn timeline_event_visibility(scopes: &[&[splice_model::Effect]]) -> (bool, u8) {
    use splice_model::EffectValue;
    let rings = splice_eval::resolve_effect(scopes, "cursor").is_some_and(|(enabled, params)| {
        enabled
            && !params
                .iter()
                .any(|p| p.name == "hidden" && p.value == EffectValue::Bool(true))
            && !params
                .iter()
                .any(|p| p.name == "clickRings" && p.value == EffectValue::Bool(false))
    });
    (rings, splice_eval::chips::shown(scopes))
}

/// Cursor samples and images on the source clock, plus typed keys.
pub(crate) struct TakeEvents {
    /// Captured source width and height in screen points.
    pub size: [f64; 2],
    pub samples: Vec<Sample>,
    pub images: Vec<CursorImage>,
    /// Keys that can make a chip, and modifier taps, in log order.
    keys: Vec<Key>,
}

/// A recorded key (or a modifier pressed alone) on the source clock.
struct Key {
    /// Source seconds (a tap: when the modifier went down).
    t: f64,
    /// Exclusive source timestamp for an authored chip dwell.
    chip_until: Option<f64>,
    /// Index among the log's key events (`keys.hide`); `None` for a modifier tap.
    index: Option<u32>,
    /// Typed text as recorded.
    text: String,
    /// macOS virtual key code, when the log has it.
    code: Option<u64>,
    /// Modifier flags (a tap: the modifiers pressed).
    mods: u64,
}

impl Key {
    fn chip_text(&self, redact: bool) -> String {
        if self.chip_until.is_some() {
            chips::authored_chip_text(&self.text, self.code, self.mods, redact)
        } else {
            chips::chip_text(
                &self.text,
                self.code,
                self.mods,
                self.index.is_none(),
                redact,
            )
        }
    }
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
                        chip_until: number("chipUntil").filter(|&end| t < end && end <= t + 5.0),
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
                        chip_until: None,
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

impl TakeEvents {
    /// Prepares visible chip intervals once. Explicit source dwells take precedence over
    /// Recorder bursts; the latter keep their original hold, joining and draw order.
    pub(crate) fn chip_cues(
        &self,
        style: Keys,
        hidden: &[u32],
        span: SourceSpan,
        speed_map: &[f64],
        fps: f64,
        redact: bool,
    ) -> Vec<([Frame; 2], String)> {
        let join = style.hold.min(0.6);
        // (first key, last key, text, last category)
        let mut bursts: Vec<(f64, f64, String, u8)> = Vec::new();
        let mut dwells = Vec::new();
        for key in &self.keys {
            let category = match key.index {
                None => chips::MODIFIERS,
                Some(_) => chips::category(key.code, key.mods, &key.text),
            };
            if key.index.is_some_and(|n| hidden.contains(&n)) || style.show & category == 0 {
                continue;
            }
            let text = key.chip_text(redact);
            if let Some(end) = key.chip_until {
                let range = chip_frames(span, speed_map, fps, [key.t, end]);
                if range[0] < range[1] {
                    dwells.push((range, text));
                }
                continue;
            }
            match bursts.last_mut() {
                Some(last) if key.t - last.1 < join && last.2.chars().count() < 24 => {
                    last.1 = key.t;
                    // Typing runs together; shortcuts and glyphs stand apart (`⌘C ⌘V`).
                    if (category != chips::TYPING || last.3 != chips::TYPING)
                        && !last.2.ends_with(' ')
                    {
                        last.2.push(' ');
                    }
                    last.2.push_str(&text);
                    last.3 = category;
                }
                _ => bursts.push((key.t, key.t, text, category)),
            }
        }

        // Recorder retains its rounded constant-rate placement and minimum cue length.
        let frame =
            |t: f64| (f64::from(span.start) + (t * fps - span.source) / span.step).round() as Frame;
        let recorder = bursts.into_iter().filter_map(|(start, end, text, _)| {
            let from = frame(start).max(span.start);
            let to = frame(end + style.hold).min(span.end);
            (to.saturating_sub(from) >= 2 && !text.trim().is_empty()).then_some(([from, to], text))
        });
        if dwells.is_empty() {
            return recorder.collect();
        }

        let winner = first_dwells(&dwells);
        let mut cues = Vec::new();
        for ([mut from, to], text) in recorder {
            let mut gap = winner.partition_point(|(range, _)| range[1] <= from);
            for &([start, end], _) in &winner[gap..] {
                if start >= to || from >= to {
                    break;
                }
                if from < start {
                    cues.push((gap * 2, [from, start], text.clone()));
                }
                from = end;
                gap += 1;
            }
            if from < to {
                cues.push((gap * 2, [from, to], text));
            }
        }
        for (i, (range, index)) in winner.into_iter().enumerate() {
            cues.push((i * 2 + 1, range, dwells[index].1.clone()));
        }
        // Sort by the gap between dwells, keeping Recorder's stacking within each gap.
        cues.sort_by_key(|cue| cue.0);
        cues.into_iter()
            .map(|(_, range, text)| (range, text))
            .collect()
    }
}

fn chip_frames(span: SourceSpan, speed_map: &[f64], fps: f64, range: [f64; 2]) -> [Frame; 2] {
    range.map(|seconds| first_frame_at(span, speed_map, fps, seconds))
}

/// The first sampled timeline frame at or after the source seconds, or the clip's end.
fn first_frame_at(span: SourceSpan, speed_map: &[f64], fps: f64, seconds: f64) -> Frame {
    let (mut lo, mut hi) = (span.start, span.end);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if span.source_at(f64::from(mid), speed_map) / fps < seconds {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Paint in reverse log order so the first visible event owns each overlapping interval.
fn first_dwells(dwells: &[([Frame; 2], String)]) -> Vec<([Frame; 2], usize)> {
    let mut edges = BTreeMap::new();
    for (index, &([from, to], _)) in dwells.iter().enumerate().rev() {
        let after = edges.range(..=to).next_back().and_then(|(_, &value)| value);
        while let Some((&boundary, _)) = edges.range(from..to).next() {
            edges.remove(&boundary);
        }
        edges.insert(from, Some(index));
        edges.insert(to, after);
    }
    edges
        .iter()
        .zip(edges.iter().skip(1))
        .filter_map(|((&from, &winner), (&to, _))| Some(([from, to], winner?)))
        .collect()
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
