use super::*;
use crate::commands::{AnchorArg, FrameArg};
pub(super) use splice_model::timing::WordSpan;
use splice_model::timing::{self, TimingError};

/// Resolves the current frame range of a clip, overlay, or marker.
pub fn span(project: &Project, id: TimelineId) -> Result<Span, CommandError> {
    match id {
        TimelineId::Clip(clip) => {
            let slot = clip_slot(project, clip)?;
            let timing = project.clips.timing(slot).expect("live clip");
            Ok(Span {
                start: timing.start,
                end: timing.end().ok_or_else(frame_range)?,
                lane: None,
            })
        }
        TimelineId::Item(item) => {
            let slot = item_slot(project, item)?;
            let timing = project.items.timing(slot).expect("live item");
            let start = anchor_start(project, timing.anchor)?;
            Ok(Span {
                start,
                end: frame_add(start, timing.length)?,
                lane: Some(project.items.data(slot).expect("live item").lane),
            })
        }
        TimelineId::Marker(marker) => {
            let at = project
                .markers
                .iter()
                .find(|m| m.id == marker)
                .ok_or_else(|| not_found(&marker, "Call timeline.find for markers."))?
                .at;
            Ok(Span {
                start: at,
                end: at,
                lane: None,
            })
        }
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

fn timing_error(error_kind: TimingError) -> CommandError {
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
        TimingError::Transcript => error(
            "TRANSCRIPT_CHANGED",
            "The transcript hash has changed.",
            "Call transcript.get and use the current hash.",
        ),
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
        Ok(match input {
            FrameArg::Frame(frame) => Anchor::Pinned(*frame),
            FrameArg::Item { item, edge, offset } => Anchor::Item {
                item: self.timeline(item)?,
                edge: *edge,
                offset: *offset,
            },
            FrameArg::Marker { marker, offset } => {
                let TimelineId::Marker(marker) = self.timeline(marker)? else {
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
            } => Anchor::Word {
                word: *word,
                transcript_hash: need_word(&self.project, *word, *transcript_hash)?.hash,
                edge: *edge,
                offset: *offset,
            },
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
        anchor_start(&self.project, anchor)?;
        Ok(anchor)
    }
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
    let nearest = preceding.map(|(slot, _)| slot).or_else(|| {
        project
            .clips
            .order()
            .iter()
            .copied()
            .min_by_key(|&slot| project.clips.timing(slot).expect("live clip").start)
    });
    let Some(slot) = nearest else {
        return Ok(Anchor::Pinned(at.max(0)));
    };
    let timing = project.clips.timing(slot).expect("live clip");
    let source = if preceding.is_some() {
        last_source(project, slot)?
    } else {
        timing.source
    };
    Ok(Anchor::ClipSource {
        clip: project.clips.id(slot).expect("live clip"),
        source,
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

pub(super) fn source_value(project: &Project, slot: Slot, at: Frame) -> Result<f64, CommandError> {
    let timing = project.clips.timing(slot).expect("live clip");
    Ok(f64::from(timing.source)
        + (f64::from(at) - f64::from(timing.start)) * source_rate(project, slot)?)
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
) -> Result<Option<WordSpan>, CommandError> {
    timing::word_span(project, id, hash).map_err(timing_error)
}

fn need_word(project: &Project, id: WordId, hash: Option<u64>) -> Result<WordSpan, CommandError> {
    if let Some(hash) = hash
        && !project.transcripts.iter().any(|t| t.hash == hash)
    {
        return Err(error(
            "TRANSCRIPT_CHANGED",
            "The transcript hash has changed.",
            "Call transcript.get and use the current hash.",
        ));
    }
    word_span(project, id, hash)?.ok_or_else(|| {
        not_found(
            &id,
            "Call transcript.get and choose a word still in the timeline.",
        )
    })
}

pub(super) fn timeline_ids(project: &Project) -> Vec<TimelineId> {
    project
        .clips
        .order()
        .iter()
        .map(|&slot| TimelineId::Clip(project.clips.id(slot).expect("live clip")))
        .chain(
            project
                .items
                .order()
                .iter()
                .map(|&slot| TimelineId::Item(project.items.id(slot).expect("live item"))),
        )
        .chain(
            project
                .markers
                .iter()
                .map(|marker| TimelineId::Marker(marker.id)),
        )
        .collect()
}

pub(super) fn diff(before: &Project, after: &Project) -> Result<Vec<Diff>, CommandError> {
    let mut ids = timeline_ids(before);
    for id in timeline_ids(after) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    let mut changed = Vec::new();
    for id in ids {
        let old = span(before, id).map(Some).or_else(absent)?;
        let new = span(after, id).map(Some).or_else(absent)?;
        if old != new || properties_changed(before, after, id) {
            changed.push(Diff {
                id,
                before: old,
                after: new,
            });
        }
    }
    Ok(changed)
}

fn absent(error: CommandError) -> Result<Option<Span>, CommandError> {
    if error.code == "NOT_FOUND" {
        Ok(None)
    } else {
        Err(error)
    }
}

fn properties_changed(before: &Project, after: &Project, id: TimelineId) -> bool {
    match id {
        TimelineId::Clip(id) => match (before.clips.find(id), after.clips.find(id)) {
            (Some(a), Some(b)) => {
                before.clips.data(a) != after.clips.data(b)
                    || before.clips.timing(a) != after.clips.timing(b)
            }
            _ => true,
        },
        TimelineId::Item(id) => match (before.items.find(id), after.items.find(id)) {
            (Some(a), Some(b)) => {
                before.items.data(a) != after.items.data(b)
                    || before.items.timing(a) != after.items.timing(b)
            }
            _ => true,
        },
        TimelineId::Marker(id) => {
            before.markers.iter().find(|m| m.id == id) != after.markers.iter().find(|m| m.id == id)
        }
    }
}
