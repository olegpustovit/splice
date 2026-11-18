use super::{PackageError, codec::bad};
use splice_model::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

pub(super) fn project(project: &Project) -> Result<(), PackageError> {
    if project.fps == 0 || project.size.contains(&0) {
        return Err(bad("fps and canvas dimensions", "positive integers"));
    }
    let mut media = BTreeMap::new();
    for descriptor in &project.media {
        if media.insert(descriptor.id.serial(), descriptor).is_some() {
            return Err(bad("media IDs", "unique"));
        }
        if descriptor.path.is_empty() || descriptor.path.contains('\0') {
            return Err(bad("media.path", "a nonempty path without NUL"));
        }
        if descriptor.probe.fps <= 0.0 || descriptor.probe.frames < 0 {
            return Err(bad("probe", "a positive fps and nonnegative frame count"));
        }
        if let Some(take) = &descriptor.take {
            for path in [
                &take.screen,
                &take.camera,
                &take.mic,
                &take.system,
                &take.events,
                &take.cursors,
            ]
            .into_iter()
            .flatten()
            {
                if path.is_empty()
                    || path.contains('\0')
                    || !Path::new(path)
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                {
                    return Err(bad("take component path", "relative to the take directory"));
                }
            }
        }
    }

    let mut lanes = BTreeMap::new();
    if project.spine.id != LaneId::SPINE {
        return Err(bad("spine.id", "lan_spine"));
    }
    let mut effect_ids = BTreeSet::new();
    effects(&project.effects, &mut effect_ids)?;
    effects(&project.spine.effects, &mut effect_ids)?;
    for lane in &project.lanes {
        if lane.id == project.spine.id || lanes.insert(lane.id.serial(), lane.kind).is_some() {
            return Err(bad("lane IDs", "unique"));
        }
        effects(&lane.effects, &mut effect_ids)?;
    }
    let mut markers = BTreeSet::new();
    for marker in &project.markers {
        if marker.at < 0 || !markers.insert(marker.id.serial()) {
            return Err(bad("markers", "unique IDs at nonnegative frames"));
        }
    }

    let mut end = 0;
    for &slot in project.clips.order() {
        let timing = project.clips.timing(slot).expect("live clip");
        let data = project.clips.data(slot).expect("live clip");
        if timing.start < end || timing.source < 0 || timing.length <= 0 || timing.speed <= 0.0 {
            return Err(bad(
                "clips",
                "ordered, non-overlapping intervals with valid source and speed",
            ));
        }
        end = timing
            .end()
            .ok_or_else(|| bad("clip end", "an integer frame in range"))?;
        if let Some(id) = data.media {
            let descriptor = media
                .get(&id.serial())
                .ok_or_else(|| bad("clip.media", "a media table ID"))?;
            let source_frames = f64::from(timing.length) * timing.speed * descriptor.probe.fps
                / f64::from(project.fps)
                + f64::from(timing.source);
            if source_frames > f64::from(descriptor.probe.frames) + 0.000_001 {
                return Err(bad("clip source range", "within its media duration"));
            }
        }
        appearance(&data.appearance, timing.length, &mut effect_ids)?;
        crop(data.framing.crop)?;
    }
    for &slot in project.items.order() {
        let timing = project.items.timing(slot).expect("live item");
        let data = project.items.data(slot).expect("live item");
        if timing.length <= 0 || lanes.get(&data.lane.serial()) != Some(&data.kind.lane_kind()) {
            return Err(bad("item", "a positive-length item on a lane of its kind"));
        }
        if let ItemKind::Media(overlay) = data.kind {
            if overlay.source < 0 || !media.contains_key(&overlay.media.serial()) {
                return Err(bad(
                    "overlay",
                    "a media table ID and nonnegative source frame",
                ));
            }
            crop(overlay.framing.crop)?;
        }
        appearance(&data.appearance, timing.length, &mut effect_ids)?;
    }
    anchors(project, &markers)?;

    let mut transcripts = BTreeSet::new();
    for transcript in &project.transcripts {
        if !media.contains_key(&transcript.media.serial())
            || !transcripts.insert(transcript.media.serial())
        {
            return Err(bad("transcript", "one transcript per known media ID"));
        }
        let mut words = BTreeSet::new();
        for word in &transcript.words {
            if word.from < 0
                || word.to <= word.from
                || !words.insert(word.id.index())
                || word
                    .confidence
                    .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence))
            {
                return Err(bad(
                    "word",
                    "a unique ID with a source interval and optional confidence from 0 to 1",
                ));
            }
        }
    }
    Ok(())
}

fn anchors(project: &Project, markers: &BTreeSet<u64>) -> Result<(), PackageError> {
    let items: BTreeMap<_, _> = project
        .items
        .order()
        .iter()
        .map(|&slot| (project.items.id(slot).expect("live item").serial(), slot))
        .collect();
    let mut checked = BTreeSet::new();
    for &start in project.items.order() {
        let mut path = Vec::new();
        let mut slot = start;
        while checked.insert(slot) {
            path.push(slot);
            match project.items.timing(slot).expect("live item").anchor {
                Anchor::ClipSource { clip, source } => {
                    if source < 0 || project.clips.find(clip).is_none() {
                        return Err(bad(
                            "clip anchor",
                            "an existing clip ID and nonnegative source frame",
                        ));
                    }
                }
                Anchor::Item {
                    item: TimelineId::Clip(id),
                    ..
                } => {
                    if project.clips.find(id).is_none() {
                        return Err(bad("item anchor", "an existing clip ID"));
                    }
                }
                Anchor::Item {
                    item: TimelineId::Item(id),
                    ..
                } => {
                    slot = *items
                        .get(&id.serial())
                        .ok_or_else(|| bad("item anchor", "an existing item ID"))?;
                    continue;
                }
                Anchor::Marker { marker, .. }
                | Anchor::Item {
                    item: TimelineId::Marker(marker),
                    ..
                } => {
                    if !markers.contains(&marker.serial()) {
                        return Err(bad("marker anchor", "an existing marker ID"));
                    }
                }
                Anchor::Pinned(_) | Anchor::Word { .. } => {}
            }
            path.clear();
            break;
        }
        if path.contains(&slot) {
            return Err(bad("anchors", "acyclic"));
        }
    }
    Ok(())
}

fn crop(crop: Option<Rect>) -> Result<(), PackageError> {
    if let Some(crop) = crop
        && (crop.x < 0.0
            || crop.y < 0.0
            || crop.w <= 0.0
            || crop.h <= 0.0
            || crop.x + crop.w > 1.0 + f64::EPSILON
            || crop.y + crop.h > 1.0 + f64::EPSILON)
    {
        return Err(bad(
            "crop",
            "a positive area within normalized source coordinates",
        ));
    }
    Ok(())
}

fn appearance(
    appearance: &Appearance,
    length: Frame,
    effect_ids: &mut BTreeSet<u64>,
) -> Result<(), PackageError> {
    if appearance.fade_in < 0
        || appearance.fade_out < 0
        || appearance.fade_in > length - appearance.fade_out
    {
        return Err(bad("fades", "nonnegative and within the item length"));
    }
    effects(&appearance.effects, effect_ids)
}

fn effects(effects: &[Effect], ids: &mut BTreeSet<u64>) -> Result<(), PackageError> {
    for effect in effects {
        if !ids.insert(effect.id.serial()) {
            return Err(bad("effect IDs", "unique"));
        }
        let allowed = match effect.kind.as_str() {
            "window" => &[
                "padding",
                "corner",
                "shadow",
                "inset",
                "background",
                "color",
                "color2",
            ][..],
            "cursor" => &["size", "clickRings", "hidden"],
            "clickZoom" => &["scale", "hold"],
            "camera" => &["corner", "size", "roundness", "shadow"],
            "captions" => &[
                "preset",
                "size",
                "position",
                "color",
                "active",
                "stroke",
                "uppercase",
                "safe",
            ],
            "keystrokes" => &["size", "position"],
            _ => return Err(bad("effect.type", "a known effect type")),
        };
        let mut names = BTreeSet::new();
        for param in &effect.params {
            if !allowed.contains(&param.name.as_str()) {
                return Err(PackageError::new(
                    "FORMAT_UNKNOWN_KEY",
                    format!("unknown {} effect parameter {:?}", effect.kind, param.name),
                    "Use fields from this effect's schema.",
                ));
            }
            if !names.insert(param.name.as_str()) {
                return Err(bad("effect parameter names", "unique"));
            }
        }
    }
    Ok(())
}
