//! Replay of journals written by the 0.3 engine (record `engine` absent). 0.4 retired some
//! commands and changed what others make, so a 0.3 record is replayed with its own meaning:
//! retired forms translate to their 0.4 equivalents, `intent.autoZoom` bakes the 0.3 transform
//! keys it baked then, and IDs mint leniently (mints.rs). Live calls never come through here.
use super::settle::Owner;
use super::time::{rounded, source_rate};
use super::*;
use splice_json::Value as Json;

/// A retired command as its 0.4 equivalent, or `None` when the op is current.
pub(super) fn translate(op: &Json) -> Option<Json> {
    let field = |key: &str| op.field(key).cloned();
    let fields = |pairs: Vec<(&str, Option<Json>)>| {
        Json::Object(
            pairs
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
                .collect(),
        )
    };
    let text = |s: &str| Some(Json::String(s.into()));
    // A motion item ended at `to`: the camera holds that magnification (at least 1×).
    let amount = || {
        op.field("to")
            .and_then(Json::as_f64)
            .map(|to| Json::Float(to.clamp(Zoom::AMOUNT[0], Zoom::AMOUNT[1])))
    };
    match op.field("cmd").and_then(Json::as_str)? {
        "motion.add" => Some(fields(vec![
            ("cmd", text("zoom.add")),
            ("at", field("at")),
            (
                "length",
                Some(field("length").unwrap_or(Json::Unsigned(120))),
            ),
            ("amount", Some(amount().unwrap_or(Json::Float(1.32)))),
            ("focus", field("focus").filter(|f| *f != Json::Null)),
            ("lane", field("lane")),
            ("ref", field("ref")),
        ])),
        "motion.set" => Some(fields(vec![
            ("cmd", text("zoom.set")),
            ("zoom", field("item")),
            ("amount", amount()),
            ("focus", field("focus")),
            ("ref", field("ref")),
        ])),
        // 0.3 zoom.set configured the click-zoom curve (now an inert legacy effect).
        "zoom.set" if op.field("zoom").is_none() => Some(fields(vec![
            ("cmd", text("effect.add")),
            (
                "target",
                Some(field("target").unwrap_or(Json::String("project".into()))),
            ),
            ("type", text("clickZoom")),
            (
                "params",
                Some(fields(vec![
                    ("scale", field("scale")),
                    ("hold", field("hold")),
                ])),
            ),
            ("enabled", field("enabled")),
            ("ref", field("ref")),
        ])),
        // 0.3 took keys on any number and rendered the default where nothing animated it;
        // 0.4 refuses those (NOT_KEYABLE). Like a 0.3 file on load, keep the first value.
        "param.keys" => {
            let path = op.field("path").and_then(Json::as_str)?;
            let (kind, name) = path.split_once('.')?;
            if keyable(kind, name) {
                return None;
            }
            let first = op
                .field("keys")?
                .as_array()?
                .first()?
                .field("value")?
                .clone();
            Some(fields(vec![
                ("cmd", text("param.set")),
                ("target", field("target")),
                ("path", field("path")),
                ("value", Some(first)),
                ("ref", field("ref")),
            ]))
        }
        "effect.add" | "effect.set" => {
            let kind = op.field("type").and_then(Json::as_str)?;
            let Json::Object(params) = op.field("params")? else {
                return None;
            };
            let unkeyed: Vec<(String, Json)> = params
                .iter()
                .map(
                    |(name, value)| match value.field("keys").and_then(Json::as_array) {
                        Some(keys) if !keyable(kind, name) => (
                            name.clone(),
                            keys.first()
                                .and_then(|key| key.field("value"))
                                .cloned()
                                .unwrap_or_else(|| value.clone()),
                        ),
                        _ => (name.clone(), value.clone()),
                    },
                )
                .collect();
            if unkeyed == *params {
                return None;
            }
            let Json::Object(mut all) = op.clone() else {
                return None;
            };
            for (key, value) in &mut all {
                if key == "params" {
                    *value = Json::Object(unkeyed.clone());
                }
            }
            Some(Json::Object(all))
        }
        _ => None,
    }
}

/// Whether the schema animates `kind.name` (unknown names stay as they are: decode reports them).
fn keyable(kind: &str, name: &str) -> bool {
    crate::commands::all_effect_schemas()
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .and_then(|params| params.field(name))
        .is_none_or(crate::commands::keyable)
}

/// A click on the source clock: frame, x, y.
type Click = (Frame, f64, f64);

/// A 0.3 `intent.autoZoom` (it carries `scale` or `hold`, which 0.4 does not take).
pub(super) struct AutoZoom {
    item: Target,
    clicks: Vec<Click>,
    scale: f64,
    hold: Frame,
}

pub(super) fn auto_zoom(op: &Json) -> Result<Option<AutoZoom>, CommandError> {
    if op.field("cmd").and_then(Json::as_str) != Some("intent.autoZoom")
        || (op.field("scale").is_none() && op.field("hold").is_none())
    {
        return Ok(None);
    }
    let bad = || CommandError::bad("clicks", "{at, x, y} source clicks");
    let mut clicks = Vec::new();
    for click in op
        .field("clicks")
        .and_then(Json::as_array)
        .ok_or_else(bad)?
    {
        let at = click
            .field("at")
            .and_then(Json::as_i64)
            .and_then(|at| Frame::try_from(at).ok())
            .ok_or_else(bad)?;
        let number = |key| click.field(key).and_then(Json::as_f64).ok_or_else(bad);
        clicks.push((at, number("x")?, number("y")?));
    }
    let item = op
        .field("item")
        .and_then(Json::as_str)
        .and_then(|id| id.parse().ok())
        .map(|id| Target::Timeline(TimelineId::Clip(id)))
        .ok_or_else(|| CommandError::bad("item", "a clip ID"))?;
    Ok(Some(AutoZoom {
        item,
        clicks,
        scale: op.field("scale").and_then(Json::as_f64).unwrap_or(1.8),
        hold: op
            .field("hold")
            .and_then(Json::as_i64)
            .and_then(|h| Frame::try_from(h).ok())
            .unwrap_or(45),
    }))
}

impl Editor {
    /// The 0.3 algorithm, unchanged: clicks grouped within the hold become transform scale and
    /// anchor keys on the clip, and the click-zoom curve is switched off.
    pub(super) fn legacy_auto_zoom(&mut self, op: AutoZoom) -> Result<Outcome, CommandError> {
        let TimelineId::Clip(id) = self.timeline(&op.item)? else {
            return Err(CommandError::bad("item", "a spine clip"));
        };
        let slot = self
            .project
            .clips
            .find(id)
            .ok_or_else(|| CommandError::bad("item", "a live clip"))?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let rate = source_rate(&self.project, slot)?;
        let ease = 12;
        // (first frame, end of hold, clicks) per group.
        let mut groups: Vec<(Frame, Frame, Vec<Click>)> = Vec::new();
        for &(source, x, y) in &op.clicks {
            let local = rounded((f64::from(source) - f64::from(timing.source)) / rate)?;
            if local < 0 || local >= timing.length {
                continue;
            }
            let (x, y) = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
            match groups.last_mut() {
                Some(group) if local <= group.1 + ease => {
                    group.1 = local + op.hold;
                    group.2.push((local, x, y));
                }
                _ => groups.push((local, local + op.hold, vec![(local, x, y)])),
            }
        }
        if groups.is_empty() {
            return Err(CommandError::bad("clicks", "clicks inside the clip"));
        }
        let key = |at, value, ease| Key { at, value, ease };
        let (mut scales, mut xs, mut ys) =
            (Vec::<Key>::new(), Vec::<Key>::new(), Vec::<Key>::new());
        let last = timing.length - 1;
        for (index, (start, end, points)) in groups.iter().enumerate() {
            let zoom_in = (start - ease).max(scales.last().map_or(0, |k| k.at + 1));
            let peak = (*start).max(zoom_in + 1);
            let out = (*end).min(last - 1).max(peak + 1);
            let rest = (out + ease).min(last).max(out + 1);
            if index > 0 && zoom_in <= scales.last().map_or(-1, |k| k.at) {
                continue;
            }
            scales.extend([
                key(zoom_in, 1.0, Ease::InOut),
                key(peak, op.scale, Ease::Linear),
                key(out, op.scale, Ease::InOut),
                key(rest, 1.0, Ease::Linear),
            ]);
            for (n, &(at, x, y)) in points.iter().enumerate() {
                let at = if n == 0 {
                    zoom_in
                } else {
                    at.max(xs.last().map_or(0, |k| k.at + 1))
                };
                let ease = if n + 1 == points.len() {
                    Ease::Hold
                } else {
                    Ease::InOut
                };
                xs.push(key(at, x, ease));
                ys.push(key(at, y, ease));
            }
        }
        scales.retain(|k| k.at <= last);
        let keyed = |name: &str, keys| EffectParam {
            name: name.into(),
            value: EffectValue::Keys(keys),
        };
        let params = vec![
            keyed("scale", scales),
            keyed("anchorX", xs),
            keyed("anchorY", ys),
        ];
        let owner = Owner::Clip(slot);
        let mut created = Vec::new();
        let fx = self.upsert(owner, "transform", params, Some(true), &mut created)?;
        if self.fits(super::tier_b::applies("clickZoom")?.0, owner) {
            self.upsert(owner, "clickZoom", Vec::new(), Some(false), &mut created)?;
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Baked {} zoom(s) into editable transform keys on {id}",
                groups.len()
            )),
            primary: Some(Target::Effect(fx)),
            created,
        })
    }
}
