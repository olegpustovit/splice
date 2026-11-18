//! Shared, allocation-free queries for editing and playback.

use crate::*;

/// A timeline reference could not be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingError {
    /// Unknown clip ID.
    Clip(ClipId),
    /// Unknown overlay ID.
    Item(ItemId),
    /// Unknown marker ID.
    Marker(MarkerId),
    /// Unknown media ID.
    Media(MediaId),
    /// Word has no visible occurrence.
    Word(WordId),
    /// Requested transcript hash is unavailable.
    Transcript,
    /// Clip slot is vacant.
    Slot,
    /// Invalid project or source rate.
    Rate,
    /// Result exceeds the integer frame range.
    Range,
    /// Item anchors form a cycle.
    Cycle,
}

/// Finds an anchor's timeline frame, resolving nested references without recursion.
pub fn anchor_start(project: &Project, mut anchor: Anchor) -> Result<Frame, TimingError> {
    let mut offset = 0i64;
    for _ in 0..=project.items.order().len() {
        let base = match anchor {
            Anchor::Pinned(frame) => frame,
            Anchor::ClipSource { clip, source } => {
                let slot = project.clips.find(clip).ok_or(TimingError::Clip(clip))?;
                let timing = project.clips.timing(slot).ok_or(TimingError::Slot)?;
                timeline_frame(timing, source_rate(project, slot)?, f64::from(source))?
            }
            Anchor::Item {
                item: TimelineId::Item(id),
                edge,
                offset: delta,
            } => {
                let slot = project.items.find(id).ok_or(TimingError::Item(id))?;
                let timing = project.items.timing(slot).ok_or(TimingError::Item(id))?;
                let edge = if edge == Edge::End { timing.length } else { 0 };
                offset = offset
                    .checked_add(i64::from(delta) + i64::from(edge))
                    .ok_or(TimingError::Range)?;
                anchor = timing.anchor;
                continue;
            }
            Anchor::Item {
                item: TimelineId::Clip(id),
                edge,
                offset: delta,
            } => {
                let slot = project.clips.find(id).ok_or(TimingError::Clip(id))?;
                let timing = project.clips.timing(slot).ok_or(TimingError::Clip(id))?;
                offset = offset
                    .checked_add(i64::from(delta))
                    .ok_or(TimingError::Range)?;
                if edge == Edge::End {
                    timing.end().ok_or(TimingError::Range)?
                } else {
                    timing.start
                }
            }
            Anchor::Item {
                item: TimelineId::Marker(marker),
                offset: delta,
                ..
            }
            | Anchor::Marker {
                marker,
                offset: delta,
            } => {
                offset = offset
                    .checked_add(i64::from(delta))
                    .ok_or(TimingError::Range)?;
                project
                    .markers
                    .iter()
                    .find(|m| m.id == marker)
                    .ok_or(TimingError::Marker(marker))?
                    .at
            }
            Anchor::Word {
                word,
                transcript_hash,
                edge,
                offset: delta,
            } => {
                if !project
                    .transcripts
                    .iter()
                    .any(|t| t.hash == transcript_hash)
                {
                    return Err(TimingError::Transcript);
                }
                let word = word_span(project, word, Some(transcript_hash))?
                    .ok_or(TimingError::Word(word))?;
                offset = offset
                    .checked_add(i64::from(delta))
                    .ok_or(TimingError::Range)?;
                if edge == Edge::End {
                    word.end
                } else {
                    word.start
                }
            }
        };
        return i64::from(base)
            .checked_add(offset)
            .and_then(|frame| Frame::try_from(frame).ok())
            .ok_or(TimingError::Range);
    }
    Err(TimingError::Cycle)
}

/// Number of source frames consumed by one timeline frame.
pub fn source_rate(project: &Project, slot: Slot) -> Result<f64, TimingError> {
    let timing = project.clips.timing(slot).ok_or(TimingError::Slot)?;
    let data = project.clips.data(slot).ok_or(TimingError::Slot)?;
    let rate = match data.media {
        Some(id) => {
            let media = project
                .media
                .iter()
                .find(|media| media.id == id)
                .ok_or(TimingError::Media(id))?;
            media.probe.fps / f64::from(project.fps) * timing.speed
        }
        None => timing.speed,
    };
    if !rate.is_finite() || rate <= 0.0 {
        return Err(TimingError::Rate);
    }
    Ok(rate)
}

fn timeline_frame(timing: ClipTiming, source_rate: f64, frame: f64) -> Result<Frame, TimingError> {
    let at =
        (f64::from(timing.start) + (frame - f64::from(timing.source)) / source_rate + 0.5).floor();
    if (f64::from(Frame::MIN)..=f64::from(Frame::MAX)).contains(&at) {
        Ok(at as Frame)
    } else {
        Err(TimingError::Range)
    }
}

/// First visible occurrence of a transcript word in the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WordSpan {
    /// Clip that contains the word.
    pub clip: ClipId,
    /// Start on the source clock.
    pub source: Frame,
    /// Transcript version.
    pub hash: u64,
    /// Inclusive start in timeline frames.
    pub start: Frame,
    /// Exclusive end, clipped to the visible source interval.
    pub end: Frame,
}

/// Finds the first visible occurrence of a word, optionally restricted to a transcript hash.
pub fn word_span(
    project: &Project,
    id: WordId,
    hash: Option<u64>,
) -> Result<Option<WordSpan>, TimingError> {
    let mut first: Option<WordSpan> = None;
    for &slot in project.clips.order() {
        let data = project.clips.data(slot).ok_or(TimingError::Slot)?;
        let Some(media) = data.media else { continue };
        let Some(transcript) = project
            .transcripts
            .iter()
            .find(|t| t.media == media && hash.is_none_or(|hash| hash == t.hash))
        else {
            continue;
        };
        let Some(word) = transcript.words.iter().find(|word| word.id == id) else {
            continue;
        };
        let timing = project.clips.timing(slot).ok_or(TimingError::Slot)?;
        let rate = source_rate(project, slot)?;
        let source_end = f64::from(timing.source) + f64::from(timing.length) * rate;
        if word.from < timing.source || f64::from(word.from) >= source_end {
            continue;
        }
        let start = timeline_frame(timing, rate, f64::from(word.from))?;
        let end = timeline_frame(timing, rate, f64::from(word.to).min(source_end))?
            .max(start.checked_add(1).ok_or(TimingError::Range)?);
        if first.as_ref().is_none_or(|first| start < first.start) {
            first = Some(WordSpan {
                clip: project.clips.id(slot).ok_or(TimingError::Slot)?,
                source: word.from,
                hash: transcript.hash,
                start,
                end,
            });
        }
    }
    Ok(first)
}
