use super::{CommandError, Json, op::*};
use splice_model::{
    Access, Aspect, Edge, EffectParam, EffectValue, Fit, Frame, LaneKind, Placement, Rect,
    TimelineId, WordId,
};

type Error = CommandError;

pub(super) const CAPTION_PRESETS: &[&str] =
    &["bold", "karaoke", "pop", "box", "clean", "typewriter"];

pub(super) trait Args {
    fn required(&self, key: &str) -> Result<&Json, Error>;
    fn string(&self, key: &str) -> Result<&str, Error>;
    fn opt_string(&self, key: &str) -> Result<Option<String>, Error>;
    fn string_patch(&self, key: &str) -> Result<Option<Option<&str>>, Error>;
    fn flag(&self, key: &str, default: bool) -> Result<bool, Error>;
    fn opt_flag(&self, key: &str) -> Result<Option<bool>, Error>;
    fn number(&self, key: &str) -> Result<f64, Error>;
    fn opt_number(&self, key: &str) -> Result<Option<f64>, Error>;
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
        .ok_or_else(|| Error::bad(key, "an integer frame in the signed 32-bit range"))
}

fn shape(value: &Json, key: &str, allowed: &[&str]) -> Result<(), Error> {
    let fields = value
        .as_object()
        .ok_or_else(|| Error::bad(key, "an object"))?;
    for (field, _) in fields {
        if !allowed.contains(&field.as_str()) {
            return Err(Error::bad(
                &format!("{key}.{field}"),
                &format!("one of the fields {}", allowed.join(", ")),
            ));
        }
    }
    Ok(())
}

fn reference(id: &str, key: &str) -> Result<Reference, Error> {
    let (name, part) = match id.split_once('.') {
        None => (id, RefPart::Primary),
        Some((name, "primary")) => (name, RefPart::Primary),
        Some((name, "right" | "new")) => (name, RefPart::FirstCreated),
        _ => {
            return Err(Error::bad(
                key,
                "a reference like $cut, $cut.right, or $cut.new",
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

pub(super) fn binding(value: &Json) -> Result<String, Error> {
    let name = value
        .as_str()
        .ok_or_else(|| Error::bad("ref", "a name starting with $"))?;
    validate_binding(name, "ref")?;
    Ok(name.into())
}

fn target(value: &Json, key: &str) -> Result<Target, Error> {
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
    result.map_err(|_| Error::bad(key, "a Splice ID or a $ref"))
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
        .ok_or_else(|| Error::bad(key, "a word ID like w12"))
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
            .map_err(|_| Error::bad(name, "a word ID like w12"))?;
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

pub(super) fn effect_params(value: &Json) -> Result<Vec<EffectParam>, Error> {
    let Some(params) = value.field("params") else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
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
        } else {
            return Err(Error::bad(key, "a finite number, boolean, or string"));
        };
        result.push(EffectParam {
            name: key.clone(),
            value,
        });
    }
    Ok(result)
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
    match value {
        "9:16" => Ok(Aspect::Portrait),
        "4:5" => Ok(Aspect::Social),
        "1:1" => Ok(Aspect::Square),
        "4:3" => Ok(Aspect::Standard),
        "16:9" => Ok(Aspect::Landscape),
        _ => Err(Error::bad("aspect", "9:16, 4:5, 1:1, 4:3, or 16:9")),
    }
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
        "motion" => Ok(LaneKind::Motion),
        "captions" => Ok(LaneKind::Captions),
        "audio" => Ok(LaneKind::Audio),
        _ => Err(Error::bad(
            "kind",
            "graphics, overlay, motion, captions, or audio",
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
