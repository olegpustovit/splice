use super::{CommandError, Json, op::*};
use splice_model::{
    Access, Aspect, Ease, Edge, EffectParam, EffectValue, Fit, Frame, Key, LaneKind, Placement,
    Rect, TimelineId, WordId,
};

type Error = CommandError;

/// Caption preset names, from the `captions` effect schema.
pub(super) fn caption_presets() -> &'static [Json] {
    super::effect_schemas()
        .field("captions")
        .and_then(|c| c.field("params"))
        .and_then(|p| p.field("preset"))
        .and_then(|p| p.field("options"))
        .and_then(Json::as_array)
        .expect("checked caption schema")
}

pub(super) trait Args {
    fn required(&self, key: &str) -> Result<&Json, Error>;
    fn string(&self, key: &str) -> Result<&str, Error>;
    fn opt_string(&self, key: &str) -> Result<Option<String>, Error>;
    fn string_patch(&self, key: &str) -> Result<Option<Option<&str>>, Error>;
    fn flag(&self, key: &str, default: bool) -> Result<bool, Error>;
    fn opt_flag(&self, key: &str) -> Result<Option<bool>, Error>;
    fn number(&self, key: &str) -> Result<f64, Error>;
    fn opt_number(&self, key: &str) -> Result<Option<f64>, Error>;
    /// An optional number within `[low, high]`, refused with the rule otherwise (never clamped).
    fn opt_within(&self, key: &str, low: f64, high: f64) -> Result<Option<f64>, Error>;
    fn opt_int(&self, key: &str) -> Result<Option<Frame>, Error>;
    fn target(&self, key: &str) -> Result<Target, Error>;
    fn opt_target(&self, key: &str) -> Result<Option<Target>, Error>;
    fn targets(&self, key: &str) -> Result<Vec<Target>, Error>;
    fn frame(&self, key: &str) -> Result<FrameArg, Error>;
    fn opt_frame(&self, key: &str) -> Result<Option<FrameArg>, Error>;
}

impl Args for Json {
    fn required(&self, key: &str) -> Result<&Json, Error> {
        self.field(key).ok_or_else(|| Error::bad(key, "a value"))
    }

    fn string(&self, key: &str) -> Result<&str, Error> {
        self.required(key)?
            .as_str()
            .ok_or_else(|| Error::bad(key, "a string"))
    }

    fn opt_string(&self, key: &str) -> Result<Option<String>, Error> {
        self.field(key)
            .map(|_| self.string(key).map(str::to_owned))
            .transpose()
    }

    fn string_patch(&self, key: &str) -> Result<Option<Option<&str>>, Error> {
        self.field(key)
            .map(|value| match value {
                Json::Null => Ok(None),
                Json::String(text) => Ok(Some(text.as_str())),
                _ => Err(Error::bad(key, "a string or null")),
            })
            .transpose()
    }

    fn flag(&self, key: &str, default: bool) -> Result<bool, Error> {
        Ok(self.opt_flag(key)?.unwrap_or(default))
    }

    fn opt_flag(&self, key: &str) -> Result<Option<bool>, Error> {
        self.field(key)
            .map(|v| v.as_bool().ok_or_else(|| Error::bad(key, "a boolean")))
            .transpose()
    }

    fn number(&self, key: &str) -> Result<f64, Error> {
        self.required(key)?
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| Error::bad(key, "a finite number"))
    }

    fn opt_number(&self, key: &str) -> Result<Option<f64>, Error> {
        self.field(key).map(|_| self.number(key)).transpose()
    }

    fn opt_within(&self, key: &str, low: f64, high: f64) -> Result<Option<f64>, Error> {
        match self.opt_number(key)? {
            Some(n) if !(low..=high).contains(&n) => Err(Error::rule(
                "BAD_ARGS",
                key,
                self.field(key).cloned(),
                &format!("a number from {low} to {high}"),
                &format!("Pass {key} between {low} and {high}."),
            )),
            value => Ok(value),
        }
    }

    fn opt_int(&self, key: &str) -> Result<Option<Frame>, Error> {
        self.field(key).map(|v| integer(v, key)).transpose()
    }

    fn target(&self, key: &str) -> Result<Target, Error> {
        target(self.required(key)?, key)
    }

    fn opt_target(&self, key: &str) -> Result<Option<Target>, Error> {
        self.field(key).map(|v| target(v, key)).transpose()
    }

    fn targets(&self, key: &str) -> Result<Vec<Target>, Error> {
        targets(self.required(key)?, key)
    }

    fn frame(&self, key: &str) -> Result<FrameArg, Error> {
        frame_arg(self.required(key)?, key)
    }

    fn opt_frame(&self, key: &str) -> Result<Option<FrameArg>, Error> {
        self.field(key).map(|v| frame_arg(v, key)).transpose()
    }
}

fn integer(value: &Json, key: &str) -> Result<Frame, Error> {
    value
        .as_i64()
        .and_then(|n| Frame::try_from(n).ok())
        .ok_or_else(|| match value.as_f64() {
            // A fraction is almost always seconds where the API counts frames.
            Some(n) if n.is_finite() && n.fract() != 0.0 => Error::rule(
                "BAD_ARGS",
                key,
                Some(value.clone()),
                "an integer frame count",
                &format!(
                    "{key} counts frames, not seconds: {n} s is {} frames at 30 fps (project.info gives fps).",
                    (n * 30.0).round()
                ),
            ),
            _ => Error::bad(key, "an integer frame in the signed 32-bit range"),
        })
}

fn shape(value: &Json, key: &str, allowed: &[&str]) -> Result<(), Error> {
    let fields = value
        .as_object()
        .ok_or_else(|| Error::bad(key, "an object"))?;
    for (field, _) in fields {
        if !allowed.contains(&field.as_str()) {
            // Name the stray field as such: "at.clip: expected item, …" read like a shape for clip.
            let mut error = Error::unknown_name(
                "UNKNOWN_PARAM",
                format!("{key} has no field {field}."),
                format!("Pass {key} as an object with {}.", allowed.join(", ")),
                field,
                &mut allowed.iter().copied(),
            );
            if let Some(rule) = error.rule.as_mut() {
                rule.param = format!("{key}.{field}");
                rule.expected = format!("one of the fields {}", allowed.join(", "));
            }
            return Err(error);
        }
    }
    Ok(())
}

pub(crate) fn reference(id: &str, key: &str) -> Result<Reference, Error> {
    let (name, part) = match id.split_once('.') {
        None => (id, RefPart::Primary),
        // After a split the referenced item keeps the left piece.
        Some((name, "primary" | "left")) => (name, RefPart::Primary),
        Some((name, "right" | "new")) => (name, RefPart::FirstCreated),
        _ => {
            return Err(Error::bad(
                key,
                "a reference like $cut, $cut.left, $cut.right, or $cut.new",
            ));
        }
    };
    validate_binding(name, key)?;
    Ok(Reference {
        name: name.into(),
        part,
    })
}

fn validate_binding(name: &str, key: &str) -> Result<(), Error> {
    let mut tail = name.strip_prefix('$').unwrap_or("").bytes();
    if !tail
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !tail.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::bad(key, "a batch name like $cut or $overlay_2"));
    }
    Ok(())
}

/// The batch name an op binds. A bare `music` binds `$music` (agents read "binds as $name" both ways).
pub(super) fn binding(value: &Json) -> Result<String, Error> {
    let name = value
        .as_str()
        .ok_or_else(|| Error::bad("ref", "a name like $music"))?;
    let name = if name.starts_with('$') {
        name.to_owned()
    } else {
        format!("${name}")
    };
    validate_binding(&name, "ref")?;
    Ok(name)
}

fn target(value: &Json, key: &str) -> Result<Target, Error> {
    // A list of one names that one target (agents write items-style lists everywhere).
    let value = match value.as_array() {
        Some([one]) => one,
        _ => value,
    };
    let id = value
        .as_str()
        .ok_or_else(|| Error::bad(key, "a Splice ID or a $ref"))?;
    if id.starts_with('$') {
        return reference(id, key).map(Target::Ref);
    }
    if id == "project" {
        return Ok(Target::Project);
    }
    let result = match id.split_once('_').map(|(prefix, _)| prefix) {
        Some("clp") => id.parse().map(|id| Target::Timeline(TimelineId::Clip(id))),
        Some("ovl") => id.parse().map(|id| Target::Timeline(TimelineId::Item(id))),
        Some("mrk") => id
            .parse()
            .map(|id| Target::Timeline(TimelineId::Marker(id))),
        Some("lan") => id.parse().map(Target::Lane),
        Some("med") => id.parse().map(Target::Media),
        Some("fx") => id.parse().map(Target::Effect),
        _ => Err(splice_model::InvalidId),
    };
    result.map_err(|_| {
        // A name a batch bound with ref, written without its $.
        let bare = !id.is_empty()
            && !id.contains('_')
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.');
        let fix = if bare {
            format!("Refs are written with $: did you mean \"${id}\"? Otherwise pass a Splice ID (clp_…, ovl_…).")
        } else {
            "Pass a Splice ID (clp_…, ovl_…, lan_…) or a $ref.".to_owned()
        };
        Error::rule(
            "BAD_ARGS",
            key,
            Some(value.clone()),
            "a Splice ID or a $ref",
            &fix,
        )
    })
}

pub(super) fn list(value: &Json) -> &[Json] {
    value
        .as_array()
        .unwrap_or_else(|| std::slice::from_ref(value))
}

pub(super) fn targets(value: &Json, key: &str) -> Result<Vec<Target>, Error> {
    let mut result = Vec::new();
    targets_into(value, key, &mut result)?;
    Ok(result)
}

pub(super) fn targets_into(value: &Json, key: &str, result: &mut Vec<Target>) -> Result<(), Error> {
    let values = list(value);
    if values.is_empty() {
        return Err(Error::bad(key, "at least one Splice ID"));
    }
    result.clear();
    result.reserve(values.len());
    for value in values {
        result.push(target(value, key)?);
    }
    Ok(())
}

pub(crate) fn frame_arg(value: &Json, key: &str) -> Result<FrameArg, Error> {
    if value.as_f64().is_some() {
        return integer(value, key).map(FrameArg::Frame);
    }
    if value.as_str().is_some_and(|id| id.starts_with('$')) {
        return Ok(FrameArg::Item {
            item: target(value, key)?,
            edge: Edge::Start,
            offset: 0,
        });
    }
    if value.as_object().is_none() {
        return Err(Error::bad(
            key,
            "an integer frame, $ref, or object with item, marker, or word",
        ));
    }
    let offset = value.opt_int("offset")?.unwrap_or(0);
    if value.field("marker").is_some() {
        shape(value, key, &["marker", "offset"])?;
        return Ok(FrameArg::Marker {
            marker: value.target("marker")?,
            offset,
        });
    }
    let edge = match value.field("edge").map(|v| v.as_str()) {
        None | Some(Some("start")) => Edge::Start,
        Some(Some("end")) => Edge::End,
        _ => return Err(Error::bad("edge", "start or end")),
    };
    if value.field("word").is_some() {
        shape(value, key, &["word", "transcriptHash", "edge", "offset"])?;
        return Ok(FrameArg::Word {
            word: word(value.required("word")?, "word")?,
            transcript_hash: hash(value)?,
            edge,
            offset,
        });
    }
    let item = if value.field("ref").is_some() {
        "ref"
    } else {
        "item"
    };
    shape(value, key, &[item, "edge", "offset"])?;
    if value.field(item).is_none() {
        return Err(Error::bad(
            key,
            "an integer frame or object with item, marker, or word",
        ));
    }
    Ok(FrameArg::Item {
        item: value.target(item)?,
        edge,
        offset,
    })
}

pub(super) fn anchor(value: &Json) -> Result<AnchorArg, Error> {
    if value.field("pinned").is_some() {
        shape(value, "anchor", &["pinned", "frame"])?;
        if !value.flag("pinned", false)? {
            return Err(Error::bad("anchor.pinned", "true"));
        }
        return Ok(AnchorArg::Pinned(value.opt_frame("frame")?));
    }
    if value.field("clip").is_some() {
        shape(value, "anchor", &["clip", "sourceFrame"])?;
        return Ok(AnchorArg::ClipSource {
            clip: value.target("clip")?,
            source: value.opt_int("sourceFrame")?.unwrap_or(0),
        });
    }
    Ok(AnchorArg::At(frame_arg(value, "anchor")?))
}

pub(super) fn position(value: &Json) -> Result<Position, Error> {
    let delta = value.opt_int("delta")?;
    if let Some(to) = value.opt_frame("to")? {
        return Ok(Position::To(to));
    }
    delta
        .map(Position::Delta)
        .ok_or_else(|| Error::bad("to or delta", "an absolute frame or a signed frame offset"))
}

pub(super) fn word(value: &Json, key: &str) -> Result<WordId, Error> {
    value
        .as_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| Error::bad(key, "a word ID like w:screen:12 from transcript.get"))
}

pub(super) fn hash(value: &Json) -> Result<Option<u64>, Error> {
    let Some(hash) = value.field("transcriptHash") else {
        return Ok(None);
    };
    let hash = hash
        .as_str()
        .filter(|s| s.len() == 16 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| {
            Error::bad(
                "transcriptHash",
                "a 16-digit hexadecimal hash from transcript.get",
            )
        })?;
    Ok(Some(
        u64::from_str_radix(hash, 16).expect("checked hexadecimal hash"),
    ))
}

pub(super) fn patch_string(value: &Json, key: &str) -> Result<Option<Option<String>>, Error> {
    Ok(value
        .string_patch(key)?
        .map(|patch| patch.map(str::to_owned)))
}

fn patch_flag(value: &Json, key: &str) -> Result<Option<Option<bool>>, Error> {
    value
        .field(key)
        .map(|v| match *v {
            Json::Null => Ok(None),
            Json::Bool(b) => Ok(Some(b)),
            _ => Err(Error::bad(key, "a boolean or null")),
        })
        .transpose()
}

pub(super) fn caption_patches(value: &Json) -> Result<Vec<CaptionWordPatch>, Error> {
    let Some(words) = value.field("words") else {
        return Ok(Vec::new());
    };
    let mut patches = Vec::new();
    for (name, data) in words
        .as_object()
        .ok_or_else(|| Error::bad("words", "an object keyed by word ID"))?
    {
        let word = name
            .parse()
            .map_err(|_| Error::bad(name, "a word ID like w:screen:12 from transcript.get"))?;
        let edit = if matches!(data, Json::Null) {
            None
        } else {
            shape(data, name, &["text", "emoji", "emph", "hidden"])?;
            Some(CaptionWordEdit {
                text: patch_string(data, "text")?,
                emoji: patch_string(data, "emoji")?,
                emph: patch_flag(data, "emph")?,
                hidden: patch_flag(data, "hidden")?,
            })
        };
        patches.push(CaptionWordPatch { word, edit });
    }
    Ok(patches)
}

/// The `params` object, built in `reuse` (its name buffers kept).
pub(super) fn effect_params(
    value: &Json,
    kind: Option<&str>,
    reuse: &mut Vec<EffectParam>,
) -> Result<Vec<EffectParam>, Error> {
    let mut result = std::mem::take(reuse);
    let Some(params) = value.field("params") else {
        result.clear();
        return Ok(result);
    };
    let mut used = 0;
    for (key, parameter) in params
        .as_object()
        .ok_or_else(|| Error::bad("params", "an object of effect parameters"))?
    {
        let value = if let Some(b) = parameter.as_bool() {
            EffectValue::Bool(b)
        } else if let Some(s) = parameter.as_str() {
            EffectValue::Text(s.into())
        } else if let Some(n) = parameter.as_f64().filter(|n| n.is_finite()) {
            EffectValue::Number(n)
        } else if let Some(keys) = decode_named_keys(parameter, kind, key) {
            EffectValue::Keys(keys)
        } else {
            return Err(Error::bad(
                key,
                "a finite number, boolean, string or {keys:[{at, value, ease}]}",
            ));
        };
        match result.get_mut(used) {
            Some(slot) => {
                slot.name.clear();
                slot.name.push_str(key);
                slot.value = value;
            }
            None => result.push(EffectParam {
                name: key.clone(),
                value,
            }),
        }
        used += 1;
    }
    result.truncate(used);
    Ok(result)
}

/// Top-level arguments that name parameters of effect `kind`, built in `reuse` (its name
/// buffers kept).
pub(super) fn schema_params(
    value: &Json,
    kind: &str,
    reuse: &mut Vec<EffectParam>,
) -> Result<Vec<EffectParam>, Error> {
    let schemas = super::effect_schemas();
    let mut result = std::mem::take(reuse);
    let Some(schema) = schemas.field(kind).and_then(|s| s.field("params")) else {
        result.clear();
        return Ok(result);
    };
    let mut used = 0;
    for (key, parameter) in value.as_object().into_iter().flatten() {
        if schema.field(key).is_none() {
            continue;
        }
        let value = if let Some(b) = parameter.as_bool() {
            EffectValue::Bool(b)
        } else if let Some(s) = parameter.as_str() {
            EffectValue::Text(s.into())
        } else if let Some(n) = parameter.as_f64().filter(|n| n.is_finite()) {
            EffectValue::Number(n)
        } else if let Some(keys) = decode_named_keys(parameter, Some(kind), key) {
            EffectValue::Keys(keys)
        } else {
            return Err(Error::bad(key, "a number, boolean, string or {keys}"));
        };
        match result.get_mut(used) {
            Some(slot) => {
                slot.name.clear();
                slot.name.push_str(key);
                slot.value = value;
            }
            None => result.push(EffectParam {
                name: key.clone(),
                value,
            }),
        }
        used += 1;
    }
    result.truncate(used);
    Ok(result)
}

/// `{keys:[…]}` whose values may name a keyed enum's options (`camera.layout`).
fn decode_named_keys(value: &Json, kind: Option<&str>, param: &str) -> Option<Vec<Key>> {
    match super::named_keys(value, kind, param) {
        Some(named) => crate::package::decode_keys(&named),
        None => crate::package::decode_keys(value),
    }
}

/// `[{at, value, ease}]` with increasing frames, for the parameter at `path` (`type.param`): a
/// keyed enum's values may be option names.
pub(super) fn keys(value: &Json, key: &str, path: Option<&str>) -> Result<Vec<Key>, Error> {
    let list = value
        .field(key)
        .ok_or_else(|| Error::bad(key, "an array of {at, value, ease}"))?;
    let named = path
        .and_then(|path| path.split_once('.'))
        .and_then(|(kind, param)| super::named_keys(list, Some(kind), param));
    let list = named.as_ref().unwrap_or(list);
    // A bad ease names its key and every accepted form.
    for (index, entry) in list.as_array().into_iter().flatten().enumerate() {
        if let Some(ease) = entry.field("ease")
            && crate::package::decode_ease(ease).is_none()
        {
            return Err(Error::rule(
                "BAD_ARGS",
                &format!("{key}[{index}].ease"),
                Some(ease.clone()),
                EASES,
                &format!("Pass {key}[{index}].ease as one of {EASES}."),
            ));
        }
    }
    let wrapped = Json::Object(vec![("keys".into(), list.clone())]);
    let mut keys = crate::package::decode_keys(&wrapped).ok_or_else(|| {
        Error::bad(
            key,
            "a nonempty array of {at, value, ease?} with increasing integer at",
        )
    })?;
    // Agent keys are eased unless they say otherwise (D20).
    for (key, entry) in keys.iter_mut().zip(list.as_array().into_iter().flatten()) {
        if entry.field("ease").is_none() {
            key.ease = Ease::InOut;
        }
    }
    Ok(keys)
}

/// Every accepted ease, as the broken rule states it (T42.1).
pub(crate) const EASES: &str = "linear, hold, in, out, inOut, back, elastic, bounce, \
    [x1, y1, x2, y2], {steps: 1–100} or {spring: {duration: 2–600 frames, bounce: −0.9–0.9 = 0}}";

/// An ease name, Bezier, steps or spring; linear when absent.
pub(super) fn ease(value: &Json, key: &str) -> Result<Ease, Error> {
    match value.field(key) {
        None => Ok(Ease::Linear),
        Some(ease) => crate::package::decode_ease(ease).ok_or_else(|| Error::bad(key, EASES)),
    }
}

pub(super) fn crop(value: &Json) -> Result<Option<Option<Rect>>, Error> {
    let Some(value) = value.field("rect").or_else(|| value.field("crop")) else {
        return Ok(None);
    };
    if *value == Json::Null {
        return Ok(Some(None));
    }
    shape(value, "crop", &["x", "y", "w", "h"])?;
    let rect = Rect {
        x: value.number("x")?,
        y: value.number("y")?,
        w: value.number("w")?,
        h: value.number("h")?,
    };
    if rect.x < 0.0
        || rect.y < 0.0
        || rect.w < 0.05
        || rect.h < 0.05
        || rect.x + rect.w > 1.0 + f64::EPSILON
        || rect.y + rect.h > 1.0 + f64::EPSILON
    {
        return Err(Error::bad(
            "crop",
            "a normalized rectangle within the frame, at least 5% wide and high",
        ));
    }
    Ok(Some(Some(rect)))
}

pub(super) fn aspect(value: &str) -> Result<Aspect, Error> {
    Aspect::ALL
        .iter()
        .find(|(_, label)| *label == value)
        .map(|(aspect, _)| *aspect)
        .ok_or_else(|| Error::bad("aspect", "9:16, 4:5, 1:1, 4:3, 16:9, 16:10 or auto"))
}

pub(super) fn access(value: &str) -> Result<Access, Error> {
    match value {
        "off" => Ok(Access::Off),
        "read" => Ok(Access::Read),
        "live" => Ok(Access::Live),
        _ => Err(Error::bad("access", "off, read, or live")),
    }
}

pub(super) fn lane_kind(value: &str) -> Result<LaneKind, Error> {
    match value {
        "graphics" => Ok(LaneKind::Graphics),
        "overlay" => Ok(LaneKind::Overlay),
        "camera" => Ok(LaneKind::Camera),
        "captions" => Ok(LaneKind::Captions),
        "audio" => Ok(LaneKind::Audio),
        _ => Err(Error::bad(
            "kind",
            "graphics, overlay, camera, captions or audio",
        )),
    }
}

pub(super) fn placement(value: &str) -> Result<Placement, Error> {
    match value {
        "full" => Ok(Placement::Full),
        "card" => Ok(Placement::Card),
        "corner-tl" => Ok(Placement::TopLeft),
        "corner-tr" => Ok(Placement::TopRight),
        "corner-bl" => Ok(Placement::BottomLeft),
        "corner-br" => Ok(Placement::BottomRight),
        _ => Err(Error::bad(
            "placement",
            "full, card, or a corner-tl/tr/bl/br value",
        )),
    }
}

pub(super) fn fit(value: &str) -> Result<Fit, Error> {
    match value {
        "fit" => Ok(Fit::Fit),
        "fill" => Ok(Fit::Fill),
        _ => Err(Error::bad("fit", "fit or fill")),
    }
}

pub(super) fn component(value: &str) -> Result<Component, Error> {
    match value {
        "screen" => Ok(Component::Screen),
        "camera" => Ok(Component::Camera),
        "mic" => Ok(Component::Mic),
        "system" => Ok(Component::System),
        "pointer" => Ok(Component::Pointer),
        "keys" => Ok(Component::Keys),
        _ => Err(Error::bad(
            "component",
            "screen, camera, mic, system, pointer, or keys",
        )),
    }
}
