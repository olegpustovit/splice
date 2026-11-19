use super::{
    Json, object,
    project::{lane_kind, media_name},
    text,
    wire::*,
};
use splice_core::{
    commands::effect_schemas,
    edit::{Span, span},
    history::StoreError,
    package,
};
use splice_eval::{CaptionWord, caption_tokens, resolve_effect};
use splice_model::{timing, *};

pub(super) fn duration(p: &Project) -> Result<Frame, StoreError> {
    let mut end = p
        .clips
        .order()
        .last()
        .and_then(|&slot| p.clips.timing(slot))
        .and_then(ClipTiming::end)
        .unwrap_or(0);
    for &slot in p.items.order() {
        end = end.max(span(p, TimelineId::Item(p.items.id(slot).unwrap()))?.end);
    }
    Ok(end)
}

pub(super) fn window(args: &Json, end: Frame) -> Result<(Frame, Frame), StoreError> {
    let from = args.frame("from")?.unwrap_or(0);
    let to = args.frame("to")?.unwrap_or(end.max(from));
    if to < from {
        return Err(bad("to", "a frame at or after from"));
    }
    Ok((from, to))
}

struct Row {
    id: TimelineId,
    span: Span,
    value: Json,
}

impl Row {
    fn label(&self, p: &Project) -> Result<Json, StoreError> {
        let name = self.value.field("name").and_then(Json::as_str);
        match self.id {
            TimelineId::Clip(id) => {
                let data = p.clips.data(p.clips.find(id).unwrap()).unwrap();
                Ok(name.map_or_else(
                    || {
                        data.media
                            .map_or_else(|| text("Gap"), |id| Json::String(media_name(p, id)))
                    },
                    text,
                ))
            }
            TimelineId::Item(id) => {
                let slot = p.items.find(id).unwrap();
                let data = p.items.data(slot).unwrap();
                Ok(match &data.kind {
                    ItemKind::Title { text } => Json::String(text.clone()),
                    ItemKind::Caption { .. } if name.is_none() => {
                        Json::String(caption_text(&tokens(p, slot)?))
                    }
                    ItemKind::Media(overlay) if name.is_none() => {
                        Json::String(media_name(p, overlay.media))
                    }
                    ItemKind::Sound(sound) if name.is_none() => {
                        Json::String(media_name(p, sound.media))
                    }
                    ItemKind::Shape { shape, .. } => text(name.unwrap_or(shape.name())),
                    _ => text(name.unwrap_or("Scale animation")),
                })
            }
            TimelineId::Marker(_) => Ok(text(name.unwrap_or("Marker"))),
        }
    }

    fn kind(&self) -> &str {
        match self.id {
            TimelineId::Clip(_) => "clip",
            TimelineId::Item(_) => self.value.field("kind").and_then(Json::as_str).unwrap(),
            TimelineId::Marker(_) => "marker",
        }
    }

    fn intersects(&self, from: Frame, to: Frame) -> bool {
        if matches!(self.id, TimelineId::Marker(_)) {
            (from..to).contains(&self.span.start)
        } else {
            from < to && self.span.start < to && from < self.span.end
        }
    }

    fn describe(&self, p: &Project, full: bool, schemas: &Json) -> Result<Json, StoreError> {
        let label = self.label(p)?;
        if !full {
            let mut fields = vec![
                ("id", display(&self.id)),
                ("label", label),
                ("from", frame(self.span.start)),
                ("to", frame(self.span.end)),
            ];
            if let Some(anchor) = self.value.field("anchor") {
                fields.push(("anchor", anchor.clone()));
            }
            return Ok(object(fields));
        }
        let mut fields = vec![
            ("label", label),
            ("resolved", range(self.span.start, self.span.end)),
        ];
        if !matches!(self.id, TimelineId::Marker(_)) {
            fields.push(("children", children(p, self.id)));
            fields.push(("effectsResolved", effects(p, self.id, schemas)));
        }
        if let TimelineId::Item(id) = self.id
            && self.kind() == "caption"
        {
            let words = tokens(p, p.items.find(id).unwrap())?;
            fields.push(("text", Json::String(caption_text(&words))));
            fields.push(("tokens", caption_json(&words)));
        }
        Ok(extend(self.value.clone(), fields))
    }
}

fn rows(p: &Project) -> Result<Vec<Row>, StoreError> {
    let Json::Object(snapshot) = package::snapshot(p) else {
        unreachable!("project snapshot")
    };
    let mut rows =
        Vec::with_capacity(p.clips.order().len() + p.items.order().len() + p.markers.len());
    for (key, value) in snapshot {
        if !matches!(key.as_str(), "clips" | "items" | "markers") {
            continue;
        }
        let Json::Array(values) = value else {
            unreachable!("row array")
        };
        for value in values {
            let id = value.field("id").and_then(Json::as_str).unwrap();
            let id = match key.as_str() {
                "clips" => TimelineId::Clip(id.parse().unwrap()),
                "items" => TimelineId::Item(id.parse().unwrap()),
                _ => TimelineId::Marker(id.parse().unwrap()),
            };
            rows.push(Row {
                id,
                span: span(p, id)?,
                value,
            });
        }
    }
    Ok(rows)
}

pub(super) fn snapshot(p: &Project, args: &Json) -> Result<Json, StoreError> {
    let full = match args.optional_string("detail")? {
        None | Some("concise") => false,
        Some("full") => true,
        _ => return Err(bad("detail", "concise or full")),
    };
    let (from, to) = window(args, duration(p)?)?;
    let explicit_end = args.field("to").is_some();
    let rows = rows(p)?;
    let mut visible: Vec<_> = rows
        .iter()
        .filter(|row| !matches!(row.id, TimelineId::Marker(_)) && row.intersects(from, to))
        .collect();
    visible.sort_by_key(|row| {
        let lane = row.span.lane.map_or(0, |id| {
            1 + p
                .lanes
                .iter()
                .position(|lane| lane.id == id)
                .expect("checked lane")
        });
        (lane, row.span.start)
    });
    let total = visible.len();
    let start = args.cursor('c', total)?;
    let end = start.saturating_add(args.limit(200)?).min(total);
    let schemas = if full {
        effect_schemas().clone()
    } else {
        Json::Null
    };
    let mut spine = Vec::new();
    let mut lane_items: Vec<Vec<Json>> = vec![Vec::new(); p.lanes.len()];
    for row in &visible[start..end] {
        let value = row.describe(p, full, &schemas)?;
        if let Some(lane) = row.span.lane {
            let index = p.lanes.iter().position(|entry| entry.id == lane).unwrap();
            lane_items[index].push(value);
        } else {
            spine.push(value);
        }
    }
    let lanes = p
        .lanes
        .iter()
        .zip(lane_items)
        .map(|(lane, items)| {
            object(vec![
                ("id", display(&lane.id)),
                ("name", text(&lane.name)),
                ("kind", text(lane_kind(lane.kind))),
                ("items", Json::Array(items)),
            ])
        })
        .collect();
    let markers = rows
        .into_iter()
        .filter(|row| {
            matches!(row.id, TimelineId::Marker(_))
                && row.span.start >= from
                && (!explicit_end || row.span.start < to)
        })
        .map(|row| row.value)
        .collect();
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("fps", Json::Unsigned(u64::from(p.fps))),
        ("range", range(from, to)),
        ("lanes", Json::Array(lanes)),
        ("spine", Json::Array(spine)),
        ("markers", Json::Array(markers)),
        ("total", count(total)),
        ("nextCursor", next_cursor('c', end, total)),
    ]))
}

pub(super) fn inspect(p: &Project, args: &Json) -> Result<Json, StoreError> {
    // `items` is what every edit calls a list of IDs; agents reach for it here too.
    let ids = args
        .field("ids")
        .or_else(|| args.field("items"))
        .and_then(Json::as_array)
        .filter(|ids| (1..=50).contains(&ids.len()))
        .ok_or_else(|| {
            bad(
                "ids",
                "an array of 1 to 50 IDs (clp_, ovl_, lan_, mrk_ or fx_)",
            )
        })?;
    let rows = rows(p)?;
    let schemas = effect_schemas();
    let mut evaluator = None;
    let items = ids
        .iter()
        .map(|value| {
            let id = value
                .as_str()
                .ok_or_else(|| bad("ids", "an array of timeline ID strings"))?;
            if let Some(lane) = p.lanes.iter().find(|lane| lane.id.to_string() == id) {
                return Ok(object(vec![
                    ("id", text(id)),
                    ("kind", text("lane")),
                    ("laneKind", text(lane_kind(lane.kind))),
                    ("name", text(&lane.name)),
                    (
                        "effects",
                        splice_core::package::encode_effects(&lane.effects),
                    ),
                ]));
            }
            if let Some(effect) = id.starts_with("fx_").then(|| effect_json(p, id)).flatten() {
                return Ok(effect);
            }
            let row = rows.iter().find(|row| row.value.field("id") == Some(value));
            let Some(row) = row else {
                return Ok(object(vec![("id", text(id)), ("error", text("NOT_FOUND"))]));
            };
            let mut described = row.describe(p, true, schemas)?;
            if row.value.field("kind").and_then(Json::as_str) == Some("zoom")
                && let Json::Object(fields) = &mut described
            {
                if evaluator.is_none() {
                    evaluator = splice_eval::Evaluator::prepare(p).ok();
                }
                if let Some(evaluator) = &evaluator {
                    fields.push((
                        "fitSafeScale".into(),
                        Json::Float(fit_safe_scale(evaluator, row.span.start)),
                    ));
                }
            }
            Ok::<Json, StoreError>(described)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("items", Json::Array(items)),
    ]))
}

/// The largest zoom whose card stays on the canvas at `at` (T38.6): in frame mode the card
/// scales with the zoom, so past min(canvas / card) it overflows; content mode never does
/// (the camera's own limit, 5×).
pub(super) fn fit_safe_scale(evaluator: &splice_eval::Evaluator, at: splice_model::Frame) -> f64 {
    let mut plan = evaluator.buffer();
    evaluator.frame(at, &mut plan);
    let [cw, ch] = plan.size.map(f64::from);
    let card = plan.layers.iter().find_map(|layer| match layer {
        splice_eval::Layer::Picture(picture) if matches!(picture.item, TimelineId::Clip(_)) => {
            Some(picture.destination)
        }
        _ => None,
    });
    // A card that already fills the canvas looks the same zoomed either way: no limit.
    let safe = match card {
        Some(d)
            if !evaluator.camera_mode(at).zooms_content(ch > cw)
                && d.w > 0.0
                && d.h > 0.0
                && (d.w < cw - 0.5 || d.h < ch - 0.5) =>
        {
            (cw / d.w).min(ch / d.h)
        }
        _ => splice_model::Zoom::AMOUNT[1],
    };
    // Floored to a hundredth: a suggestion that is always safe to pass back.
    (safe.clamp(1.0, splice_model::Zoom::AMOUNT[1]) * 100.0).floor() / 100.0
}

pub(super) fn find(p: &Project, args: &Json) -> Result<Json, StoreError> {
    let kind = args.optional_string("kind")?;
    let query = args
        .optional_string("text")?
        .unwrap_or_default()
        .to_lowercase();
    let color = args.optional_string("color")?;
    let lane = args
        .optional_string("lane")?
        .map(|value| {
            value
                .parse::<LaneId>()
                .map_err(|_| bad("lane", "a lane ID from project.info"))
        })
        .transpose()?;
    let (from, to) = window(args, Frame::MAX)?;
    let want = |value: &str| kind.is_none_or(|kind| kind == value);
    let limit = args.limit(100)?;
    let mut found = Vec::new();
    let rows = rows(p)?;
    if kind == Some("edit") {
        let mut points: Vec<_> = rows
            .iter()
            .filter(|row| matches!(row.id, TimelineId::Clip(_)))
            .flat_map(|row| [row.span.start, row.span.end])
            .collect();
        points.sort_unstable();
        points.dedup();
        found.extend(
            points
                .into_iter()
                .filter(|at| (from..to).contains(at))
                .map(|at| object(vec![("kind", text("edit")), ("frame", frame(at))])),
        );
    } else {
        for row in rows {
            let lane_id = row
                .span
                .lane
                .or_else(|| matches!(row.id, TimelineId::Clip(_)).then_some(LaneId::SPINE));
            if !want(row.kind())
                || !row.intersects(from, to)
                || color.is_some_and(|color| {
                    row.value.field("color").and_then(Json::as_str) != Some(color)
                })
                || lane.is_some_and(|lane| lane_id != Some(lane))
            {
                continue;
            }
            let label = row.label(p)?;
            if !label.as_str().unwrap().to_lowercase().contains(&query) {
                continue;
            }
            let mut fields = vec![
                ("id", display(&row.id)),
                ("kind", text(row.kind())),
                ("label", label),
                ("frame", frame(row.span.start)),
            ];
            if let Some(lane) = row.span.lane {
                fields.push(("lane", display(&lane)));
            }
            found.push(object(fields));
        }
        if want("word") && !query.is_empty() && color.is_none() && lane.is_none() {
            for transcript in &p.transcripts {
                for word in transcript
                    .words
                    .iter()
                    .filter(|word| word.text.to_lowercase().contains(&query))
                {
                    let at = timing::word_span(p, word.id, Some(transcript.hash), None)
                        .map_err(|error| {
                            failure(
                                "INVARIANT",
                                &format!("{error:?}"),
                                "Check transcript and clip timing.",
                            )
                        })?
                        .map(|span| span.start);
                    if at.is_some_and(|at| !(from..to).contains(&at))
                        || (at.is_none()
                            && (args.field("from").is_some() || args.field("to").is_some()))
                    {
                        continue;
                    }
                    found.push(object(vec![
                        ("id", display(&word.id)),
                        ("kind", text("word")),
                        ("label", text(&word.text)),
                        ("frame", at.map_or(Json::Null, frame)),
                        ("hash", hash(transcript.hash)),
                    ]));
                }
            }
        }
    }
    let total = found.len();
    found.truncate(limit);
    Ok(object(vec![
        ("revision", rev(p.rev)),
        ("matches", Json::Array(found)),
        ("total", count(total)),
    ]))
}

fn children(p: &Project, parent: TimelineId) -> Json {
    Json::Array(
        p.items
            .order()
            .iter()
            .filter_map(|&slot| {
                let item = p.items.timing(slot)?.anchor;
                let target = match item {
                    Anchor::ClipSource { clip, .. } => TimelineId::Clip(clip),
                    Anchor::Item { item, .. } => item,
                    Anchor::Marker { marker, .. } => TimelineId::Marker(marker),
                    _ => return None,
                };
                (target == parent).then(|| display(&p.items.id(slot).expect("live item")))
            })
            .collect(),
    )
}

fn effects(p: &Project, id: TimelineId, schemas: &Json) -> Json {
    let (chain, types): ([(String, &[Effect]); 3], &[&str]) = match id {
        TimelineId::Clip(id) => {
            let data = p.clips.data(p.clips.find(id).unwrap()).unwrap();
            let Some(media) = data.media else {
                return object(Vec::new());
            };
            let is_take = p
                .media
                .iter()
                .any(|item| item.id == media && item.kind == MediaKind::Take);
            let types: &[&str] = if is_take {
                &["window", "cursor", "camera", "keystrokes"]
            } else {
                &["window"]
            };
            (
                [
                    ("project".into(), &p.effects),
                    (p.spine.id.to_string(), &p.spine.effects),
                    (id.to_string(), &data.appearance.effects),
                ],
                types,
            )
        }
        TimelineId::Item(id) => {
            let data = p.items.data(p.items.find(id).unwrap()).unwrap();
            let lane = p.lanes.iter().find(|lane| lane.id == data.lane).unwrap();
            let (global, types): (&[Effect], &[&str]) = match data.kind {
                ItemKind::Media(_) => (&[], &["window"]),
                ItemKind::Caption { .. } => (&p.effects, &["captions"]),
                _ => return object(Vec::new()),
            };
            (
                [
                    ("project".into(), global),
                    (lane.id.to_string(), &lane.effects),
                    (id.to_string(), &data.appearance.effects),
                ],
                types,
            )
        }
        TimelineId::Marker(_) => return object(Vec::new()),
    };
    let layers = [chain[0].1, chain[1].1, chain[2].1];
    let mut resolved = Vec::new();
    for &kind in types {
        let Some((enabled, overrides)) = resolve_effect(&layers, kind) else {
            continue;
        };
        let schema = schemas
            .field(kind)
            .and_then(|value| value.field("params"))
            .and_then(Json::as_object)
            .unwrap();
        let mut params = Vec::new();
        let mut sources = Vec::new();
        for (key, definition) in schema {
            if let Some(param) = overrides.iter().find(|param| &param.name == key) {
                let scope = chain
                    .iter()
                    .rev()
                    .find(|(_, entries)| {
                        entries
                            .iter()
                            .find(|effect| effect.kind == kind)
                            .is_some_and(|effect| {
                                effect.params.iter().any(|param| &param.name == key)
                            })
                    })
                    .unwrap();
                let value = match &param.value {
                    EffectValue::Text(s) => text(s),
                    EffectValue::Number(n) => Json::Float(*n),
                    EffectValue::Bool(flag) => Json::Bool(*flag),
                    EffectValue::Keys(keys) => splice_core::package::encode_keys(keys),
                };
                params.push((key.clone(), value));
                sources.push((key.clone(), text(&scope.0)));
            } else if let Some(value) = definition.field("def") {
                params.push((key.clone(), value.clone()));
                sources.push((key.clone(), text("default")));
            }
        }
        let scopes = chain
            .iter()
            .filter(|(_, entries)| entries.iter().any(|effect| effect.kind == kind))
            .map(|(name, _)| text(name))
            .collect();
        resolved.push((
            kind,
            object(vec![
                ("enabled", Json::Bool(enabled)),
                ("params", Json::Object(params)),
                ("scopes", Json::Array(scopes)),
                ("from", Json::Object(sources)),
            ]),
        ));
    }
    object(resolved)
}

pub(super) fn tokens(p: &Project, slot: Slot) -> Result<Vec<CaptionWord>, StoreError> {
    let data = p.items.data(slot).expect("live item");
    let ItemKind::Caption { words, edits } = &data.kind else {
        return Ok(Vec::new());
    };
    let timing = p.items.timing(slot).unwrap();
    caption_tokens(p, timing.anchor, words, edits, false).map_err(|error| {
        failure(
            "INVARIANT",
            &error.to_string(),
            "Check caption anchors and transcript timing.",
        )
    })
}

pub(super) fn caption_text(words: &[CaptionWord]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn caption_json(words: &[CaptionWord]) -> Json {
    Json::Array(
        words
            .iter()
            .map(|word| {
                let mut fields = vec![
                    ("id", display(&word.id)),
                    ("text", text(&word.text)),
                    ("from", frame(word.start)),
                    ("to", frame(word.end)),
                ];
                if word.emph {
                    fields.push(("emph", Json::Bool(true)));
                }
                if let Some(emoji) = &word.emoji {
                    fields.push(("emoji", text(emoji)));
                }
                object(fields)
            })
            .collect(),
    )
}

/// An effect by ID, encoded as in the package with its kind and owner's ID ("project" or
/// "lan_spine" for those scopes).
fn effect_json(p: &Project, id: &str) -> Option<Json> {
    let wanted: EffectId = id.parse().ok()?;
    let lists = [
        ("project".to_owned(), &p.effects),
        ("lan_spine".to_owned(), &p.spine.effects),
    ]
    .into_iter()
    .chain(
        p.lanes
            .iter()
            .map(|lane| (lane.id.to_string(), &lane.effects)),
    )
    .chain(p.clips.order().iter().map(|&slot| {
        let id = p.clips.id(slot).expect("live clip").to_string();
        (
            id,
            &p.clips.data(slot).expect("live clip").appearance.effects,
        )
    }))
    .chain(p.items.order().iter().map(|&slot| {
        let id = p.items.id(slot).expect("live item").to_string();
        (
            id,
            &p.items.data(slot).expect("live item").appearance.effects,
        )
    }));
    for (owner, effects) in lists {
        let Some(effect) = effects.iter().find(|effect| effect.id == wanted) else {
            continue;
        };
        let Json::Array(mut one) =
            splice_core::package::encode_effects(std::slice::from_ref(effect))
        else {
            unreachable!("effects encode as an array")
        };
        let Json::Object(mut fields) = one.remove(0) else {
            unreachable!("an effect encodes as an object")
        };
        fields.insert(1, ("kind".into(), text("effect")));
        fields.insert(2, ("owner".into(), text(&owner)));
        return Some(Json::Object(fields));
    }
    None
}
