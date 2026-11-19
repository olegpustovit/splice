use super::{PackageError, codec::*, timeline};
use splice_json::Value as Json;
use splice_model::*;

pub(crate) fn encode(p: &Project) -> Json {
    let mut fields = vec![
        ("format", text(FORMAT)),
        ("rev", Json::Unsigned(p.rev)),
        ("hash", hash_text(p.hash)),
        ("serial", Json::Unsigned(p.serial)),
        ("fps", Json::Unsigned(u64::from(p.fps))),
        (
            "size",
            Json::Array(
                p.size
                    .into_iter()
                    .map(|n| Json::Unsigned(u64::from(n)))
                    .collect(),
            ),
        ),
        (
            "aspect",
            text(match p.aspect {
                Aspect::Portrait => "9:16",
                Aspect::Social => "4:5",
                Aspect::Square => "1:1",
                Aspect::Standard => "4:3",
                Aspect::Landscape => "16:9",
            }),
        ),
        (
            "access",
            text(match p.access {
                Access::Off => "off",
                Access::Read => "read",
                Access::Live => "live",
            }),
        ),
        ("effects", timeline::encode_effects(&p.effects)),
        (
            "spine",
            object(vec![
                ("id", display(&p.spine.id)),
                ("name", text(&p.spine.name)),
                ("effects", timeline::encode_effects(&p.spine.effects)),
            ]),
        ),
        (
            "lanes",
            Json::Array(p.lanes.iter().map(encode_lane).collect()),
        ),
        (
            "clips",
            Json::Array(
                p.clips
                    .order()
                    .iter()
                    .map(|&slot| {
                        timeline::encode_clip(
                            p.clips.id(slot).expect("live clip"),
                            p.clips.timing(slot).expect("live clip"),
                            p.clips.data(slot).expect("live clip"),
                        )
                    })
                    .collect(),
            ),
        ),
        (
            "items",
            Json::Array(
                p.items
                    .order()
                    .iter()
                    .map(|&slot| {
                        timeline::encode_item(
                            p.items.id(slot).expect("live item"),
                            p.items.timing(slot).expect("live item"),
                            p.items.data(slot).expect("live item"),
                        )
                    })
                    .collect(),
            ),
        ),
        (
            "markers",
            Json::Array(p.markers.iter().map(encode_marker).collect()),
        ),
    ];
    if !p.compounds.is_empty() {
        fields.push((
            "compounds",
            Json::Array(p.compounds.iter().map(encode_compound).collect()),
        ));
    }
    object(fields)
}

fn encode_compound(c: &Compound) -> Json {
    object(vec![
        ("media", display(&c.media)),
        (
            "clips",
            Json::Array(
                c.clips
                    .iter()
                    .map(|clip| timeline::encode_clip(clip.id, clip.timing, &clip.data))
                    .collect(),
            ),
        ),
        (
            "items",
            Json::Array(
                c.items
                    .iter()
                    .map(|item| timeline::encode_item(item.id, item.timing, &item.data))
                    .collect(),
            ),
        ),
        (
            "lanes",
            Json::Array(c.lanes.iter().map(encode_lane).collect()),
        ),
    ])
}

fn decode_compound(value: &Json) -> Result<Compound, PackageError> {
    shape(value, &["media", "clips", "items", "lanes"])?;
    Ok(Compound {
        media: value
            .string("media")?
            .parse()
            .map_err(|_| bad_id("compound.media"))?,
        clips: value
            .array("clips")?
            .iter()
            .map(timeline::decode_clip)
            .collect::<Result<_, _>>()?,
        items: value
            .array("items")?
            .iter()
            .map(timeline::decode_item)
            .collect::<Result<_, _>>()?,
        lanes: value
            .array("lanes")?
            .iter()
            .map(decode_lane)
            .collect::<Result<_, _>>()?,
    })
}

pub(crate) fn decode(value: &Json) -> Result<Project, PackageError> {
    check_version(value)?;
    shape(
        value,
        &[
            "format",
            "rev",
            "hash",
            "serial",
            "fps",
            "size",
            "aspect",
            "access",
            "effects",
            "spine",
            "lanes",
            "clips",
            "items",
            "markers",
            "compounds",
        ],
    )?;
    let values = value.array("size")?;
    let [width, height] = values else {
        return Err(bad("size", "a width and height"));
    };
    let dimension = |n: &Json| {
        n.as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| bad("size", "unsigned dimensions"))
    };
    let spine = value.required("spine")?;
    shape(spine, &["id", "name", "effects"])?;
    let clips = value.array("clips")?;
    let items = value.array("items")?;
    let mut project = Project {
        rev: u64_field(value, "rev")?,
        hash: hash(value, "hash")?,
        serial: u64_field(value, "serial")?,
        fps: u32_field(value, "fps")?,
        size: [dimension(width)?, dimension(height)?],
        aspect: match value.string("aspect")? {
            "9:16" => Aspect::Portrait,
            "4:5" => Aspect::Social,
            "1:1" => Aspect::Square,
            "4:3" => Aspect::Standard,
            "16:9" => Aspect::Landscape,
            _ => return Err(bad("aspect", "9:16, 4:5, 1:1, 4:3, or 16:9")),
        },
        access: match value.string("access")? {
            "off" => Access::Off,
            "read" => Access::Read,
            "live" => Access::Live,
            _ => return Err(bad("access", "off, read, or live")),
        },
        effects: timeline::decode_effects(value)?,
        spine: Spine {
            id: spine
                .string("id")?
                .parse()
                .map_err(|_| bad_id("spine.id"))?,
            name: spine.string("name")?.into(),
            effects: timeline::decode_effects(spine)?,
        },
        lanes: value
            .array("lanes")?
            .iter()
            .map(decode_lane)
            .collect::<Result<Vec<_>, _>>()?,
        clips: Clips::with_capacity(clips.len()),
        items: Items::with_capacity(items.len()),
        markers: value
            .array("markers")?
            .iter()
            .map(decode_marker)
            .collect::<Result<Vec<_>, _>>()?,
        media: Vec::new(),
        transcripts: Vec::new(),
        compounds: value
            .array_or_empty("compounds")?
            .iter()
            .map(decode_compound)
            .collect::<Result<Vec<_>, _>>()?,
    };
    for value in clips {
        let clip = timeline::decode_clip(value)?;
        if project.clips.insert(clip).is_none() {
            return Err(PackageError::invalid(
                &"duplicate clip ID or too many clips",
            ));
        }
    }
    for value in items {
        let item = timeline::decode_item(value)?;
        if project.items.insert(item).is_none() {
            return Err(PackageError::invalid(
                &"duplicate item ID or too many items",
            ));
        }
    }
    Ok(project)
}

fn encode_lane(lane: &Lane) -> Json {
    object(vec![
        ("id", display(&lane.id)),
        ("name", text(&lane.name)),
        (
            "kind",
            text(match lane.kind {
                LaneKind::Graphics => "graphics",
                LaneKind::Overlay => "overlay",
                LaneKind::Motion => "motion",
                LaneKind::Captions => "captions",
                LaneKind::Audio => "audio",
            }),
        ),
        ("effects", timeline::encode_effects(&lane.effects)),
    ])
}

fn decode_lane(value: &Json) -> Result<Lane, PackageError> {
    shape(value, &["id", "name", "kind", "effects"])?;
    let kind = match value.string("kind")? {
        "graphics" => LaneKind::Graphics,
        "overlay" => LaneKind::Overlay,
        "motion" => LaneKind::Motion,
        "captions" => LaneKind::Captions,
        "audio" => LaneKind::Audio,
        _ => return Err(bad("lane.kind", "graphics, overlay, motion, or captions")),
    };
    Ok(Lane {
        id: value.string("id")?.parse().map_err(|_| bad_id("lane.id"))?,
        name: value.string("name")?.into(),
        kind,
        effects: timeline::decode_effects(value)?,
    })
}

fn encode_marker(marker: &Marker) -> Json {
    let mut fields = vec![
        ("id", display(&marker.id)),
        ("at", frame_json(marker.at)),
        ("name", text(&marker.name)),
    ];
    push_text(&mut fields, "color", &marker.color);
    object(fields)
}

fn decode_marker(value: &Json) -> Result<Marker, PackageError> {
    shape(value, &["id", "at", "name", "color"])?;
    Ok(Marker {
        id: value
            .string("id")?
            .parse()
            .map_err(|_| bad_id("marker.id"))?,
        at: frame(value, "at")?,
        name: value.string("name")?.into(),
        color: optional_string(value, "color")?,
    })
}
