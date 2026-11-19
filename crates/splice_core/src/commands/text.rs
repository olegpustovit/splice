//! Decoders for text and motion (T42): a title's inline `box`, `style`, `in`, `out` and `loop`,
//! `item.order` and `motion.preset`.
use super::{CommandError as Error, DecodeScratch, Json, args::*, check_effect, op::*};
use splice_model::{EffectParam, EffectValue, Frame};

type DecodedOp = Result<Op, Error>;

/// A title's inline look, or `None` when the op passes none of its fields.
pub(super) fn title_look(a: &Json) -> Result<Option<Box<TitleLook>>, Error> {
    let look = TitleLook {
        text_box: inline(a, "box", "layout")?,
        style: inline(a, "style", "text")?.unwrap_or_default(),
        enter: inline(a, "in", "animate")?.unwrap_or_default(),
        exit: motion(a, "out", &MotionPreset::EXIT, "duration", [1, 120], 8)?,
        repeat: motion(a, "loop", &MotionPreset::LOOP, "period", [6, 600], 60)?,
    };
    Ok((look != TitleLook::default()).then(|| Box::new(look)))
}

/// `{name: value}` checked against effect `kind`'s schema; a wrong name or value names
/// `key.name`.
fn inline(a: &Json, key: &str, kind: &str) -> Result<Option<Vec<EffectParam>>, Error> {
    let Some(value) = a.field(key) else {
        return Ok(None);
    };
    let schema = super::effect_schemas()
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .expect("schema");
    let known = || {
        schema
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, _)| name.as_str())
    };
    let fields = value.as_object().ok_or_else(|| {
        Error::bad(
            key,
            &format!(
                "an object of {kind} parameters: {}",
                known().collect::<Vec<_>>().join(", ")
            ),
        )
    })?;
    let mut params = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        if schema.field(name).is_none() {
            let mut error = Error::unknown_name(
                "UNKNOWN_PARAM",
                format!("{key} has no field {name:?}."),
                format!("{key} takes {}.", known().collect::<Vec<_>>().join(", ")),
                name,
                &mut known(),
            );
            if let Some(rule) = &mut error.rule {
                rule.param = format!("{key}.{name}");
            }
            return Err(error);
        }
        let value = match value {
            Json::Bool(b) => EffectValue::Bool(*b),
            Json::String(text) => EffectValue::Text(text.clone()),
            other => match other.as_f64().filter(|n| n.is_finite()) {
                Some(n) => EffectValue::Number(n),
                None => {
                    return Err(Error::bad(
                        &format!("{key}.{name}"),
                        "a number, boolean or string",
                    ));
                }
            },
        };
        params.push(EffectParam {
            name: name.clone(),
            value,
        });
    }
    check_effect(kind, &params).map_err(|mut error| {
        if let Some(rule) = &mut error.rule {
            rule.param = format!("{key}.{}", rule.param);
            rule.got = fields
                .iter()
                .find(|(name, _)| format!("{key}.{name}") == rule.param)
                .map(|(_, value)| value.clone());
        }
        error
    })?;
    Ok(Some(params))
}

/// `{preset, <timing>, intensity}` for `out` (timing `duration`) or `loop` (timing `period`).
fn motion(
    a: &Json,
    key: &str,
    presets: &[(MotionPreset, &str)],
    timing: &str,
    [low, high]: [Frame; 2],
    default: Frame,
) -> Result<Option<Motion>, Error> {
    let Some(value) = a.field(key) else {
        return Ok(None);
    };
    let names = presets
        .iter()
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let fields = value.as_object().ok_or_else(|| {
        Error::bad(
            key,
            &format!("{{preset: {names}, {timing}: {low}–{high} frames, intensity: 0–1}}"),
        )
    })?;
    for (name, _) in fields {
        if ![("preset"), timing, "intensity"].contains(&name.as_str()) {
            let mut error = Error::unknown_name(
                "UNKNOWN_PARAM",
                format!("{key} has no field {name:?}."),
                format!("{key} takes preset, {timing} and intensity."),
                name,
                &mut ["preset", timing, "intensity"].into_iter(),
            );
            if let Some(rule) = &mut error.rule {
                rule.param = format!("{key}.{name}");
            }
            return Err(error);
        }
    }
    let preset = value.field("preset").and_then(Json::as_str);
    let preset = presets
        .iter()
        .find(|(_, name)| Some(*name) == preset)
        .map(|(preset, _)| *preset)
        .ok_or_else(|| {
            Error::rule(
                "BAD_ARGS",
                &format!("{key}.preset"),
                value.field("preset").cloned(),
                &names,
                &format!("Pass {key}.preset as one of {names}."),
            )
        })?;
    let length = match value.field(timing) {
        None => default,
        Some(n) => n
            .as_i64()
            .and_then(|n| Frame::try_from(n).ok())
            .filter(|n| (low..=high).contains(n))
            .ok_or_else(|| {
                Error::rule(
                    "BAD_ARGS",
                    &format!("{key}.{timing}"),
                    Some(n.clone()),
                    &format!("whole frames from {low} to {high}"),
                    &format!("Pass {key}.{timing} between {low} and {high} frames."),
                )
            })?,
    };
    let intensity = match value.field("intensity") {
        None => 0.5,
        Some(n) => n
            .as_f64()
            .filter(|n| (0.0..=1.0).contains(n))
            .ok_or_else(|| {
                Error::rule(
                    "BAD_ARGS",
                    &format!("{key}.intensity"),
                    Some(n.clone()),
                    "a number from 0 to 1",
                    &format!("Pass {key}.intensity between 0 and 1."),
                )
            })?,
    };
    Ok(Some(Motion {
        preset,
        length,
        intensity,
    }))
}

pub(super) fn item_order(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let to = a.required("to")?;
    let expected = "\"front\", \"back\", {above: id}, {below: id} or an integer z";
    let bad = || {
        Error::rule(
            "BAD_ARGS",
            "to",
            Some(to.clone()),
            expected,
            &format!("Pass to as {expected}."),
        )
    };
    let to = match to {
        Json::String(name) if name == "front" => Order::Front,
        Json::String(name) if name == "back" => Order::Back,
        Json::Object(fields) if fields.len() == 1 => {
            if to.field("above").is_some() {
                Order::Above(to.target("above")?)
            } else if to.field("below").is_some() {
                Order::Below(to.target("below")?)
            } else {
                return Err(bad());
            }
        }
        other => Order::Z(
            other
                .as_i64()
                .and_then(|z| i32::try_from(z).ok())
                .filter(|z| z.unsigned_abs() <= 1_000_000)
                .ok_or_else(bad)?,
        ),
    };
    Ok(Op::ItemOrder {
        item: a.target("item")?,
        to,
    })
}

pub(super) fn motion_preset(a: &Json, _scratch: &mut DecodeScratch) -> DecodedOp {
    let names = MotionPreset::MOTION
        .iter()
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let preset = a.field("preset").and_then(Json::as_str);
    let preset = MotionPreset::MOTION
        .iter()
        .find(|(_, name)| Some(*name) == preset)
        .map(|(preset, _)| *preset)
        .ok_or_else(|| Error::bad("preset", &names))?;
    let frames = |key: &str, low: Frame, high: Frame| -> Result<Option<Frame>, Error> {
        match a.opt_int(key)? {
            Some(n) if !(low..=high).contains(&n) => Err(Error::rule(
                "BAD_ARGS",
                key,
                a.field(key).cloned(),
                &format!("whole frames from {low} to {high}"),
                &format!("Pass {key} between {low} and {high}."),
            )),
            value => Ok(value),
        }
    };
    let at = frames("at", 0, 1_000_000)?.unwrap_or(0);
    let length = frames("length", 2, Frame::MAX)?;
    Ok(Op::MotionPreset {
        items: a.targets("items")?,
        motion: Motion {
            preset,
            length: length.unwrap_or(0),
            intensity: a.opt_within("intensity", 0.0, 1.0)?.unwrap_or(0.5),
        },
        at,
        sized: length.is_some(),
    })
}
