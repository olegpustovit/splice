use super::*;
use crate::commands::{AnchorArg, FrameArg};
pub(super) use splice_model::timing::WordSpan;
use splice_model::timing::{self, TimingError};

/// Resolves the current frame range of a clip, overlay, or marker.
pub fn span(project: &Project, id: TimelineId) -> Result<Span, CommandError> {
    match id {
        TimelineId::Clip(clip) => Row::Clip(clip_slot(project, clip)?).span(project),
        TimelineId::Item(item) => item_span(project, item_slot(project, item)?),
        TimelineId::Marker(marker) => {
            let index = project
                .markers
                .iter()
                .position(|m| m.id == marker)
                .ok_or_else(|| not_found(&marker, "Call timeline.find for markers."))?;
            Row::Marker(index).span(project)
        }
    }
}

pub(super) fn item_span(project: &Project, slot: Slot) -> Result<Span, CommandError> {
    let timing = project.items.timing(slot).expect("live item");
    let start = anchor_start(project, timing.anchor)?;
    Ok(Span {
        start,
        end: frame_add(start, timing.length)?,
        lane: Some(project.items.data(slot).expect("live item").lane),
    })
}

pub(super) fn find_item(items: &Items, id: ItemId, slot: Slot) -> Option<Slot> {
    if items.id(slot) == Some(id) {
        Some(slot)
    } else {
        items.find(id)
    }
}

pub(super) fn clip_slot(project: &Project, id: ClipId) -> Result<Slot, CommandError> {
    project
        .clips
        .find(id)
        .ok_or_else(|| not_found(&id, "Call timeline.snapshot for current clip IDs."))
}

pub(super) fn item_slot(project: &Project, id: ItemId) -> Result<Slot, CommandError> {
    project
        .items
        .find(id)
        .ok_or_else(|| not_found(&id, "Call timeline.snapshot for current item IDs."))
}

pub(super) fn anchor_start(project: &Project, anchor: Anchor) -> Result<Frame, CommandError> {
    timing::anchor_start(project, anchor).map_err(timing_error)
}

/// Resolves a timeline frame or anchor from a viewer request.
pub fn resolve_view_frame(project: &Project, value: &Json) -> Result<Frame, CommandError> {
    let input = commands::frame_arg(value, "at")?;
    let anchor = relative_anchor(project, &input, false, &|target| match target {
        Target::Timeline(id) => Ok(*id),
        _ => Err(CommandError::bad(
            "at",
            "a timeline ID or frame outside an edit batch",
        )),
    })?;
    match anchor {
        Anchor::Pinned(frame) => Ok(frame),
        _ => anchor_start(project, anchor),
    }
}

pub(super) fn timing_error(error_kind: TimingError) -> CommandError {
    match error_kind {
        TimingError::Clip(id) => not_found(&id, "Call timeline.snapshot for current clip IDs."),
        TimingError::Item(id) => not_found(&id, "Call timeline.snapshot for current item IDs."),
        TimingError::Marker(id) => not_found(&id, "Call timeline.find for markers."),
        TimingError::Media(id) => not_found(&id, "Call project.info for current media IDs."),
        TimingError::Word(id) => not_found(
            &id,
            "Call transcript.get and choose a word still in the timeline.",
        ),
        TimingError::Range => frame_range(),
        TimingError::Cycle => error(
            "BAD_ARGS",
            "The item anchors form a cycle.",
            "Anchor one of the items to a clip or a fixed frame.",
        ),
        TimingError::Transcript => transcript_changed(),
        TimingError::Rate | TimingError::Slot => error(
            "INVARIANT",
            "A clip has an invalid source rate or slot.",
            "Check the project and media frame rates.",
        ),
    }
}

impl Editor {
    pub(super) fn frame(&self, input: &FrameArg) -> Result<Frame, CommandError> {
        match input {
            FrameArg::Frame(frame) => Ok(*frame),
            _ => anchor_start(&self.project, self.relative_anchor(input)?),
        }
    }

    fn relative_anchor(&self, input: &FrameArg) -> Result<Anchor, CommandError> {
        relative_anchor(&self.project, input, self.replay.is_some(), &|target| {
            self.timeline(target)
        })
    }

    pub(super) fn explicit_anchor(
        &self,
        input: &AnchorArg,
        default: Frame,
    ) -> Result<Anchor, CommandError> {
        let anchor = match input {
            AnchorArg::Pinned(frame) => Anchor::Pinned(
                frame
                    .as_ref()
                    .map(|frame| self.frame(frame))
                    .transpose()?
                    .unwrap_or(default),
            ),
            AnchorArg::ClipSource { clip, source } => {
                let TimelineId::Clip(clip) = self.timeline(clip)? else {
                    return Err(wrong_kind("Use a spine clip ID in a source-frame anchor."));
                };
                if *source < 0 {
                    return Err(CommandError::bad(
                        "sourceFrame",
                        "a nonnegative source frame",
                    ));
                }
                Anchor::ClipSource {
                    clip,
                    source: *source,
                }
            }
            AnchorArg::At(position) => self.relative_anchor(position)?,
        };
        let at = anchor_start(&self.project, anchor)?;
        // H3: a word anchor becomes the clip frame under the word, so cutting the word or
        // replacing the transcript never leaves an item without a position.
        if matches!(anchor, Anchor::Word { .. }) {
            return anchor_at(&self.project, at);
        }
        Ok(anchor)
    }
}

/// Load-time half of H3: word anchors saved before 0.4 become clip anchors at the frame they
/// resolve to. One that no longer resolves (a stale transcript) is pinned where it can be found
/// last, frame 0.
pub(crate) fn resolve_word_anchors(project: &mut Project) {
    for index in 0..project.items.order().len() {
        let slot = project.items.order()[index];
        let anchor = project.items.timing(slot).expect("live item").anchor;
        if !matches!(anchor, Anchor::Word { .. }) {
            continue;
        }
        let to = timing::anchor_start(project, anchor)
            .ok()
            .and_then(|at| anchor_at(project, at).ok())
            .unwrap_or(Anchor::Pinned(0));
        project.items.timing_mut(slot).expect("live item").anchor = to;
    }
}

fn relative_anchor(
    project: &Project,
    input: &FrameArg,
    replay: bool,
    timeline: &dyn Fn(&Target) -> Result<TimelineId, CommandError>,
) -> Result<Anchor, CommandError> {
    Ok(match input {
        FrameArg::Frame(frame) => Anchor::Pinned(*frame),
        FrameArg::Item { item, edge, offset } => Anchor::Item {
            item: timeline(item)?,
            edge: *edge,
            offset: *offset,
        },
        FrameArg::Marker { marker, offset } => {
            let TimelineId::Marker(marker) = timeline(marker)? else {
                return Err(wrong_kind("Use a marker ID in a marker anchor."));
            };
            Anchor::Marker {
                marker,
                offset: *offset,
            }
        }
        FrameArg::Word {
            word,
            transcript_hash,
            edge,
            offset,
        } => {
            let word = bind_word(project, *word, *transcript_hash, replay)?;
            Anchor::Word {
                word,
                transcript_hash: need_word(project, word, *transcript_hash)?.hash,
                edge: *edge,
                offset: *offset,
            }
        }
    })
}

pub(super) fn anchor_at(project: &Project, at: Frame) -> Result<Anchor, CommandError> {
    let mut preceding = None;
    for &slot in project.clips.order() {
        let timing = project.clips.timing(slot).expect("live clip");
        if timing.contains(at) {
            let source = source_frame(project, slot, at)?;
            return Ok(Anchor::ClipSource {
                clip: project.clips.id(slot).expect("live clip"),
                source,
            });
        }
        if timing.start <= at && preceding.is_none_or(|(_, last_start)| timing.start > last_start) {
            preceding = Some((slot, timing.start));
        }
    }
    // In a gap or past the spine end (H2): keep the exact frame, following the clip before it
    // (its end edge) or, before the first clip, the clip after it (its start edge).
    let (slot, edge) = match preceding {
        Some((slot, _)) => (slot, Edge::End),
        None => match project
            .clips
            .order()
            .iter()
            .copied()
            .min_by_key(|&slot| project.clips.timing(slot).expect("live clip").start)
        {
            Some(slot) => (slot, Edge::Start),
            None => return Ok(Anchor::Pinned(at.max(0))),
        },
    };
    let timing = project.clips.timing(slot).expect("live clip");
    let base = match edge {
        Edge::End => timing.end().ok_or_else(frame_range)?,
        Edge::Start => timing.start,
    };
    Ok(Anchor::Item {
        item: TimelineId::Clip(project.clips.id(slot).expect("live clip")),
        edge,
        offset: at.checked_sub(base).ok_or_else(frame_range)?,
    })
}

pub(super) fn source_rate(project: &Project, slot: Slot) -> Result<f64, CommandError> {
    timing::source_rate(project, slot).map_err(timing_error)
}

pub(super) fn media(project: &Project, id: MediaId) -> Result<&Media, CommandError> {
    project
        .media
        .iter()
        .find(|media| media.id == id)
        .ok_or_else(|| not_found(&id, "Call project.info for current media IDs."))
}

/// Source frames the clip's first `frames` timeline frames consume, speed effect included (a
/// freeze consumes none: H4).
pub(super) fn consumed(project: &Project, slot: Slot, frames: f64) -> Result<f64, CommandError> {
    timing::consumed(project, slot, frames).map_err(timing_error)
}

/// Media frames of `media` that play in `frames` timeline frames at normal speed (M2: overlay
/// sources live on the media clock).
pub(super) fn media_frames(
    project: &Project,
    media_id: MediaId,
    frames: Frame,
) -> Result<Frame, CommandError> {
    let fps = media(project, media_id)?.probe.fps;
    rounded(f64::from(frames) * fps / f64::from(project.fps))
}

/// The source frame (fractional) a clip plays at timeline frame `at`, through its speed effect;
/// `timing::anchor_start` maps it back the same way, so anchors round-trip on ramped clips.
pub(super) fn source_value(project: &Project, slot: Slot, at: Frame) -> Result<f64, CommandError> {
    let timing = project.clips.timing(slot).expect("live clip");
    let local = f64::from(at) - f64::from(timing.start);
    // Before the clip there is no ramp to integrate: extend at the clip's base rate.
    let played = if local < 0.0 {
        local * source_rate(project, slot)?
    } else {
        consumed(project, slot, local)?
    };
    Ok(f64::from(timing.source) + played)
}

pub(super) fn source_frame(
    project: &Project,
    slot: Slot,
    at: Frame,
) -> Result<Frame, CommandError> {
    rounded(source_value(project, slot, at)?)
}

pub(super) fn last_source(project: &Project, slot: Slot) -> Result<Frame, CommandError> {
    let timing = project.clips.timing(slot).expect("live clip");
    let last = frame_add(timing.start, timing.length - 1)?;
    checked_frame(source_value(project, slot, last)?.floor())
}

pub(super) fn rounded(value: f64) -> Result<Frame, CommandError> {
    checked_frame((value + 0.5).floor())
}

fn checked_frame(integer: f64) -> Result<Frame, CommandError> {
    if (f64::from(Frame::MIN)..=f64::from(Frame::MAX)).contains(&integer) {
        Ok(integer as Frame)
    } else {
        Err(frame_range())
    }
}

pub(super) fn word_span(
    project: &Project,
    id: WordId,
    hash: Option<u64>,
    near: Option<ClipId>,
) -> Result<Option<WordSpan>, CommandError> {
    timing::word_span(project, id, hash, near).map_err(timing_error)
}

/// Binds a word ID from an op to its media (T37.1). A legacy `w<n>` names a word in whichever
/// transcript has that index; when several do, a replayed batch keeps its old meaning (the first
/// occurrence on the timeline) and a live one is refused as ambiguous.
pub(super) fn bind_word(
    project: &Project,
    word: WordId,
    hash: Option<u64>,
    replay: bool,
) -> Result<WordId, CommandError> {
    if word.media().is_some() {
        return Ok(word);
    }
    if let Some(hash) = hash
        && !project.transcripts.iter().any(|t| t.hash == hash)
    {
        return Err(transcript_changed());
    }
    let mut found: Option<(WordId, Frame)> = None;
    for transcript in &project.transcripts {
        if hash.is_some_and(|hash| hash != transcript.hash)
            || !transcript
                .words
                .iter()
                .any(|w| w.id.index() == word.index())
        {
            continue;
        }
        let bound = WordId::new(transcript.media, word.index());
        let start = word_span(project, bound, hash, None)?.map_or(Frame::MAX, |span| span.start);
        match found {
            None => found = Some((bound, start)),
            Some(_) if !replay => {
                return Err(CommandError::rule(
                    "AMBIGUOUS_WORD",
                    "word",
                    Some(Json::String(word.to_string())),
                    "a project-unique word ID like w:<media>:<n>",
                    "Several transcripts have this word number. Use the id transcript.get returns.",
                ));
            }
            Some((_, old)) if start < old => found = Some((bound, start)),
            Some(_) => {}
        }
    }
    found.map(|(bound, _)| bound).ok_or_else(|| {
        not_found(
            &word,
            "Call transcript.get; word IDs look like w:<media>:<n>.",
        )
    })
}

pub(super) fn transcript_changed() -> CommandError {
    error(
        "TRANSCRIPT_CHANGED",
        "The transcript hash has changed.",
        "Call transcript.get and use the current hash.",
    )
}

fn need_word(project: &Project, id: WordId, hash: Option<u64>) -> Result<WordSpan, CommandError> {
    if let Some(hash) = hash
        && !project.transcripts.iter().any(|t| t.hash == hash)
    {
        return Err(transcript_changed());
    }
    word_span(project, id, hash, None)?.ok_or_else(|| {
        not_found(
            &id,
            "Call transcript.get and choose a word still in the timeline.",
        )
    })
}

#[derive(Clone, Copy)]
enum Row {
    Clip(Slot),
    Item(Slot),
    Marker(usize),
}

impl Row {
    fn span(self, project: &Project) -> Result<Span, CommandError> {
        match self {
            Self::Clip(slot) => {
                let timing = project.clips.timing(slot).expect("live clip");
                Ok(Span {
                    start: timing.start,
                    end: timing.end().ok_or_else(frame_range)?,
                    lane: None,
                })
            }
            Self::Item(slot) => item_span(project, slot),
            Self::Marker(index) => {
                let at = project.markers[index].at;
                Ok(Span {
                    start: at,
                    end: at,
                    lane: None,
                })
            }
        }
    }

    fn matching(self, id: TimelineId, project: &Project) -> Option<Self> {
        match (self, id) {
            (Self::Clip(slot), TimelineId::Clip(id)) => {
                if project.clips.id(slot) == Some(id) {
                    Some(self)
                } else {
                    project.clips.find(id).map(Self::Clip)
                }
            }
            (Self::Item(slot), TimelineId::Item(id)) => {
                find_item(&project.items, id, slot).map(Self::Item)
            }
            (Self::Marker(index), TimelineId::Marker(id)) => {
                if project
                    .markers
                    .get(index)
                    .is_some_and(|marker| marker.id == id)
                {
                    Some(self)
                } else {
                    project
                        .markers
                        .iter()
                        .position(|marker| marker.id == id)
                        .map(Self::Marker)
                }
            }
            _ => unreachable!("row kind matches timeline ID"),
        }
    }

    fn properties_changed(self, new: Self, before: &Project, after: &Project) -> bool {
        match (self, new) {
            (Self::Clip(a), Self::Clip(b)) => {
                before.clips.data(a) != after.clips.data(b)
                    || before.clips.timing(a) != after.clips.timing(b)
            }
            (Self::Item(a), Self::Item(b)) => {
                before.items.data(a) != after.items.data(b)
                    || before.items.timing(a) != after.items.timing(b)
            }
            (Self::Marker(a), Self::Marker(b)) => before.markers[a] != after.markers[b],
            _ => unreachable!("matching rows have the same kind"),
        }
    }
}

pub(super) fn diff(
    before: &Project,
    after: &Project,
    changed: &mut Vec<Diff>,
) -> Result<(), CommandError> {
    changed.clear();
    visit_timeline(before, &mut |id, row| {
        let next = row.matching(id, after);
        let old = optional_span(row, before)?;
        let new = next
            .map(|row| optional_span(row, after))
            .transpose()?
            .flatten();
        if old != new || next.is_none_or(|next| row.properties_changed(next, before, after)) {
            changed.push(Diff {
                id,
                before: old,
                after: new,
            });
        }
        Ok(())
    })?;
    visit_timeline(after, &mut |id, row| {
        if row.matching(id, before).is_none() {
            changed.push(Diff {
                id,
                before: None,
                after: optional_span(row, after)?,
            });
        }
        Ok(())
    })
}

fn optional_span(row: Row, project: &Project) -> Result<Option<Span>, CommandError> {
    match row.span(project) {
        Ok(span) => Ok(Some(span)),
        Err(error) if error.code == "NOT_FOUND" => Ok(None),
        Err(error) => Err(error),
    }
}

fn visit_timeline(
    project: &Project,
    visit: &mut dyn FnMut(TimelineId, Row) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    for &slot in project.clips.order() {
        visit(
            TimelineId::Clip(project.clips.id(slot).expect("live clip")),
            Row::Clip(slot),
        )?;
    }
    for &slot in project.items.order() {
        visit(
            TimelineId::Item(project.items.id(slot).expect("live item")),
            Row::Item(slot),
        )?;
    }
    for (index, marker) in project.markers.iter().enumerate() {
        visit(TimelineId::Marker(marker.id), Row::Marker(index))?;
    }
    Ok(())
}
