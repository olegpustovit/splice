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
                let word = word_span(project, word, Some(transcript_hash), None)?
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

/// Source frames the first `frames` timeline frames of a clip consume: its source rate times the
/// integral of its `speed` effect's rate. A freeze (rate 0) consumes none, so edits of a frozen
/// clip never run out of source.
pub fn consumed(project: &Project, slot: Slot, frames: f64) -> Result<f64, TimingError> {
    let data = project.clips.data(slot).ok_or(TimingError::Slot)?;
    let rate = data
        .appearance
        .effects
        .iter()
        .find(|e| e.kind == "speed" && e.enabled != Some(false))
        .and_then(|e| e.params.iter().find(|p| p.name == "rate"))
        .map_or(frames, |rate| match &rate.value {
            EffectValue::Number(r) => r * frames,
            EffectValue::Keys(keys) => crate::keys::integrate(keys, frames),
            _ => frames,
        });
    Ok(rate * source_rate(project, slot)?)
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

/// Finds a visible occurrence of a word, optionally restricted to a transcript hash. A media item
/// can play in several clips (a split take, a duplicate): the occurrence in `near` wins, then the
/// first one at or after `near`, then the first one in the sequence.
pub fn word_span(
    project: &Project,
    id: WordId,
    hash: Option<u64>,
    near: Option<ClipId>,
) -> Result<Option<WordSpan>, TimingError> {
    let Some(media) = id.media() else {
        return Ok(None);
    };
    let Some(transcript) = project
        .transcripts
        .iter()
        .find(|t| t.media == media && hash.is_none_or(|hash| hash == t.hash))
    else {
        return Ok(None);
    };
    let Some(word) = transcript.words.iter().find(|word| word.id == id) else {
        return Ok(None);
    };
    let near_start = near
        .and_then(|clip| project.clips.find(clip))
        .and_then(|slot| project.clips.timing(slot))
        .map(|timing| timing.start);
    let rank = |span: &WordSpan| {
        if Some(span.clip) == near {
            (0, span.start)
        } else if near_start.is_some_and(|near| span.start >= near) {
            (1, span.start)
        } else {
            (2, span.start)
        }
    };
    let mut first: Option<WordSpan> = None;
    for &slot in project.clips.order() {
        if project.clips.data(slot).ok_or(TimingError::Slot)?.media != Some(media) {
            continue;
        }
        if let Some(span) = span_in(project, slot, transcript.hash, word)?
            && first.as_ref().is_none_or(|first| rank(&span) < rank(first))
        {
            first = Some(span);
        }
    }
    Ok(first)
}

/// Visits every transcript word a clip shows, in source order, with its span in that clip.
pub fn visible_words(
    project: &Project,
    slot: Slot,
    visit: &mut dyn FnMut(&Word, WordSpan),
) -> Result<(), TimingError> {
    let Some(media) = project.clips.data(slot).ok_or(TimingError::Slot)?.media else {
        return Ok(());
    };
    let Some(transcript) = project.transcripts.iter().find(|t| t.media == media) else {
        return Ok(());
    };
    for word in &transcript.words {
        if let Some(span) = span_in(project, slot, transcript.hash, word)? {
            visit(word, span);
        }
    }
    Ok(())
}

/// Where a clip shows a word of its media, or `None` when the word is outside its source range.
fn span_in(
    project: &Project,
    slot: Slot,
    hash: u64,
    word: &Word,
) -> Result<Option<WordSpan>, TimingError> {
    let timing = project.clips.timing(slot).ok_or(TimingError::Slot)?;
    let rate = source_rate(project, slot)?;
    let source_end = f64::from(timing.source) + consumed(project, slot, f64::from(timing.length))?;
    if word.from < timing.source || f64::from(word.from) >= source_end {
        return Ok(None);
    }
    let start = timeline_frame(timing, rate, f64::from(word.from))?;
    let end = timeline_frame(timing, rate, f64::from(word.to).min(source_end))?
        .max(start.checked_add(1).ok_or(TimingError::Range)?);
    Ok(Some(WordSpan {
        clip: project.clips.id(slot).ok_or(TimingError::Slot)?,
        source: word.from,
        hash,
        start,
        end,
    }))
}
