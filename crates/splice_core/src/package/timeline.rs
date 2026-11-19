use super::{PackageError, codec::*};
use splice_json::Value as Json;
use splice_model::*;

pub(super) fn encode_clip(id: ClipId, timing: ClipTiming, data: &ClipData) -> Json {
    let mut enabled = Vec::new();
    for (name, enabled_value) in [
        ("screen", data.enabled.screen),
        ("camera", data.enabled.camera),
        ("mic", data.enabled.mic),
        ("system", data.enabled.system),
        ("pointer", data.enabled.pointer),
        ("keys", data.enabled.keys),
    ] {
        push_bool(&mut enabled, name, enabled_value);
    }
    let mut fields = vec![
        ("id", display(&id)),
        ("media", data.media.map_or(Json::Null, |id| display(&id))),
        ("start", frame_json(timing.start)),
        ("length", frame_json(timing.length)),
        ("source", frame_json(timing.source)),
        ("speed", Json::Float(timing.speed)),
        ("fadeIn", frame_json(data.appearance.fade_in)),
        ("fadeOut", frame_json(data.appearance.fade_out)),
        (
            "volume",
            object(vec![
                ("mic", Json::Float(data.volume.mic)),
                ("system", Json::Float(data.volume.system)),
            ]),
        ),
        ("enabled", object(enabled)),
        ("crop", encode_crop(data.framing.crop)),
        ("fit", encode_fit(data.framing.fit)),
        ("effects", encode_effects(&data.appearance.effects)),
    ];
    push_text(&mut fields, "name", &data.appearance.name);
    push_text(&mut fields, "color", &data.appearance.color);
    if !data.joins.is_empty() {
        fields.push((
            "joins",
            Json::Array(data.joins.iter().map(|&at| frame_json(at)).collect()),
        ));
    }
    if !data.hidden_keys.is_empty() {
        fields.push((
            "hiddenKeys",
            Json::Array(
                data.hidden_keys
                    .iter()
                    .map(|&index| Json::Unsigned(u64::from(index)))
                    .collect(),
            ),
        ));
    }
    object(fields)
}

pub(super) fn decode_clip(value: &Json) -> Result<Clip, PackageError> {
    shape(
        value,
        &[
            "id",
            "media",
            "start",
            "length",
            "source",
            "speed",
            "fadeIn",
            "fadeOut",
            "volume",
            "enabled",
            "crop",
            "fit",
            "effects",
            "name",
            "color",
            "hiddenKeys",
            "joins",
        ],
    )?;
    let media = optional_string(value, "media")?
        .map(|text| text.parse().map_err(|_| bad_id("clip.media")))
        .transpose()?;
    let volume = value.required("volume")?;
    shape(volume, &["mic", "system"])?;
    let enabled = value.required("enabled")?;
    shape(
        enabled,
        &["screen", "camera", "mic", "system", "pointer", "keys"],
    )?;
    let mut hidden_keys = Vec::new();
    for index in value.array_or_empty("hiddenKeys")? {
        hidden_keys.push(
            index
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| bad("hiddenKeys", "an array of unsigned indices"))?,
        );
    }
    let mut joins = Vec::new();
    for at in value.array_or_empty("joins")? {
        joins.push(
            at.as_i64()
                .and_then(|n| Frame::try_from(n).ok())
                .filter(|&n| n > 0)
                .ok_or_else(|| bad("joins", "an array of positive frame offsets"))?,
        );
    }
    Ok(Clip {
        id: value.string("id")?.parse().map_err(|_| bad_id("clip.id"))?,
        timing: ClipTiming {
            start: frame(value, "start")?,
            length: frame(value, "length")?,
            source: frame(value, "source")?,
            speed: number(value, "speed")?,
        },
        data: ClipData {
            media,
            appearance: decode_appearance(value)?,
            framing: decode_framing(value)?,
            enabled: ComponentStates {
                screen: optional_bool(enabled, "screen")?,
                camera: optional_bool(enabled, "camera")?,
                mic: optional_bool(enabled, "mic")?,
                system: optional_bool(enabled, "system")?,
                pointer: optional_bool(enabled, "pointer")?,
                keys: optional_bool(enabled, "keys")?,
            },
            volume: AudioGains {
                mic: number(volume, "mic")?,
                system: number(volume, "system")?,
            },
            hidden_keys,
            joins,
        },
    })
}

pub(super) fn encode_item(id: ItemId, timing: ItemTiming, data: &ItemData) -> Json {
    let (kind, params) = match &data.kind {
        ItemKind::Title { text: title } => ("title", object(vec![("text", text(title))])),
        ItemKind::Zoom(zoom) => {
            let mut fields = vec![("amount", Json::Float(zoom.amount))];
            if let Some([x, y]) = zoom.focus {
                fields.push((
                    "focus",
                    object(vec![("x", Json::Float(x)), ("y", Json::Float(y))]),
                ));
            }
            if zoom.mode != CameraMode::Auto {
                fields.push(("mode", text(zoom.mode.name())));
            }
            if zoom.spring != Spring::Focused {
                fields.push(("spring", text(zoom.spring.name())));
            }
            ("zoom", object(fields))
        }
        ItemKind::Caption {
            words,
            edits,
            text: cue,
        } => {
            let mut fields = vec![
                (
                    "words",
                    Json::Array(words.iter().map(|id| display(id)).collect()),
                ),
                (
                    "edits",
                    Json::Array(edits.iter().map(encode_caption_edit).collect()),
                ),
            ];
            push_text(&mut fields, "text", cue);
            ("caption", object(fields))
        }
        ItemKind::Media(overlay) => (
            "media",
            object(vec![
                ("media", display(&overlay.media)),
                ("source", frame_json(overlay.source)),
                ("placement", text(placement_name(overlay.placement))),
                ("behindPresenter", Json::Bool(overlay.behind_presenter)),
                ("muted", Json::Bool(overlay.muted)),
                ("crop", encode_crop(overlay.framing.crop)),
                ("fit", encode_fit(overlay.framing.fit)),
            ]),
        ),
        ItemKind::Sound(sound) => {
            let mut fields = vec![
                ("media", display(&sound.media)),
                ("source", frame_json(sound.source)),
                ("volume", Json::Float(sound.volume)),
            ];
            if let Some(duck) = sound.duck {
                fields.push((
                    "duck",
                    object(vec![
                        ("amount", Json::Float(duck.amount)),
                        ("attack", frame_json(duck.attack)),
                        ("release", frame_json(duck.release)),
                    ]),
                ));
            }
            ("sound", object(fields))
        }
        ItemKind::Shape { shape, rect } => (
            "shape",
            object(vec![
                ("shape", text(shape.name())),
                ("rect", encode_rect(*rect)),
            ]),
        ),
    };
    let mut fields = vec![
        ("id", display(&id)),
        ("kind", text(kind)),
        ("lane", display(&data.lane)),
        ("anchor", encode_anchor(timing.anchor)),
        ("length", frame_json(timing.length)),
        ("params", params),
        ("effects", encode_effects(&data.appearance.effects)),
    ];
    if data.appearance.fade_in != 0 {
        fields.push(("fadeIn", frame_json(data.appearance.fade_in)));
    }
    if data.appearance.fade_out != 0 {
        fields.push(("fadeOut", frame_json(data.appearance.fade_out)));
    }
    push_text(&mut fields, "name", &data.appearance.name);
    push_text(&mut fields, "color", &data.appearance.color);
    object(fields)
}

pub(super) fn decode_item(value: &Json) -> Result<Item, PackageError> {
    shape(
        value,
        &[
            "id", "kind", "lane", "anchor", "length", "params", "effects", "fadeIn", "fadeOut",
            "name", "color",
        ],
    )?;
    let params = value.required("params")?;
    let kind = match value.string("kind")? {
        "title" => {
            shape(params, &["text"])?;
            ItemKind::Title {
                text: params.string("text")?.into(),
            }
        }
        "zoom" => {
            shape(params, &["amount", "focus", "mode", "spring"])?;
            let focus = match params.field("focus") {
                None => None,
                Some(point) => {
                    shape(point, &["x", "y"])?;
                    let [x, y] = [number(point, "x")?, number(point, "y")?];
                    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                        return Err(bad("zoom.focus", "a content point with x and y in 0–1"));
                    }
                    Some([x, y])
                }
            };
            let amount = number(params, "amount")?;
            if !(Zoom::AMOUNT[0]..=Zoom::AMOUNT[1]).contains(&amount) {
                return Err(bad("zoom.amount", "a magnification from 1 to 5"));
            }
            let named = |key: &str| match params.field(key) {
                None => Ok(None),
                Some(value) => value
                    .as_str()
                    .map(Some)
                    .ok_or_else(|| bad("zoom.mode/spring", "a name")),
            };
            ItemKind::Zoom(Zoom {
                amount,
                focus,
                mode: match named("mode")? {
                    None => CameraMode::Auto,
                    Some(name) => CameraMode::parse(name)
                        .ok_or_else(|| bad("zoom.mode", "auto, content or frame"))?,
                },
                spring: match named("spring")? {
                    None => Spring::Focused,
                    Some(name) => Spring::parse(name)
                        .ok_or_else(|| bad("zoom.spring", "focused, smooth, snappy or instant"))?,
                },
            })
        }
        // 0.3 motion items migrate to camera segments that hold their end scale (D12): the old
        // renderer snapped back to 1× after them, which nobody wanted.
        "motion" => {
            shape(params, &["from", "to", "focus", "ease"])?;
            let to = number(params, "to")?;
            let focus = match params.field("focus") {
                None => None,
                Some(point) => {
                    shape(point, &["x", "y"])?;
                    Some([number(point, "x")?, number(point, "y")?])
                }
            };
            ItemKind::Zoom(Zoom {
                amount: to.max(1.0),
                focus,
                mode: CameraMode::Auto,
                spring: Spring::Focused,
            })
        }
        "media" => {
            shape(
                params,
                &[
                    "media",
                    "source",
                    "placement",
                    "behindPresenter",
                    "muted",
                    "crop",
                    "fit",
                ],
            )?;
            ItemKind::Media(Overlay {
                media: params
                    .string("media")?
                    .parse()
                    .map_err(|_| bad_id("item.media"))?,
                source: frame(params, "source")?,
                placement: decode_placement(params.string("placement")?)?,
                behind_presenter: boolean(params, "behindPresenter")?,
                muted: boolean(params, "muted")?,
                framing: decode_framing(params)?,
            })
        }
        "sound" => {
            shape(params, &["media", "source", "volume", "duck"])?;
            let duck = match params.field("duck") {
                None => None,
                Some(duck) => {
                    shape(duck, &["amount", "attack", "release"])?;
                    Some(Duck {
                        amount: number(duck, "amount")?,
                        attack: frame(duck, "attack")?,
                        release: frame(duck, "release")?,
                    })
                }
            };
            ItemKind::Sound(Sound {
                media: params
                    .string("media")?
                    .parse()
                    .map_err(|_| bad_id("item.media"))?,
                source: frame(params, "source")?,
                volume: number(params, "volume")?,
                duck,
            })
        }
        "caption" => {
            shape(params, &["words", "edits", "text"])?;
            let mut words = Vec::new();
            for word in params.array("words")? {
                words.push(
                    word.as_str()
                        .and_then(|value| value.parse().ok())
                        .ok_or_else(|| bad_id("caption word"))?,
                );
            }
            let edits = params
                .array_or_empty("edits")?
                .iter()
                .map(decode_caption_edit)
                .collect::<Result<Vec<_>, _>>()?;
            let text = optional_string(params, "text")?;
            if text.is_some() && !words.is_empty() {
                return Err(bad("caption", "words or free text, not both"));
            }
            ItemKind::Caption { words, edits, text }
        }
        "shape" => {
            shape(params, &["shape", "rect"])?;
            ItemKind::Shape {
                shape: ShapeKind::parse(params.string("shape")?).ok_or_else(|| {
                    bad(
                        "shape",
                        "solid, gradient, rect, ellipse, arrow, line or callout",
                    )
                })?,
                rect: decode_rect(params.required("rect")?)?,
            }
        }
        _ => {
            return Err(bad(
                "item.kind",
                "title, media, motion, sound, shape or caption",
            ));
        }
    };
    Ok(Item {
        id: value.string("id")?.parse().map_err(|_| bad_id("item.id"))?,
        timing: ItemTiming {
            anchor: decode_anchor(value.required("anchor")?)?,
            length: frame(value, "length")?,
        },
        data: ItemData {
            lane: value
                .string("lane")?
                .parse()
                .map_err(|_| bad_id("item.lane"))?,
            kind,
            appearance: decode_appearance(value)?,
        },
    })
}

fn encode_caption_edit(edit: &CaptionEdit) -> Json {
    let mut fields = vec![("word", display(&edit.word))];
    push_text(&mut fields, "text", &edit.text);
    if edit.emph {
        fields.push(("emph", Json::Bool(true)));
    }
    push_text(&mut fields, "emoji", &edit.emoji);
    object(fields)
}

fn decode_caption_edit(value: &Json) -> Result<CaptionEdit, PackageError> {
    shape(value, &["word", "text", "emph", "emoji"])?;
    Ok(CaptionEdit {
        word: value
            .string("word")?
            .parse()
            .map_err(|_| bad_id("caption.word"))?,
        text: optional_string(value, "text")?,
        emph: optional_bool(value, "emph")?.unwrap_or(false),
        emoji: optional_string(value, "emoji")?,
    })
}

fn placement_name(placement: Placement) -> &'static str {
    match placement {
        Placement::Full => "full",
        Placement::Card => "card",
        Placement::TopLeft => "corner-tl",
        Placement::TopRight => "corner-tr",
        Placement::BottomLeft => "corner-bl",
        Placement::BottomRight => "corner-br",
    }
}

fn decode_placement(value: &str) -> Result<Placement, PackageError> {
    match value {
        "full" => Ok(Placement::Full),
        "card" => Ok(Placement::Card),
        "corner-tl" => Ok(Placement::TopLeft),
        "corner-tr" => Ok(Placement::TopRight),
        "corner-bl" => Ok(Placement::BottomLeft),
        "corner-br" => Ok(Placement::BottomRight),
        _ => Err(bad("placement", "full, card, or corner-tl/tr/bl/br")),
    }
}

fn encode_crop(rect: Option<Rect>) -> Json {
    rect.map_or(Json::Null, |r| {
        object(vec![
            ("x", Json::Float(r.x)),
            ("y", Json::Float(r.y)),
            ("w", Json::Float(r.w)),
            ("h", Json::Float(r.h)),
        ])
    })
}

fn encode_fit(fit: Fit) -> Json {
    text(match fit {
        Fit::Fit => "fit",
        Fit::Fill => "fill",
    })
}

fn decode_framing(value: &Json) -> Result<Framing, PackageError> {
    let crop = if let Some(crop) = value.optional("crop") {
        shape(crop, &["x", "y", "w", "h"])?;
        Some(Rect {
            x: number(crop, "x")?,
            y: number(crop, "y")?,
            w: number(crop, "w")?,
            h: number(crop, "h")?,
        })
    } else {
        None
    };
    let fit = match value.string("fit")? {
        "fit" => Fit::Fit,
        "fill" => Fit::Fill,
        _ => return Err(bad("fit", "fit or fill")),
    };
    Ok(Framing { crop, fit })
}

fn decode_appearance(value: &Json) -> Result<Appearance, PackageError> {
    Ok(Appearance {
        fade_in: optional_frame(value, "fadeIn")?,
        fade_out: optional_frame(value, "fadeOut")?,
        name: optional_string(value, "name")?,
        color: optional_string(value, "color")?,
        effects: decode_effects(value)?,
    })
}

fn optional_frame(value: &Json, key: &'static str) -> Result<Frame, PackageError> {
    value.field(key).map_or(Ok(0), |_| frame(value, key))
}

pub(super) fn encode_anchor(anchor: Anchor) -> Json {
    object(match anchor {
        Anchor::Pinned(at) => vec![("pinned", Json::Bool(true)), ("frame", frame_json(at))],
        Anchor::ClipSource { clip, source } => vec![
            ("clip", display(&clip)),
            ("sourceFrame", frame_json(source)),
        ],
        Anchor::Item { item, edge, offset } => vec![
            (
                "item",
                match item {
                    TimelineId::Clip(id) => display(&id),
                    TimelineId::Item(id) => display(&id),
                    TimelineId::Marker(id) => display(&id),
                },
            ),
            ("edge", encode_edge(edge)),
            ("offset", frame_json(offset)),
        ],
        Anchor::Marker { marker, offset } => {
            vec![("marker", display(&marker)), ("offset", frame_json(offset))]
        }
        Anchor::Word {
            word,
            transcript_hash,
            edge,
            offset,
        } => vec![
            ("word", display(&word)),
            ("transcriptHash", hash_text(transcript_hash)),
            ("edge", encode_edge(edge)),
            ("offset", frame_json(offset)),
        ],
    })
}

fn decode_anchor(value: &Json) -> Result<Anchor, PackageError> {
    shape(
        value,
        &[
            "pinned",
            "frame",
            "clip",
            "sourceFrame",
            "item",
            "edge",
            "offset",
            "marker",
            "word",
            "transcriptHash",
        ],
    )?;
    if value.field("pinned").is_some() {
        shape(value, &["pinned", "frame"])?;
        if !boolean(value, "pinned")? {
            return Err(bad("anchor.pinned", "true"));
        }
        Ok(Anchor::Pinned(frame(value, "frame")?))
    } else if value.field("clip").is_some() {
        shape(value, &["clip", "sourceFrame"])?;
        Ok(Anchor::ClipSource {
            clip: value
                .string("clip")?
                .parse()
                .map_err(|_| bad_id("anchor.clip"))?,
            source: frame(value, "sourceFrame")?,
        })
    } else if value.field("item").is_some() {
        shape(value, &["item", "edge", "offset"])?;
        let id = value.string("item")?;
        let item = if let Ok(id) = id.parse() {
            TimelineId::Clip(id)
        } else if let Ok(id) = id.parse() {
            TimelineId::Item(id)
        } else if let Ok(id) = id.parse() {
            TimelineId::Marker(id)
        } else {
            return Err(bad_id("anchor.item"));
        };
        Ok(Anchor::Item {
            item,
            edge: decode_edge(value)?,
            offset: optional_frame(value, "offset")?,
        })
    } else if value.field("marker").is_some() {
        shape(value, &["marker", "offset"])?;
        Ok(Anchor::Marker {
            marker: value
                .string("marker")?
                .parse()
                .map_err(|_| bad_id("anchor.marker"))?,
            offset: optional_frame(value, "offset")?,
        })
    } else if value.field("word").is_some() {
        shape(value, &["word", "transcriptHash", "edge", "offset"])?;
        Ok(Anchor::Word {
            word: value
                .string("word")?
                .parse()
                .map_err(|_| bad_id("anchor.word"))?,
            transcript_hash: hash(value, "transcriptHash")?,
            edge: decode_edge(value)?,
            offset: optional_frame(value, "offset")?,
        })
    } else {
        Err(bad(
            "anchor",
            "a pinned, clip-source, item, marker, or word anchor",
        ))
    }
}

fn encode_edge(edge: Edge) -> Json {
    text(match edge {
        Edge::Start => "start",
        Edge::End => "end",
    })
}

fn decode_edge(value: &Json) -> Result<Edge, PackageError> {
    match optional_string(value, "edge")?.as_deref() {
        None | Some("start") => Ok(Edge::Start),
        Some("end") => Ok(Edge::End),
        _ => Err(bad("anchor.edge", "start or end")),
    }
}

/// Effects as the package and the MCP read surface encode them.
pub fn encode_effects(effects: &[Effect]) -> Json {
    Json::Array(
        effects
            .iter()
            .map(|effect| {
                let mut params: Vec<_> = effect
                    .params
                    .iter()
                    .map(|param| {
                        let value = match &param.value {
                            EffectValue::Number(n) => Json::Float(*n),
                            EffectValue::Bool(b) => Json::Bool(*b),
                            EffectValue::Text(s) => text(s),
                            EffectValue::Keys(keys) => encode_keys(keys),
                        };
                        (param.name.clone(), value)
                    })
                    .collect();
                params.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                let mut fields = vec![("id", display(&effect.id)), ("type", text(&effect.kind))];
                push_bool(&mut fields, "enabled", effect.enabled);
                fields.push(("params", Json::Object(params)));
                object(fields)
            })
            .collect(),
    )
}

pub(super) fn decode_effects(value: &Json) -> Result<Vec<Effect>, PackageError> {
    let mut effects = Vec::new();
    for value in value.array_or_empty("effects")? {
        shape(value, &["id", "type", "enabled", "params"])?;
        // 0.3 click zoom was a render-time curve; 0.4 zooms are explicit camera segments
        // (intent.autoZoom makes them). The old setting is dropped on load.
        if value.field("type").and_then(Json::as_str) == Some("clickZoom") {
            continue;
        }
        let kind = value.field("type").and_then(Json::as_str).unwrap_or("");
        let mut params = Vec::new();
        for (name, value) in value
            .required("params")?
            .as_object()
            .ok_or_else(|| bad("effect.params", "an object"))?
        {
            let parameter = if let Some(n) = value.as_f64() {
                EffectValue::Number(n)
            } else if let Some(b) = value.as_bool() {
                EffectValue::Bool(b)
            } else if let Some(s) = value.as_str() {
                EffectValue::Text(s.into())
            } else if let Some(keys) = decode_keys(value) {
                // 0.3 accepted keys everywhere and rendered the default where the evaluator
                // did not sample them; 0.4 keeps the first key's value there (T36.4).
                let keyable = crate::commands::effect_schemas()
                    .field(kind)
                    .and_then(|schema| schema.field("params"))
                    .and_then(|params| params.field(name))
                    .is_none_or(crate::commands::keyable);
                if keyable {
                    EffectValue::Keys(keys)
                } else {
                    EffectValue::Number(keys[0].value)
                }
            } else {
                return Err(bad(name, "a number, boolean, string or {keys}"));
            };
            params.push(EffectParam {
                name: name.clone(),
                value: parameter,
            });
        }
        effects.push(Effect {
            id: value
                .string("id")?
                .parse()
                .map_err(|_| bad_id("effect.id"))?,
            kind: value.string("type")?.into(),
            enabled: optional_bool(value, "enabled")?,
            params,
        });
    }
    Ok(effects)
}

/// `{"keys":[{"at","value","ease"}]}`; `ease` is a name or `[x1, y1, x2, y2]`.
pub fn encode_keys(keys: &[Key]) -> Json {
    object(vec![(
        "keys",
        Json::Array(
            keys.iter()
                .map(|key| {
                    object(vec![
                        ("at", frame_json(key.at)),
                        ("value", Json::Float(key.value)),
                        ("ease", encode_ease(key.ease)),
                    ])
                })
                .collect(),
        ),
    )])
}

/// Parses `encode_keys` output: at least one key, frames strictly increasing, finite values,
/// Bezier x control points within 0…1. `None` when the shape is wrong.
pub fn decode_keys(value: &Json) -> Option<Vec<Key>> {
    let fields = value.as_object()?;
    if fields.len() != 1 {
        return None;
    }
    let list = value.field("keys")?.as_array()?;
    let mut keys = Vec::with_capacity(list.len());
    for entry in list {
        let fields = entry.as_object()?;
        if fields
            .iter()
            .any(|(name, _)| !matches!(name.as_str(), "at" | "value" | "ease"))
        {
            return None;
        }
        let at = Frame::try_from(entry.field("at")?.as_i64()?).ok()?;
        let value = entry.field("value")?.as_f64().filter(|v| v.is_finite())?;
        let ease = match entry.field("ease") {
            None => Ease::Linear,
            Some(ease) => decode_ease(ease)?,
        };
        if keys.last().is_some_and(|last: &Key| last.at >= at) {
            return None;
        }
        keys.push(Key { at, value, ease });
    }
    (!keys.is_empty()).then_some(keys)
}

/// An ease: a name, `[x1, y1, x2, y2]`, `{"steps":n}` or `{"spring":{"duration","bounce"}}`.
pub fn encode_ease(ease: Ease) -> Json {
    match ease {
        Ease::Bezier(points) => Json::Array(points.iter().map(|&p| Json::Float(p)).collect()),
        Ease::Steps(n) => object(vec![("steps", Json::Unsigned(u64::from(n)))]),
        Ease::Spring { duration, bounce } => object(vec![(
            "spring",
            object(vec![
                ("duration", Json::Float(duration)),
                ("bounce", Json::Float(bounce)),
            ]),
        )]),
        other => text(other.name().expect("named ease")),
    }
}

/// Parses `encode_ease` output; `None` when the shape or a range is wrong. A spring's bounce
/// defaults to 0 (critically damped).
pub fn decode_ease(value: &Json) -> Option<Ease> {
    let in_range = |v: f64, [lo, hi]: [f64; 2]| v.is_finite() && v >= lo && v <= hi;
    match value {
        Json::Array(points) if points.len() == 4 => {
            let mut p = [0.0; 4];
            for (slot, point) in p.iter_mut().zip(points) {
                *slot = point.as_f64().filter(|v| v.is_finite())?;
            }
            if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[2]) {
                return None;
            }
            Some(Ease::Bezier(p))
        }
        Json::Object(fields) if fields.len() == 1 => {
            if let Some(n) = value.field("steps") {
                let n = u32::try_from(n.as_u64()?).ok()?;
                return (keys::STEPS[0]..=keys::STEPS[1])
                    .contains(&n)
                    .then_some(Ease::Steps(n));
            }
            let spring = value.field("spring")?;
            let names = spring.as_object()?;
            if names
                .iter()
                .any(|(name, _)| !matches!(name.as_str(), "duration" | "bounce"))
            {
                return None;
            }
            let duration = spring.field("duration")?.as_f64()?;
            let bounce = match spring.field("bounce") {
                Some(bounce) => bounce.as_f64()?,
                None => 0.0,
            };
            (in_range(duration, keys::SPRING_DURATION) && in_range(bounce, keys::SPRING_BOUNCE))
                .then_some(Ease::Spring { duration, bounce })
        }
        _ => Ease::parse(value.as_str()?),
    }
}

fn encode_rect(rect: Rect) -> Json {
    object(vec![
        ("x", Json::Float(rect.x)),
        ("y", Json::Float(rect.y)),
        ("w", Json::Float(rect.w)),
        ("h", Json::Float(rect.h)),
    ])
}

fn decode_rect(value: &Json) -> Result<Rect, PackageError> {
    shape(value, &["x", "y", "w", "h"])?;
    Ok(Rect {
        x: number(value, "x")?,
        y: number(value, "y")?,
        w: number(value, "w")?,
        h: number(value, "h")?,
    })
}
