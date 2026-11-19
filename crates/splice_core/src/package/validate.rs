use super::{PackageError, codec::bad};
use splice_model::*;
use std::path::{Component, Path};

#[derive(Default)]
pub(crate) struct Scratch {
    media: Vec<u64>,
    lanes: Vec<(u64, LaneKind)>,
    markers: Vec<u64>,
    effects: Vec<u64>,
    marks: Vec<u8>,
    path: Vec<Slot>,
    transcripts: Vec<u64>,
    words: Vec<u32>,
}

pub(crate) fn project(project: &Project) -> Result<(), PackageError> {
    with_scratch(project, &mut Scratch::default())
}

pub(crate) fn with_scratch(project: &Project, scratch: &mut Scratch) -> Result<(), PackageError> {
    if project.fps == 0 || project.size.contains(&0) {
        return Err(bad("fps and canvas dimensions", "positive integers"));
    }
    let media = &mut scratch.media;
    media.clear();
    for descriptor in &project.media {
        insert_id(media, descriptor.id.serial(), "media IDs")?;
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

    let lanes = &mut scratch.lanes;
    lanes.clear();
    if project.spine.id != LaneId::SPINE {
        return Err(bad("spine.id", "lan_spine"));
    }
    let effect_ids = &mut scratch.effects;
    effect_ids.clear();
    effects(&project.effects, effect_ids)?;
    effects(&project.spine.effects, effect_ids)?;
    for lane in &project.lanes {
        if lane.id == project.spine.id {
            return Err(bad("lane IDs", "unique"));
        }
        lanes.push((lane.id.serial(), lane.kind));
        effects(&lane.effects, effect_ids)?;
    }
    lanes.sort_unstable_by_key(|&(id, _)| id);
    if lanes.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(bad("lane IDs", "unique"));
    }
    let markers = &mut scratch.markers;
    markers.clear();
    for marker in &project.markers {
        if marker.at < 0 {
            return Err(bad("markers", "nonnegative frames"));
        }
        insert_id(markers, marker.id.serial(), "marker IDs")?;
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
        if let Some(id) = data.media
            && !project.media.iter().any(|media| media.id == id)
        {
            return Err(bad("clip.media", "a media table ID"));
        }
        // A still has no duration of its own: it spans any clip length.
        if let Some(descriptor) = data
            .media
            .and_then(|id| project.media.iter().find(|media| media.id == id))
            .filter(|media| media.kind != MediaKind::Image)
        {
            // A speed ramp consumes the integral of its rate instead of the clip length.
            let source_frames =
                splice_model::timing::consumed(project, slot, f64::from(timing.length))
                    .map_err(|_| bad("clip speed", "a positive rate"))?
                    + f64::from(timing.source);
            if source_frames > f64::from(descriptor.probe.frames) + 0.001 {
                return Err(bad(
                    "clip source range",
                    "within its media duration (a faster speed ramp needs a shorter clip)",
                ));
            }
        }
        appearance(&data.appearance, timing.length, effect_ids)?;
        crop(data.framing.crop)?;
    }
    for &slot in project.items.order() {
        let timing = project.items.timing(slot).expect("live item");
        let data = project.items.data(slot).expect("live item");
        if timing.length <= 0
            || lanes
                .binary_search_by_key(&data.lane.serial(), |&(id, _)| id)
                .ok()
                .is_none_or(|index| lanes[index].1 != data.kind.lane_kind())
        {
            return Err(bad("item", "a positive-length item on a lane of its kind"));
        }
        if let ItemKind::Media(overlay) = data.kind {
            if overlay.source < 0 || media.binary_search(&overlay.media.serial()).is_err() {
                return Err(bad(
                    "overlay",
                    "a media table ID and nonnegative source frame",
                ));
            }
            crop(overlay.framing.crop)?;
        }
        appearance(&data.appearance, timing.length, effect_ids)?;
    }
    anchors(project, markers, &mut scratch.marks, &mut scratch.path)?;
    let transcripts = &mut scratch.transcripts;
    transcripts.clear();
    for transcript in &project.transcripts {
        if media.binary_search(&transcript.media.serial()).is_err() {
            return Err(bad("transcript", "a known media ID"));
        }
        insert_id(
            transcripts,
            transcript.media.serial(),
            "transcript media IDs",
        )?;
        let words = &mut scratch.words;
        words.clear();
        for word in &transcript.words {
            if word.id.media() != Some(transcript.media) {
                return Err(bad(
                    "word IDs",
                    "w:<media>:<n> naming the transcript's media",
                ));
            }
            if word.from < 0
                || word.to <= word.from
                || word
                    .confidence
                    .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence))
            {
                return Err(bad(
                    "word",
                    "a unique ID with a source interval and optional confidence from 0 to 1",
                ));
            }
            match words.binary_search(&word.id.index()) {
                Ok(_) => return Err(bad("word IDs", "unique")),
                Err(index) => words.insert(index, word.id.index()),
            }
        }
    }
    Ok(())
}

fn insert_id(ids: &mut Vec<u64>, id: u64, name: &str) -> Result<(), PackageError> {
    match ids.binary_search(&id) {
        Ok(_) => Err(bad(name, "unique")),
        Err(index) => {
            ids.insert(index, id);
            Ok(())
        }
    }
}

fn anchors(
    project: &Project,
    markers: &[u64],
    marks: &mut Vec<u8>,
    path: &mut Vec<Slot>,
) -> Result<(), PackageError> {
    marks.clear();
    marks.resize(
        project
            .items
            .order()
            .iter()
            .max()
            .map_or(0, |&slot| slot as usize + 1),
        0,
    );
    path.clear();
    for &start in project.items.order() {
        let mut slot = start;
        loop {
            match marks[slot as usize] {
                1 => return Err(bad("anchors", "acyclic")),
                2 => break,
                _ => {}
            }
            marks[slot as usize] = 1;
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
                    slot = project
                        .items
                        .find(id)
                        .ok_or_else(|| bad("item anchor", "an existing item ID"))?;
                    continue;
                }
                Anchor::Marker { marker, .. }
                | Anchor::Item {
                    item: TimelineId::Marker(marker),
                    ..
                } => {
                    if markers.binary_search(&marker.serial()).is_err() {
                        return Err(bad("marker anchor", "an existing marker ID"));
                    }
                }
                Anchor::Pinned(_) | Anchor::Word { .. } => {}
            }
            break;
        }
        for slot in path.drain(..) {
            marks[slot as usize] = 2;
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
    effect_ids: &mut Vec<u64>,
) -> Result<(), PackageError> {
    if appearance.fade_in < 0
        || appearance.fade_out < 0
        || appearance.fade_in > length - appearance.fade_out
    {
        return Err(bad("fades", "nonnegative and within the item length"));
    }
    effects(&appearance.effects, effect_ids)
}

fn effects(effects: &[Effect], ids: &mut Vec<u64>) -> Result<(), PackageError> {
    for effect in effects {
        insert_id(ids, effect.id.serial(), "effect IDs")?;
        crate::commands::check_effect(&effect.kind, &effect.params).map_err(|error| {
            PackageError::new(
                "FORMAT_UNKNOWN_KEY",
                format!("{} effect: {}", effect.kind, error.message),
                "Use fields and values from this effect's schema.",
            )
        })?;
        for (index, param) in effect.params.iter().enumerate() {
            if effect.params[..index]
                .iter()
                .any(|earlier| earlier.name == param.name)
            {
                return Err(bad("effect parameter names", "unique"));
            }
        }
    }
    Ok(())
}

/// Load-time migration (T37.1): binds legacy `w<n>` word IDs in captions and word anchors to the
/// media whose transcript holds them. A caption follows its clip's media; otherwise the transcript
/// that has the index wins. Transcripts must already be attached.
pub(crate) fn migrate_words(project: &mut Project) {
    let transcripts = &project.transcripts;
    let bind = |hint: Option<MediaId>, word: &mut WordId| {
        if word.media().is_some() {
            return;
        }
        let has = |t: &&Transcript| t.words.iter().any(|w| w.id.index() == word.index());
        let media = transcripts
            .iter()
            .filter(has)
            .find(|t| Some(t.media) == hint)
            .or_else(|| transcripts.iter().find(has))
            .map(|t| t.media);
        if let Some(media) = media {
            *word = WordId::new(media, word.index());
        }
    };
    let hint = |clips: &Clips, anchor: Anchor| match anchor {
        Anchor::ClipSource { clip, .. } => clips.find(clip).and_then(|s| clips.data(s)?.media),
        Anchor::Word {
            transcript_hash, ..
        } => transcripts
            .iter()
            .find(|t| t.hash == transcript_hash)
            .map(|t| t.media),
        _ => None,
    };
    let order = project.items.order().to_vec();
    for slot in order {
        let anchor = project.items.timing(slot).expect("live item").anchor;
        let media = hint(&project.clips, anchor);
        if let Anchor::Word { mut word, .. } = anchor {
            bind(media, &mut word);
            if let Some(Anchor::Word { word: stored, .. }) = project
                .items
                .timing_mut(slot)
                .map(|timing| &mut timing.anchor)
            {
                *stored = word;
            }
        }
        if let ItemKind::Caption { words, edits } =
            &mut project.items.data_mut(slot).expect("live item").kind
        {
            words.iter_mut().for_each(|word| bind(media, word));
            edits
                .iter_mut()
                .for_each(|edit| bind(media, &mut edit.word));
        }
    }
    for compound in &mut project.compounds {
        for item in &mut compound.items {
            if let Anchor::Word { word, .. } = &mut item.timing.anchor {
                bind(None, word);
            }
            if let ItemKind::Caption { words, edits } = &mut item.data.kind {
                words.iter_mut().for_each(|word| bind(None, word));
                edits.iter_mut().for_each(|edit| bind(None, &mut edit.word));
            }
        }
    }
}
