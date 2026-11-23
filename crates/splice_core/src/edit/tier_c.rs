//! 0.3 commands (PLAN.md T16–T21): the prototype's planned rows plus Splice's trim, time,
//! parameter, generator, template and media rows. Visual and audio treatments are effects
//! (`commands/effects.json`); these handlers only write effects, items and clip timing.
use super::edits::{arena_full, clamp_fades};
use super::settle::{Owner, effects, effects_vec_mut};
use super::tier_b::{applies, check_params};
use super::time::{anchor_at, clip_slot, item_slot, media, rounded, source_rate};
use super::*;
use crate::commands::{Basis, EffectSelection, FrameArg, OverlayContent};

/// Filler words (lowercase, without punctuation).
const FILLERS: &[&str] = &["um", "uh", "er", "erm", "ah", "hmm", "mm", "uhm"];

/// Whether a transcript word is a filler ("um", "uh", …).
pub fn is_filler(text: &str) -> bool {
    let word: String = text
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    FILLERS.contains(&word.as_str())
}

/// Abandoned attempts (T19.3): word index ranges of phrases a later phrase restarts with the
/// same opening words. `words` are (text, from, to) in source frames; a pause over `gap`
/// frames ends a phrase.
pub fn retakes(words: &[(&str, Frame, Frame)], gap: Frame) -> Vec<(usize, usize)> {
    let key = |text: &str| -> String {
        text.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let mut phrases: Vec<(usize, usize)> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        match phrases.last_mut() {
            Some(last) if word.1 - words[last.1 - 1].2 <= gap => last.1 = index + 1,
            _ => phrases.push((index, index + 1)),
        }
    }
    let opening = |(from, to): (usize, usize)| -> Vec<String> {
        words[from..to]
            .iter()
            .map(|w| key(w.0))
            .filter(|w| !w.is_empty() && !FILLERS.contains(&w.as_str()))
            .take(3)
            .collect()
    };
    let mut out = Vec::new();
    for (index, &phrase) in phrases.iter().enumerate() {
        let first = opening(phrase);
        if first.len() < 2 {
            continue;
        }
        let restarted = phrases[index + 1..].iter().take(3).any(|&later| {
            let next = opening(later);
            next.len() >= first.len() && next[..first.len()] == first[..]
        });
        if restarted {
            out.push(phrase);
        }
    }
    out
}

fn number(name: &str, value: f64) -> EffectParam {
    EffectParam {
        name: name.into(),
        value: EffectValue::Number(value),
    }
}

fn keyed(name: &str, keys: Vec<Key>) -> EffectParam {
    EffectParam {
        name: name.into(),
        value: EffectValue::Keys(keys),
    }
}

fn flag(name: &str, value: bool) -> EffectParam {
    EffectParam {
        name: name.into(),
        value: EffectValue::Bool(value),
    }
}

fn text_param(name: &str, value: &str) -> EffectParam {
    EffectParam {
        name: name.into(),
        value: EffectValue::Text(value.into()),
    }
}

pub(super) fn done(summary: String) -> Outcome {
    Outcome {
        summary: Summary::Text(summary),
        primary: None,
        created: Vec::new(),
    }
}

/// A transcript word or a burst of recorded activity on the timeline (D15).
pub(super) struct Spoken {
    pub(super) start: Frame,
    pub(super) end: Frame,
    pub(super) clip: ClipId,
    pub(super) filler: bool,
    /// The transcript word; `None` for activity.
    pub(super) id: Option<WordId>,
}

impl Editor {
    pub(super) fn run_tier_c(&mut self, op: Op) -> Result<Outcome, CommandError> {
        match op {
            Op::ClipJoin { items } => self.join(&items),
            Op::ClipUngroup { item } => self.ungroup(&item),
            Op::ClipRestore { items } => self.restore(&items),
            Op::LaneReorder { lane, to } => self.reorder_lane(&lane, to),
            Op::CropKeys { item, keys } => self.crop_keys(&item, keys),
            Op::TransitionAdd { item, params } => self.transition(&item, params, true),
            Op::TransitionSet { item, params } => self.transition(&item, params, false),
            Op::TakePointer { clip, marks } => self.take_pointer(&clip, &marks),
            Op::TransitionRemove { item } => {
                let owner = self.spine_owner(&item)?;
                let list = effects_vec_mut(&mut self.project, owner);
                let before = list.len();
                list.retain(|effect| effect.kind != "transition");
                if list.len() == before {
                    return Err(no_transition());
                }
                Ok(done(format!(
                    "Removed the transition into {}",
                    self.owner_label(owner)
                )))
            }
            Op::MaskAdd {
                item,
                mut params,
                span,
            } => {
                let owner = self.owner(&item)?;
                self.mask_span(owner, span, &mut params)?;
                let mut created = Vec::new();
                let id = self.upsert(owner, "mask", params, None, &mut created)?;
                Ok(Outcome {
                    summary: Summary::Text(format!(
                        "Added mask {id} on {}",
                        self.owner_label(owner)
                    )),
                    primary: Some(Target::Effect(id)),
                    created,
                })
            }
            Op::MaskSet {
                mask,
                mut params,
                enabled,
                span,
            } => {
                let (owner, _) = self.expect_kind(&mask, "mask")?;
                self.mask_span(owner, span, &mut params)?;
                self.set_effect(EffectSelection::Id(mask), params, enabled, &[])
            }
            Op::MaskTrack { mask, keys } => self.track_mask(&mask, &keys),
            Op::CaptionsTranslate {
                lane,
                language,
                texts,
            } => self.translate_captions(&lane, &language, &texts),
            Op::CaptionsImport { lane, text } => self.import_captions(lane.as_ref(), &text),
            Op::CaptionAdd {
                text,
                from,
                to,
                lane,
            } => self.add_caption(text, from, to, lane.as_ref()),
            Op::AudioUnlink { item, lane } => self.unlink_audio(&item, lane.as_ref()),
            Op::AudioLink { item } => self.link_audio(&item),
            Op::AudioSync { item, to, offset } => self.sync_audio(&item, &to, offset),
            Op::StyleSet { params } => self.set_style(params),
            Op::CutPauses {
                min,
                keep,
                basis,
                activity,
            } => {
                let (spans, active) = self.signal(basis, &activity)?;
                // Activity pauses are idle stretches: 2 s, keeping 0.25 s at each end.
                let (min_default, keep_default) = if active { (2.0, 0.25) } else { (0.7, 0.0) };
                let min = min.unwrap_or(self.seconds(min_default)).max(2);
                let keep = keep
                    .unwrap_or(if active {
                        self.seconds(keep_default)
                    } else {
                        4
                    })
                    .max(0);
                let (count, frames) = self.cut_gaps(&spans, min, keep, keep)?;
                Ok(done(format!(
                    "Cut {count} {}, {frames} frames",
                    if active {
                        "idle stretch(es)"
                    } else {
                        "pause(s)"
                    }
                )))
            }
            Op::IntentPace {
                max_pause,
                basis,
                activity,
            } => {
                let (spans, active) = self.signal(basis, &activity)?;
                let max = max_pause
                    .unwrap_or(self.seconds(if active { 1.0 } else { 0.4 }))
                    .max(0);
                let (count, frames) = self.cut_gaps(&spans, max + 1, max / 2, max - max / 2)?;
                Ok(done(format!(
                    "Tightened {count} pause(s) to {max} frames, {frames} frames shorter{}",
                    if active { " (activity)" } else { "" }
                )))
            }
            Op::IntentFitDuration {
                seconds,
                basis,
                activity,
            } => {
                let (spans, _) = self.signal(basis, &activity)?;
                self.fit_duration(seconds, &spans)
            }
            Op::IntentSteps {
                item,
                lead,
                settle,
                max_gap,
                retake_prefix,
                activity,
            } => self.steps(
                &item,
                [lead, settle, max_gap],
                retake_prefix.as_deref().unwrap_or("retake:"),
                &activity,
            ),
            Op::IntentSpeedTyping {
                item,
                factor,
                min_run,
                typing,
            } => self.speed_typing(&item, factor, min_run, &typing),
            Op::IntentKeepShots { ranges } => self.keep_ranges(&ranges),
            Op::IntentAutoZoom {
                item,
                clicks,
                typing,
                amount,
                lead,
                tail,
                window,
                follow,
                motion_blur,
            } => self.auto_zoom(
                &item,
                &clicks,
                &typing,
                AutoZoomShape {
                    amount: amount.unwrap_or(1.5),
                    lead,
                    tail,
                    window,
                    follow,
                    motion_blur,
                },
            ),
            Op::ItemRoll { item, by } => self.roll(&item, by),
            Op::ItemSlip { item, by } => self.slip(&item, by),
            Op::ItemSlide { item, by } => self.slide(&item, by),
            Op::ClipFreeze { item, at, length } => self.freeze(&item, &at, length),
            Op::ClipCompound { items } => self.compound(&items),
            Op::ClipUncompound { item } => self.uncompound(&item),
            Op::ParamSet {
                target,
                path,
                value,
            } => {
                let result = self.set_param(&target, &path, EffectValue::Number(value), false);
                self.decode_scratch.path = path;
                result
            }
            Op::ParamKeys {
                target,
                path,
                keys,
                merge,
            } => {
                let springs = super::motion::springs(&keys);
                let outcome = self.set_param(&target, &path, EffectValue::Keys(keys), merge)?;
                self.report_springs(springs);
                Ok(outcome)
            }
            Op::GeneratorAdd {
                shape,
                at,
                length,
                rect,
                lane,
                params,
            } => {
                let rect = rect.unwrap_or(match shape {
                    ShapeKind::Solid | ShapeKind::Gradient => Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 1.0,
                        h: 1.0,
                    },
                    ShapeKind::Arrow | ShapeKind::Line => Rect {
                        x: 0.35,
                        y: 0.4,
                        w: 0.3,
                        h: 0.2,
                    },
                    _ => Rect {
                        x: 0.3,
                        y: 0.35,
                        w: 0.4,
                        h: 0.3,
                    },
                });
                let outcome = self.add_item(
                    OverlayContent::Shape { shape, rect },
                    Some(&at),
                    None,
                    lane.as_ref(),
                    length,
                )?;
                if !params.is_empty() {
                    let target = outcome.primary.clone().expect("created shape");
                    let owner = self.owner(&target)?;
                    let mut created = Vec::new();
                    self.upsert(owner, "shape", params, None, &mut created)?;
                }
                Ok(outcome)
            }
            Op::TemplateAdd {
                name,
                at,
                length,
                fields,
            } => {
                let at = self.frame(&at)?;
                self.add_template(&name, at, length, &fields)
            }
            Op::TemplateSet { item, fields } => self.set_template(&item, &fields),
            Op::MediaConsolidate { media: targets } => {
                let mut ids = Vec::new();
                for target in &targets {
                    let Target::Media(id) = self.resolve(target)? else {
                        return Err(wrong_kind("Pass media IDs (med_…)."));
                    };
                    media(&self.project, id)?;
                    ids.push(id);
                }
                let mut count = 0;
                for descriptor in &mut self.project.media {
                    if (!ids.is_empty() && !ids.contains(&descriptor.id))
                        || !descriptor.path.starts_with('/')
                    {
                        continue;
                    }
                    descriptor.path = consolidated_path(descriptor.id, &descriptor.path);
                    count += 1;
                }
                Ok(done(format!("Consolidated {count} file(s) into media/")))
            }
            op @ (Op::ItemOrder { .. } | Op::MotionPreset { .. }) => self.run_motion(op),
            op @ (Op::CounterAdd { .. }
            | Op::ProgressAdd { .. }
            | Op::ParticlesAdd { .. }
            | Op::LookApply { .. }
            | Op::LookSave { .. }) => self.run_graphics(op),
            _ => unreachable!("tier C dispatch"),
        }
    }

    // ── Spine structure ──

    /// Merges declared pointer marks into the take's media, in source time (T39.4).
    fn take_pointer(
        &mut self,
        clip: &Target,
        marks: &[(Frame, f64, f64, bool)],
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(clip)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let media_id = self.project.clips.data(slot).expect("live clip").media;
        let index = media_id
            .and_then(|m| self.project.media.iter().position(|media| media.id == m))
            .filter(|&i| self.project.media[i].kind == MediaKind::Take)
            .ok_or_else(|| wrong_kind(&format!("{id} is not a screen take.")))?;
        let fps = self.project.media[index].probe.fps.max(1.0);
        let mut seconds = Vec::with_capacity(marks.len());
        for &(at, x, y, click) in marks {
            if !(timing.start..timing.start + timing.length).contains(&at) {
                return Err(CommandError::rule(
                    "OUT_OF_RANGE",
                    "marks",
                    Some(Json::Signed(i64::from(at))),
                    &format!(
                        "frames {}–{} (inside {id})",
                        timing.start,
                        timing.start + timing.length - 1
                    ),
                    "Declare marks inside the take's clip.",
                ));
            }
            let t = time::source_value(&self.project, slot, at)? / fps;
            seconds.push(PointerMark { t, x, y, click });
        }
        let take = self.project.media[index]
            .take
            .get_or_insert_with(TakeComponents::default);
        for mark in seconds {
            // A new mark at the same instant replaces the old one.
            take.pointer.retain(|old| (old.t - mark.t).abs() > 1e-6);
            take.pointer.push(mark);
        }
        take.pointer.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.say(format_args!(
            "Declared {} pointer marks on {id}",
            marks.len()
        ));
        Ok(Outcome {
            summary: Summary::Note,
            primary: Some(Target::Timeline(TimelineId::Clip(id))),
            created: Vec::new(),
        })
    }

    pub(super) fn spine_clip(&self, target: &Target) -> Result<(ClipId, Slot), CommandError> {
        let TimelineId::Clip(id) = self.timeline(target)? else {
            return Err(wrong_kind("Pass a spine clip (clp_…)."));
        };
        Ok((id, clip_slot(&self.project, id)?))
    }

    pub(super) fn spine_owner(&self, target: &Target) -> Result<Owner, CommandError> {
        Ok(Owner::Clip(self.spine_clip(target)?.1))
    }

    /// The spine slot after `slot`, when it starts where `slot` ends.
    fn next_adjacent(&self, slot: Slot) -> Option<Slot> {
        let order = self.project.clips.order();
        let index = order.iter().position(|&s| s == slot)?;
        let next = *order.get(index + 1)?;
        let a = self.project.clips.timing(slot)?;
        let b = self.project.clips.timing(next)?;
        (a.start + a.length == b.start).then_some(next)
    }

    fn previous_adjacent(&self, slot: Slot) -> Option<Slot> {
        let order = self.project.clips.order();
        let index = order.iter().position(|&s| s == slot)?;
        let previous = *order.get(index.checked_sub(1)?)?;
        (self.next_adjacent(previous) == Some(slot)).then_some(previous)
    }

    /// Source frame just past the clip's last frame.
    fn source_end(&self, slot: Slot) -> Result<f64, CommandError> {
        let timing = self.project.clips.timing(slot).expect("live clip");
        self.source_at(slot, timing.length)
    }

    /// Source frame `frames` timeline frames into the clip, speed effect included (H4).
    fn source_at(&self, slot: Slot, frames: Frame) -> Result<f64, CommandError> {
        let timing = self.project.clips.timing(slot).expect("live clip");
        Ok(f64::from(timing.source) + time::consumed(&self.project, slot, f64::from(frames))?)
    }

    fn media_frames(&self, slot: Slot) -> Option<f64> {
        let id = self.project.clips.data(slot)?.media?;
        media(&self.project, id)
            .ok()
            .map(|m| f64::from(m.probe.frames))
    }

    /// Whether `b` continues `a` in its source without a gap.
    fn continuous(&self, a: Slot, b: Slot) -> Result<bool, CommandError> {
        let (da, db) = (
            self.project.clips.data(a).expect("live clip"),
            self.project.clips.data(b).expect("live clip"),
        );
        let (ta, tb) = (
            self.project.clips.timing(a).expect("live clip"),
            self.project.clips.timing(b).expect("live clip"),
        );
        Ok(da.media.is_some()
            && da.media == db.media
            && ta.speed == tb.speed
            && (self.source_end(a)? - f64::from(tb.source)).abs() < 0.5)
    }

    fn reanchor_clip(&mut self, from: ClipId, to: ClipId) {
        for index in 0..self.project.items.order().len() {
            let slot = self.project.items.order()[index];
            let anchor = self.project.items.timing(slot).expect("live item").anchor;
            if let Anchor::ClipSource { clip, source } = anchor
                && clip == from
            {
                self.reanchor(slot, Anchor::ClipSource { clip: to, source });
            }
        }
    }

    /// Merges `b` into `a` (adjacent, continuous); records the seam.
    fn merge_clips(&mut self, a: Slot, b: Slot) -> Result<(), CommandError> {
        let id_a = self.project.clips.id(a).expect("live clip");
        let id_b = self.project.clips.id(b).expect("live clip");
        let tb = self.project.clips.timing(b).expect("live clip");
        let joins_b = self.project.clips.data(b).expect("live clip").joins.clone();
        let fade_out = self
            .project
            .clips
            .data(b)
            .expect("live clip")
            .appearance
            .fade_out;
        let ta = self.project.clips.timing_mut(a).expect("live clip");
        let seam = ta.length;
        ta.length = frame_add(ta.length, tb.length)?;
        let length = ta.length;
        let data = self.project.clips.data_mut(a).expect("live clip");
        data.joins.push(seam);
        data.joins.extend(joins_b.iter().map(|&at| at + seam));
        data.appearance.fade_out = fade_out;
        clamp_fades(&mut data.appearance, length);
        self.project.clips.remove(b);
        self.reanchor_clip(id_b, id_a);
        // Edge anchors at the seam (a's end, b's start) and b's end keep their frames (M5).
        for index in 0..self.project.items.order().len() {
            let slot = self.project.items.order()[index];
            if let Anchor::Item {
                item: TimelineId::Clip(clip),
                edge,
                offset,
            } = self.project.items.timing(slot).expect("live item").anchor
                && (clip == id_b || (clip == id_a && edge == Edge::End))
            {
                let (edge, offset) = match (clip == id_a, edge) {
                    (true, _) | (false, Edge::Start) => (Edge::Start, offset + seam),
                    (false, Edge::End) => (Edge::End, offset),
                };
                self.reanchor(
                    slot,
                    Anchor::Item {
                        item: TimelineId::Clip(id_a),
                        edge,
                        offset,
                    },
                );
            }
        }
        Ok(())
    }

    fn join(&mut self, items: &[Target]) -> Result<Outcome, CommandError> {
        if items.len() < 2 {
            return Err(CommandError::bad(
                "items",
                "two or more adjacent spine clips",
            ));
        }
        let mut slots = Vec::with_capacity(items.len());
        for target in items {
            slots.push(self.spine_clip(target)?.1);
        }
        slots.sort_by_key(|&slot| self.project.clips.timing(slot).expect("live clip").start);
        for pair in slots.windows(2) {
            if self.next_adjacent(pair[0]) != Some(pair[1]) || !self.continuous(pair[0], pair[1])? {
                return Err(error(
                    "NOT_JOINABLE",
                    "Only adjacent clips that continue one source at one speed can join.",
                    "Use clip.restore to bring back cut material between them first.",
                ));
            }
        }
        let first = slots[0];
        for &slot in &slots[1..] {
            self.merge_clips(first, slot)?;
        }
        let id = self.project.clips.id(first).expect("live clip");
        Ok(Outcome::edit(
            format!("Joined {} clips into {id}", slots.len()),
            Target::Timeline(TimelineId::Clip(id)),
        ))
    }

    fn ungroup(&mut self, item: &Target) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let mut joins =
            std::mem::take(&mut self.project.clips.data_mut(slot).expect("live clip").joins);
        if joins.is_empty() {
            return Err(error(
                "NOT_JOINED",
                &format!("{id} was not made by clip.join."),
                "Use item.split to cut a clip.",
            ));
        }
        let start = self.project.clips.timing(slot).expect("live clip").start;
        let length = self.project.clips.timing(slot).expect("live clip").length;
        joins.sort_unstable();
        let mut created = Vec::new();
        // A seam trimmed within MIN_LENGTH of an edge or another seam stays joined (M1).
        let mut end = length;
        for &at in joins.iter().rev() {
            if at < MIN_LENGTH || end - at < MIN_LENGTH {
                continue;
            }
            let right = self.split_clip(id, start + at, at)?;
            created.push(Target::Timeline(TimelineId::Clip(right)));
            end = at;
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Ungrouped {id} into {} clips", created.len() + 1)),
            primary: Some(Target::Timeline(TimelineId::Clip(id))),
            created,
        })
    }

    fn restore(&mut self, items: &[Target]) -> Result<Outcome, CommandError> {
        let mut wanted = Vec::new();
        for target in items {
            wanted.push(self.spine_clip(target)?.0);
        }
        let mut restored = 0;
        let mut frames = 0;
        let mut index = 0;
        while index + 1 < self.project.clips.order().len() {
            let a = self.project.clips.order()[index];
            let b = self.project.clips.order()[index + 1];
            let id_a = self.project.clips.id(a).expect("live clip");
            let id_b = self.project.clips.id(b).expect("live clip");
            let (da, db) = (
                self.project.clips.data(a).expect("live clip").media,
                self.project.clips.data(b).expect("live clip").media,
            );
            let (ta, tb) = (
                self.project.clips.timing(a).expect("live clip"),
                self.project.clips.timing(b).expect("live clip"),
            );
            let chosen = wanted.is_empty() || wanted.contains(&id_a) || wanted.contains(&id_b);
            let gap = f64::from(tb.source) - self.source_end(a)?;
            if !chosen
                || da.is_none()
                || da != db
                || ta.speed != tb.speed
                || ta.start + ta.length != tb.start
                || gap < 0.5
            {
                index += 1;
                continue;
            }
            let delta = rounded(gap / source_rate(&self.project, a)?)?;
            self.shift_after(tb.start, delta, None)?;
            self.project.clips.timing_mut(a).expect("live clip").length += delta;
            self.merge_clips(a, b)?;
            restored += 1;
            frames += delta;
        }
        if restored == 0 {
            return Err(error(
                "NOTHING_TO_RESTORE",
                "No seam between these clips hides cut material.",
                "Pass clips whose neighbors come from the same source with a gap.",
            ));
        }
        Ok(done(format!("Restored {restored} cut(s), {frames} frames")))
    }

    fn reorder_lane(&mut self, lane: &Target, to: usize) -> Result<Outcome, CommandError> {
        let Target::Lane(id) = self.resolve(lane)? else {
            return Err(wrong_kind("Pass a lane ID (lan_…)."));
        };
        let from = self
            .project
            .lanes
            .iter()
            .position(|lane| lane.id == id)
            .ok_or_else(|| not_found(&id, "project.info lists lanes."))?;
        let lane = self.project.lanes.remove(from);
        let to = to.min(self.project.lanes.len());
        self.project.lanes.insert(to, lane);
        self.say(format_args!("Moved {id} to position {to}"));
        Ok(Outcome::noted(Target::Lane(id)))
    }

    fn roll(&mut self, item: &Target, by: Frame) -> Result<Outcome, CommandError> {
        let (id, a) = self.spine_clip(item)?;
        let b = self.next_adjacent(a).ok_or_else(|| {
            error(
                "NO_NEIGHBOR",
                &format!("{id} has no clip right after it."),
                "Roll the edit point between two adjacent spine clips.",
            )
        })?;
        let ta = self.project.clips.timing(a).expect("live clip");
        let tb = self.project.clips.timing(b).expect("live clip");
        let rate_b = source_rate(&self.project, b)?;
        let source_b = f64::from(tb.source) + f64::from(by) * rate_b;
        let end_a = self.source_at(a, ta.length + by)?;
        if ta.length + by < MIN_LENGTH
            || tb.length - by < MIN_LENGTH
            || source_b < -0.001
            || self
                .media_frames(a)
                .is_some_and(|frames| end_a > frames + 0.001)
        {
            return Err(no_handles());
        }
        self.project.clips.timing_mut(a).expect("live clip").length += by;
        let timing = self.project.clips.timing_mut(b).expect("live clip");
        timing.start += by;
        timing.length -= by;
        timing.source = rounded(source_b)?;
        let effects = &mut self
            .project
            .clips
            .data_mut(b)
            .expect("live clip")
            .appearance
            .effects;
        super::edits::shift_keys(effects, by);
        self.say(format_args!("Rolled the edit after {id} by {by} frames"));
        Ok(Outcome::noted(Target::Timeline(TimelineId::Clip(id))))
    }

    fn slip(&mut self, item: &Target, by: Frame) -> Result<Outcome, CommandError> {
        match self.timeline(item)? {
            TimelineId::Clip(id) => {
                let slot = clip_slot(&self.project, id)?;
                let timing = self.project.clips.timing(slot).expect("live clip");
                let source = timing.source + by;
                let end = self.source_end(slot)? + f64::from(by);
                if source < 0
                    || self
                        .media_frames(slot)
                        .is_some_and(|frames| end > frames + 0.001)
                {
                    return Err(no_handles());
                }
                self.project
                    .clips
                    .timing_mut(slot)
                    .expect("live clip")
                    .source = source;
                self.say(format_args!("Slipped {id} by {by} source frames"));
                Ok(Outcome::noted(Target::Timeline(TimelineId::Clip(id))))
            }
            TimelineId::Item(id) => {
                let slot = item_slot(&self.project, id)?;
                let length = self.project.items.timing(slot).expect("live item").length;
                let ItemKind::Media(overlay) =
                    self.project.items.data(slot).expect("live item").kind
                else {
                    return Err(wrong_kind("Slip a spine clip or a media overlay."));
                };
                let frames = media(&self.project, overlay.media)?.probe.frames;
                let source = overlay.source + by;
                // `by` is in source frames; the item's length is in timeline frames (M2).
                let needed = time::media_frames(&self.project, overlay.media, length)?;
                if source < 0 || source + needed > frames {
                    return Err(no_handles());
                }
                if let ItemKind::Media(overlay) =
                    &mut self.project.items.data_mut(slot).expect("live item").kind
                {
                    overlay.source = source;
                }
                self.say(format_args!("Slipped {id} by {by} source frames"));
                Ok(Outcome::noted(Target::Timeline(TimelineId::Item(id))))
            }
            TimelineId::Marker(_) => Err(wrong_kind("Slip a spine clip or a media overlay.")),
        }
    }

    fn slide(&mut self, item: &Target, by: Frame) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let (Some(previous), Some(next)) = (self.previous_adjacent(slot), self.next_adjacent(slot))
        else {
            return Err(error(
                "NO_NEIGHBOR",
                &format!("{id} needs adjacent clips on both sides to slide."),
                "Use item.move to reposition a clip at the edge of the spine.",
            ));
        };
        let tp = self.project.clips.timing(previous).expect("live clip");
        let tn = self.project.clips.timing(next).expect("live clip");
        let end_p = self.source_at(previous, tp.length + by)?;
        let source_n = f64::from(tn.source) + f64::from(by) * source_rate(&self.project, next)?;
        if tp.length + by < MIN_LENGTH
            || tn.length - by < MIN_LENGTH
            || source_n < -0.001
            || self
                .media_frames(previous)
                .is_some_and(|f| end_p > f + 0.001)
        {
            return Err(no_handles());
        }
        self.project
            .clips
            .timing_mut(previous)
            .expect("live clip")
            .length += by;
        self.project
            .clips
            .timing_mut(slot)
            .expect("live clip")
            .start += by;
        let timing = self.project.clips.timing_mut(next).expect("live clip");
        timing.start += by;
        timing.length -= by;
        timing.source = rounded(source_n)?;
        // The next clip now starts `by` frames into its content: its keys follow (Low list).
        let effects = &mut self
            .project
            .clips
            .data_mut(next)
            .expect("live clip")
            .appearance
            .effects;
        super::edits::shift_keys(effects, by);
        self.say(format_args!("Slid {id} by {by} frames"));
        Ok(Outcome::noted(Target::Timeline(TimelineId::Clip(id))))
    }

    fn freeze(
        &mut self,
        item: &Target,
        at: &FrameArg,
        length: Frame,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let at = self.frame(at)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        if at < timing.start || at >= timing.start + timing.length {
            return Err(CommandError::bad("at", "a frame inside the clip"));
        }
        let held = rounded(time::source_value(&self.project, slot, at)?)?;
        let mut data = self.project.clips.data(slot).expect("live clip").clone();
        if at > timing.start {
            super::edits::check_split(timing.start, timing.start + timing.length, at)?;
            self.split_clip(id, at, at - timing.start)?;
        }
        self.shift_after(at, length, None)?;
        let freeze = ClipId::from_serial(self.mint("clp")?);
        data.appearance = Appearance {
            name: Some("Freeze frame".into()),
            effects: data
                .appearance
                .effects
                .iter()
                .filter(|effect| !matches!(effect.kind.as_str(), "transition" | "speed" | "volume"))
                .cloned()
                .collect(),
            ..Appearance::default()
        };
        data.joins.clear();
        for (kind, param) in [
            ("speed", number("rate", 0.0)),
            ("volume", flag("mute", true)),
        ] {
            let id = EffectId::from_serial(self.mint("fx")?);
            data.appearance.effects.push(Effect {
                id,
                kind: kind.into(),
                enabled: None,
                params: vec![param],
            });
        }
        self.project
            .clips
            .insert(Clip {
                id: freeze,
                timing: ClipTiming {
                    start: at,
                    length,
                    source: held,
                    speed: timing.speed,
                },
                data,
            })
            .ok_or_else(arena_full)?;
        self.project.clips.sort_by_start();
        Ok(Outcome::create(
            format!("Froze frame {at} of {id} for {length} frames"),
            Target::Timeline(TimelineId::Clip(freeze)),
        ))
    }

    // ── Parameters and effects ──

    fn expect_kind(&self, target: &Target, kind: &str) -> Result<(Owner, usize), CommandError> {
        let (owner, index) = self.find_effect(target)?;
        if effects(&self.project, owner)[index].kind != kind {
            return Err(wrong_kind(&format!("Pass a {kind} effect ID (fx_…).")));
        }
        Ok((owner, index))
    }

    /// A timed mask (T46.3): timeline `from`/`to` (exclusive) become the owner-relative
    /// `start`/`end` parameters, so the mask stays on its content when the owner moves. Both
    /// must fall inside the owner; nothing is clamped.
    fn mask_span(
        &self,
        owner: Owner,
        span: [Option<Frame>; 2],
        params: &mut Vec<EffectParam>,
    ) -> Result<(), CommandError> {
        if span == [None, None] {
            return Ok(());
        }
        let id = match owner {
            Owner::Clip(slot) => TimelineId::Clip(self.project.clips.id(slot).expect("live clip")),
            Owner::Item(slot) => TimelineId::Item(self.project.items.id(slot).expect("live item")),
            _ => {
                return Err(wrong_kind(
                    "Masks belong to a clip (clp_) or an overlay (ovl_).",
                ));
            }
        };
        let range = time::span(&self.project, id)?;
        for (name, value, key) in [("from", span[0], "start"), ("to", span[1], "end")] {
            let Some(frame) = value else { continue };
            let inside = if key == "start" {
                frame >= range.start && frame < range.end
            } else {
                frame > range.start && frame <= range.end
            };
            if !inside {
                return Err(CommandError::rule(
                    "OUT_OF_RANGE",
                    name,
                    Some(Json::Signed(i64::from(frame))),
                    &format!(
                        "a timeline frame within {id} ({}–{}, to exclusive)",
                        range.start, range.end
                    ),
                    &format!(
                        "Pass {name} between {} and {}: from/to are timeline frames, not \
                         frames of the item.",
                        range.start, range.end
                    ),
                ));
            }
            let relative = EffectValue::Number(f64::from(frame - range.start));
            match params.iter_mut().find(|param| param.name == key) {
                Some(param) => param.value = relative,
                None => params.push(EffectParam {
                    name: key.into(),
                    value: relative,
                }),
            }
        }
        Ok(())
    }

    pub(super) fn set_param(
        &mut self,
        target: &Target,
        path: &str,
        value: EffectValue,
        merge: bool,
    ) -> Result<Outcome, CommandError> {
        let (kind, name) = path.split_once('.').expect("decoded path");
        let resolved = self.resolve(target)?;
        let (owner, index) = if let Target::Effect(_) = resolved {
            let (owner, index) = self.find_effect(&resolved)?;
            if effects(&self.project, owner)[index].kind != kind {
                return Err(CommandError::bad(
                    "path",
                    &format!(
                        "a parameter of {}",
                        effects(&self.project, owner)[index].kind
                    ),
                ));
            }
            (owner, index)
        } else {
            let owner = self.owner(&resolved)?;
            let (class, _) = applies(kind)?;
            if !self.fits(class, owner) {
                return Err(wrong_kind(&format!(
                    "{kind} does not apply to {}.",
                    self.owner_label(owner)
                )));
            }
            match effects(&self.project, owner)
                .iter()
                .position(|effect| effect.kind == kind)
            {
                Some(index) => (owner, index),
                None => {
                    let mut created = Vec::new();
                    self.upsert(owner, kind, Vec::new(), None, &mut created)?;
                    let index = effects(&self.project, owner)
                        .iter()
                        .rposition(|effect| effect.kind == kind)
                        .expect("upserted");
                    (owner, index)
                }
            }
        };
        let value = match (value, merge) {
            (EffectValue::Keys(new), true) => {
                let mut keys = match effects(&self.project, owner)[index]
                    .params
                    .iter()
                    .find(|param| param.name == name)
                    .map(|param| &param.value)
                {
                    Some(EffectValue::Keys(old)) => old.clone(),
                    _ => Vec::new(),
                };
                for key in new {
                    match keys.iter_mut().find(|old| old.at == key.at) {
                        Some(old) => *old = key,
                        None => keys.push(key),
                    }
                }
                keys.sort_by_key(|key| key.at);
                EffectValue::Keys(keys)
            }
            (value, _) => value,
        };
        let effect = &mut effects_vec_mut(&mut self.project, owner)[index];
        match effect.params.iter_mut().position(|old| old.name == name) {
            Some(at) => {
                let old = std::mem::replace(&mut effect.params[at].value, value);
                if let Err(error) = check_params(&effect.kind, &effect.params[at..=at]) {
                    effect.params[at].value = old;
                    return Err(error);
                }
            }
            None => {
                let param = EffectParam {
                    name: name.into(),
                    value,
                };
                check_params(&effect.kind, std::slice::from_ref(&param))?;
                effect.params.push(param);
            }
        }
        let id = effect.id;
        if kind == "transition" {
            self.check_transition(owner)?;
        }
        let label = self.owner_label(owner);
        self.say(format_args!("Set {path} on {label}"));
        Ok(Outcome::noted(Target::Effect(id)))
    }

    fn crop_keys(
        &mut self,
        item: &Target,
        keys: Vec<(Frame, [f64; 4], Ease)>,
    ) -> Result<Outcome, CommandError> {
        let owner = self.owner(item)?;
        let channel = |i: usize| {
            keys.iter()
                .map(|&(at, rect, ease)| Key {
                    at,
                    value: rect[i],
                    ease,
                })
                .collect::<Vec<_>>()
        };
        let params = vec![
            keyed("x", channel(0)),
            keyed("y", channel(1)),
            keyed("w", channel(2)),
            keyed("h", channel(3)),
        ];
        let mut created = Vec::new();
        let id = self.upsert(owner, "crop", params, Some(true), &mut created)?;
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Animated crop with {} key(s) on {}",
                keys.len(),
                self.owner_label(owner)
            )),
            primary: Some(Target::Effect(id)),
            created,
        })
    }

    fn transition(
        &mut self,
        item: &Target,
        params: Vec<EffectParam>,
        add: bool,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let owner = Owner::Clip(slot);
        let exists = effects(&self.project, owner)
            .iter()
            .any(|effect| effect.kind == "transition");
        if !add && !exists {
            return Err(no_transition());
        }
        let mut created = Vec::new();
        // upsert runs check_transition (scope, neighbor, length, handles).
        let fx = self.upsert(owner, "transition", params, Some(true), &mut created)?;
        let length = transition_length(&self.project, owner);
        let verb = if add { "Added" } else { "Updated" };
        self.say(format_args!("{verb} a {length}-frame transition into {id}"));
        Ok(Outcome {
            summary: Summary::Note,
            primary: Some(Target::Effect(fx)),
            created,
        })
    }

    /// The rules every path that adds or changes a transition goes through (H9, D14): it
    /// belongs to a spine clip right after another, is no longer than either clip, and both
    /// sides have the source handles it consumes (a transition never changes length).
    pub(super) fn check_transition(&self, owner: Owner) -> Result<(), CommandError> {
        let Owner::Clip(slot) = owner else {
            return Err(error(
                "WRONG_KIND",
                &format!(
                    "A transition belongs to the spine clip it leads into, not {}.",
                    self.owner_label(owner)
                ),
                "Use transition.add {item:\"clp_…\", kind, length} on the clip after the cut.",
            ));
        };
        let id = self.project.clips.id(slot).expect("live clip");
        let previous = self.previous_adjacent(slot).ok_or_else(|| {
            error(
                "NO_NEIGHBOR",
                &format!("{id} has no clip right before it."),
                "Add transitions between adjacent spine clips; fade the first clip in with item.set {fadeIn}.",
            )
        })?;
        let length = transition_length(&self.project, owner);
        let ta = self.project.clips.timing(previous).expect("live clip");
        let tb = self.project.clips.timing(slot).expect("live clip");
        let longest = ta.length.min(tb.length);
        if length > longest {
            return Err(CommandError::rule(
                "TOO_LONG",
                "length",
                Some(Json::Signed(i64::from(length))),
                &format!("at most {longest} (the shorter clip it joins)"),
                &format!("Use length ≤ {longest}."),
            ));
        }
        // Handles in timeline frames: unused source before the incoming clip and after the
        // outgoing one. Stills and gaps have unlimited handles.
        let still = |slot: Slot| {
            self.project
                .clips
                .data(slot)
                .and_then(|data| data.media)
                .and_then(|id| media(&self.project, id).ok())
                .is_none_or(|m| m.kind == MediaKind::Image)
        };
        let head = if still(slot) {
            f64::MAX
        } else {
            f64::from(tb.source) / source_rate(&self.project, slot)?
        };
        let tail = match self.media_frames(previous) {
            Some(frames) if !still(previous) => {
                (frames - self.source_end(previous)?) / source_rate(&self.project, previous)?
            }
            _ => f64::MAX,
        };
        let (need_head, need_tail) = (length / 2, length - length / 2);
        let fits =
            |head: f64, tail: f64| head >= f64::from(need_head) && tail >= f64::from(need_tail);
        if !fits(head, tail) {
            let max = (2 * head.min(tail).max(0.0).floor() as Frame).min(longest);
            let previous_id = self.project.clips.id(previous).expect("live clip");
            let fix = if max >= 2 {
                format!(
                    "Use length ≤ {max}, or trim {id}'s head / {previous_id}'s tail to free source \
                     (clip.trim) and retry."
                )
            } else {
                format!(
                    "Trim at least {need_head} frames off {id}'s head and {need_tail} off \
                     {previous_id}'s tail (clip.trim), then retry; or fade with item.set {{fadeIn}}."
                )
            };
            return Err(CommandError::rule(
                "NO_HANDLES",
                "length",
                Some(Json::Signed(i64::from(length))),
                &format!(
                    "handles of {need_head}+{need_tail} frames (have {} before {id}, {} after {previous_id})",
                    if head == f64::MAX {
                        "∞".into()
                    } else {
                        (head.max(0.0).floor() as Frame).to_string()
                    },
                    if tail == f64::MAX {
                        "∞".into()
                    } else {
                        (tail.max(0.0).floor() as Frame).to_string()
                    },
                ),
                &fix,
            ));
        }
        Ok(())
    }

    fn track_mask(
        &mut self,
        mask: &Target,
        keys: &[(Frame, f64, f64)],
    ) -> Result<Outcome, CommandError> {
        let (owner, index) = self.expect_kind(mask, "mask")?;
        let size = |name: &str, default: f64| {
            effects(&self.project, owner)[index]
                .params
                .iter()
                .find(|p| p.name == name)
                .and_then(|p| match &p.value {
                    EffectValue::Number(n) => Some(*n),
                    EffectValue::Keys(keys) => keys.first().map(|k| k.value),
                    _ => None,
                })
                .unwrap_or(default)
        };
        let (w, h) = (size("w", 0.5), size("h", 0.5));
        let track = |center: fn(&(Frame, f64, f64)) -> f64, half: f64| {
            keys.iter()
                .map(|k| Key {
                    at: k.0,
                    value: center(k) - half,
                    ease: Ease::Linear,
                })
                .collect::<Vec<_>>()
        };
        let params = vec![
            keyed("x", track(|k| k.1, w / 2.0)),
            keyed("y", track(|k| k.2, h / 2.0)),
        ];
        check_params("mask", &params)?;
        let effect = &mut effects_vec_mut(&mut self.project, owner)[index];
        for param in params {
            match effect.params.iter_mut().find(|old| old.name == param.name) {
                Some(old) => old.value = param.value,
                None => effect.params.push(param),
            }
        }
        let id = effect.id;
        Ok(Outcome::edit(
            format!("Tracked {id} over {} key(s)", keys.len()),
            Target::Effect(id),
        ))
    }

    fn set_style(&mut self, params: Vec<EffectParam>) -> Result<Outcome, CommandError> {
        let mut created = Vec::new();
        let get = |name: &str| {
            params.iter().find_map(|p| match &p.value {
                EffectValue::Text(text) if p.name == name => Some(text.clone()),
                _ => None,
            })
        };
        let (color, color2, accent, text_color, font) = (
            get("color"),
            get("color2"),
            get("accent"),
            get("textColor"),
            get("font"),
        );
        let id = self.upsert(Owner::Project, "brand", params, Some(true), &mut created)?;
        let mut window = Vec::new();
        if let Some(color) = &color {
            window.push(text_param("color", color));
        }
        if let Some(color2) = &color2 {
            window.push(text_param("color2", color2));
        }
        // Recolor the project's window, or the windows below it when it has none; never create
        // one (M17: a new project window instance turns the frame on for every clip).
        if !window.is_empty() {
            let has = |project: &Project, owner| {
                effects(project, owner).iter().any(|e| e.kind == "window")
            };
            let owners: Vec<Owner> = if has(&self.project, Owner::Project) {
                vec![Owner::Project]
            } else {
                [Owner::Spine]
                    .into_iter()
                    .chain((0..self.project.lanes.len()).map(Owner::Lane))
                    .chain(self.project.clips.order().iter().copied().map(Owner::Clip))
                    .chain(self.project.items.order().iter().copied().map(Owner::Item))
                    .filter(|&owner| has(&self.project, owner))
                    .collect()
            };
            for owner in owners {
                self.upsert(owner, "window", window.clone(), None, &mut created)?;
            }
        }
        if let Some(accent) = &accent {
            self.upsert(
                Owner::Project,
                "captions",
                vec![text_param("active", accent)],
                None,
                &mut created,
            )?;
        }
        let mut text = Vec::new();
        if let Some(font) = &font {
            text.push(text_param("font", font));
        }
        if let Some(color) = &text_color {
            text.push(text_param("color", color));
        }
        if !text.is_empty() {
            self.upsert(Owner::Project, "text", text, None, &mut created)?;
        }
        Ok(Outcome {
            summary: Summary::Text("Applied the brand kit to the project".into()),
            primary: Some(Target::Effect(id)),
            created,
        })
    }

    // ── Transcript-driven cutting ──

    /// Transcript words on the timeline, in time order (empty without a transcript).
    fn spoken(&self) -> Result<Vec<Spoken>, CommandError> {
        let mut words = Vec::new();
        for &slot in self.project.clips.order() {
            splice_model::timing::visible_words(&self.project, slot, &mut |word, span| {
                words.push(Spoken {
                    start: span.start,
                    end: span.end,
                    clip: span.clip,
                    filler: is_filler(&word.text),
                    id: Some(word.id),
                });
            })
            .map_err(time::timing_error)?;
        }
        words.sort_by_key(|word| word.start);
        Ok(words)
    }

    /// What smart cuts measure pauses against (D15): transcript words, or bursts of recorded
    /// activity (`basis:"activity"`, or `auto` on a timeline without words). The flag is true
    /// for activity.
    fn signal(
        &self,
        basis: Basis,
        activity: &[(MediaId, Frame, Frame)],
    ) -> Result<(Vec<Spoken>, bool), CommandError> {
        if basis != Basis::Activity {
            let words = self.spoken()?;
            if !words.is_empty() {
                return Ok((words, false));
            }
            if basis == Basis::Transcript || activity.is_empty() {
                return Err(no_transcript());
            }
        }
        let spans = self.activity_spans(activity)?;
        if spans.is_empty() {
            return Err(error(
                "NO_ACTIVITY",
                "No recorded activity is on the timeline.",
                "Cut takes recorded with input events (record.start events:true), pass activity:[{media, from, to}] busy source frames, or transcribe and use basis:\"transcript\".",
            ));
        }
        Ok((spans, true))
    }

    /// Frames in `seconds` at the project rate.
    pub(super) fn seconds(&self, seconds: f64) -> Frame {
        (seconds * f64::from(self.project.fps)).round() as Frame
    }

    /// Pauses between consecutive spans in one clip, longest first: (from, to) to cut, keeping
    /// `before` frames after the earlier span and `after` frames before the later one.
    fn pauses(
        &self,
        words: &[Spoken],
        min: Frame,
        before: Frame,
        after: Frame,
    ) -> Vec<(Frame, Frame)> {
        let mut cuts = Vec::new();
        for pair in words.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if a.clip != b.clip || b.start - a.end < min {
                continue;
            }
            let (from, to) = (a.end + before, b.start - after);
            if to > from {
                cuts.push((from, to));
            }
        }
        cuts.sort_by_key(|&(from, to)| std::cmp::Reverse(to - from));
        cuts
    }

    /// Ripple-cuts ranges latest first so earlier ranges keep their frames.
    pub(super) fn cut_ranges(
        &mut self,
        mut ranges: Vec<(Frame, Frame)>,
    ) -> Result<Frame, CommandError> {
        ranges.sort_unstable_by_key(|&(from, _)| std::cmp::Reverse(from));
        let mut limit = Frame::MAX;
        let mut total = 0;
        for (from, to) in ranges {
            let to = to.min(limit);
            if to > from {
                self.cut_range(from, to, true, None)?;
                total += to - from;
                limit = from;
            }
        }
        Ok(total)
    }

    fn cut_gaps(
        &mut self,
        words: &[Spoken],
        min: Frame,
        before: Frame,
        after: Frame,
    ) -> Result<(usize, Frame), CommandError> {
        let cuts = self.pauses(words, min, before, after);
        let count = cuts.len();
        let frames = self.cut_ranges(cuts)?;
        Ok((count, frames))
    }

    /// Pauses (or idle stretches), then fillers, then the shortest sentences (or activity
    /// bursts) until the spine fits.
    // ponytail: activity fits cut idle then whole bursts; speed typing up (intent.speedTyping)
    // before cutting bursts once fitDuration needs to keep every action of a silent take.
    fn fit_duration(&mut self, seconds: f64, words: &[Spoken]) -> Result<Outcome, CommandError> {
        let target = (seconds * f64::from(self.project.fps)).round() as Frame;
        let start = self.spine_end()?;
        let mut excess = start - target;
        if excess <= 0 {
            return Ok(done(format!("Already {start} frames, within {target}")));
        }
        // 1. Pauses, longest first, down to 4 frames of air on each side.
        let mut cuts = Vec::new();
        for (from, to) in self.pauses(words, 9, 4, 4) {
            if excess <= 0 {
                break;
            }
            let take = (to - from).min(excess);
            cuts.push((to - take, to));
            excess -= take;
        }
        // 2. Fillers.
        if excess > 0 {
            for word in words.iter().filter(|word| word.filler) {
                if excess <= 0 {
                    break;
                }
                if !cuts.iter().any(|&(f, t)| f < word.end && word.start < t) {
                    cuts.push((word.start, word.end));
                    excess -= word.end - word.start;
                }
            }
        }
        // 3. Whole sentences (runs of words split at pauses over 12 frames), shortest first.
        // ponytail: shortest-first stands in for energy ranking; rank by loudness once
        // media.analyze peaks reach the core.
        if excess > 0 {
            let mut sentences: Vec<(Frame, Frame)> = Vec::new();
            for word in words {
                match sentences.last_mut() {
                    Some(last) if word.start - last.1 <= 12 => last.1 = word.end,
                    _ => sentences.push((word.start, word.end)),
                }
            }
            sentences.sort_by_key(|&(from, to)| to - from);
            for (from, to) in sentences {
                if excess <= 0 {
                    break;
                }
                if !cuts.iter().any(|&(f, t)| f < to && from < t) {
                    cuts.push((from, to));
                    excess -= to - from;
                }
            }
        }
        if excess > 0 {
            return Err(error(
                "FIT_IMPOSSIBLE",
                &format!(
                    "Cutting every pause, filler and sentence still leaves {} frames over {target}.",
                    excess
                ),
                "Choose a longer target or cut whole clips with item.delete.",
            ));
        }
        // Merge overlaps (pauses and words never overlap, sentences may contain cut pauses).
        cuts.sort_unstable();
        let mut merged: Vec<(Frame, Frame)> = Vec::with_capacity(cuts.len());
        for (from, to) in cuts {
            match merged.last_mut() {
                Some(last) if from <= last.1 => last.1 = last.1.max(to),
                _ => merged.push((from, to)),
            }
        }
        let count = merged.len();
        let frames = self.cut_ranges(merged)?;
        Ok(done(format!(
            "Cut {count} range(s), {frames} frames: {start} → {} frames",
            self.spine_end()?
        )))
    }

    fn keep_ranges(&mut self, ranges: &[(Frame, Frame)]) -> Result<Outcome, CommandError> {
        let end = self.spine_end()?;
        let mut keep: Vec<(Frame, Frame)> = ranges.to_vec();
        keep.sort_unstable();
        let mut cuts = Vec::new();
        let mut at = 0;
        for (from, to) in keep {
            if from > at {
                cuts.push((at, from.min(end)));
            }
            at = at.max(to);
        }
        if at < end {
            cuts.push((at, end));
        }
        cuts.retain(|&(from, to)| to > from);
        let frames = self.cut_ranges(cuts)?;
        Ok(done(format!(
            "Kept {} shot(s), cut {frames} frames",
            ranges.len()
        )))
    }

    /// Camera segments from a take's clicks (T38.4, Cap's rules plus Recorder's distance split):
    /// a zoom opens `lead` (0.3 s) before a click and holds `tail` (2.5 s) after the last one;
    /// clicks merge into one zoom while they come within 2.5 s and 0.25 of the screen; typing
    /// extends a zoom and never moves it. Earlier auto zooms on the clip (inside the window,
    /// when one is given) are replaced. Following zooms track the cursor instead of the clicks.
    fn auto_zoom(
        &mut self,
        item: &Target,
        clicks: &[(Frame, f64, f64)],
        typing: &[(Frame, Frame)],
        shape: AutoZoomShape,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let fps = f64::from(self.project.fps);
        let frames = |seconds: f64| (seconds * fps).round() as Frame;
        let (lead, tail, merge, settle, gap, dwell) = (
            shape.lead.unwrap_or(frames(0.3)),
            shape.tail.unwrap_or(frames(2.5)),
            frames(2.5),
            frames(0.5),
            frames(0.6),
            frames(DWELL_SECONDS),
        );
        // The window on the clip's clock (timeline frames minus the clip start).
        let window = [
            shape.window[0].map_or(0, |from| from - timing.start),
            shape.window[1].map_or(timing.length, |to| to - timing.start),
        ];
        if window[0] >= timing.length || window[1] <= 0 {
            return Err(CommandError::rule(
                "OUT_OF_RANGE",
                "from",
                None,
                "a window that overlaps the clip",
                &format!(
                    "{id} plays over frames {}–{}; pass from/to inside that span.",
                    timing.start,
                    timing.start + timing.length
                ),
            ));
        }
        // Where each source frame plays, through the clip's speed effect (as anchors resolve).
        let project = &self.project;
        let local = |source: Frame| -> Result<Frame, CommandError> {
            let at = splice_model::timing::anchor_start(
                project,
                Anchor::ClipSource { clip: id, source },
            )
            .map_err(|_| CommandError::bad("clicks", "source frames inside the take"))?;
            Ok(at - timing.start)
        };
        // Clicks are points of the captured source; zoom focus is a point of the clip's crop at
        // that moment (A5): the animated crop (crop.keys) where there is one, else the static
        // crop. A click outside the crop is not on screen: it moves no zoom.
        let data = self.project.clips.data(slot).expect("live clip");
        let still = data.framing.crop.unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        });
        let keyed = data
            .appearance
            .effects
            .iter()
            .find(|effect| effect.kind == "crop" && effect.enabled != Some(false));
        let crop_at = |at: Frame| -> [f64; 4] {
            let Some(effect) = keyed else {
                return [still.x, still.y, still.w, still.h];
            };
            ["x", "y", "w", "h"].map(|name| {
                let fallback = if matches!(name, "w" | "h") { 1.0 } else { 0.0 };
                match effect.params.iter().find(|param| param.name == name) {
                    Some(EffectParam {
                        value: EffectValue::Number(value),
                        ..
                    }) => *value,
                    Some(EffectParam {
                        value: EffectValue::Keys(keys),
                        ..
                    }) => splice_model::keys::sample(keys, f64::from(at)),
                    _ => fallback,
                }
            })
        };
        let mut outside = 0;
        // Clusters: first and last click frame and the bounds of their points.
        let mut clusters: Vec<(Frame, Frame, [f64; 4])> = Vec::new();
        for &(source, x, y) in clicks {
            let at = local(source)?;
            if at < window[0].max(0) || at >= window[1].min(timing.length) {
                continue;
            }
            let [cx, cy, cw, ch] = crop_at(at);
            let p = [(x - cx) / cw, (y - cy) / ch];
            if !p.iter().all(|v| (0.0..=1.0).contains(v)) {
                outside += 1;
                continue;
            }
            let joins = clusters.last().is_some_and(|&(_, last, b)| {
                let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                at - last <= merge && ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt() < 0.25
            });
            match clusters.last_mut() {
                Some(cluster) if joins => {
                    cluster.1 = at;
                    cluster.2 = grow(cluster.2, p);
                }
                _ => clusters.push((at, at, [p[0], p[1], p[0], p[1]])),
            }
        }
        if clusters.is_empty() {
            return Err(CommandError::bad(
                "clicks",
                if outside > 0 {
                    "clicks inside the clip's crop (every click here is cropped away)"
                } else {
                    "clicks inside the clip (and inside from/to when given)"
                },
            ));
        }
        // The camera dwells on a target: a cluster that starts sooner than `dwell` after the one
        // before joins it, so far-apart clicks in quick succession frame both instead of whipping
        // the camera between them (a sped-up take puts clicks frames apart).
        let mut framed: Vec<(Frame, Frame, [f64; 4])> = Vec::with_capacity(clusters.len());
        for cluster in clusters {
            match framed.last_mut() {
                Some(previous) if cluster.0 - previous.0 < dwell => {
                    previous.1 = cluster.1;
                    previous.2 = grow(
                        grow(previous.2, [cluster.2[0], cluster.2[1]]),
                        [cluster.2[2], cluster.2[3]],
                    );
                }
                _ => framed.push(cluster),
            }
        }
        let typing: Vec<(Frame, Frame)> = typing
            .iter()
            .map(|&(from, to)| Ok((local(from)?, local(to)?)))
            .collect::<Result<_, CommandError>>()?;
        // Segments [start, end) on the clip's clock with their magnification and focus (snapped
        // to edges within 0.25). A segment zooms only as far as keeps all its clicks in view with
        // a margin, and not at all when that is barely a zoom.
        let snap = |v: f64| {
            if v < 0.25 {
                0.0
            } else if v > 0.75 {
                1.0
            } else {
                v
            }
        };
        let mut segments: Vec<(Frame, Frame, Frame, f64, [f64; 2])> = Vec::new();
        for &(first, last, [x0, y0, x1, y1]) in &framed {
            let fits = 1.0 / ((x1 - x0).max(y1 - y0) + FRAMING_MARGIN);
            let amount = shape.amount.min(fits);
            if amount < MIN_AMOUNT {
                continue;
            }
            let mut end = last + tail;
            for &(from, to) in &typing {
                if from < end && to > first - lead {
                    end = end.max(to + settle);
                }
            }
            let start = (first - lead).max(0).max(window[0]);
            let end = end.min(timing.length).min(window[1]);
            if let Some(previous) = segments.last_mut()
                && previous.1 > start
            {
                // Zoom out between far-apart clicks when there is room; otherwise pan.
                previous.1 = (start - gap).max(previous.2 + settle).min(start);
            }
            let focus = [snap((x0 + x1) / 2.0), snap((y0 + y1) / 2.0)];
            segments.push((start, end, last, amount, focus));
        }
        // Replace this clip's earlier auto zooms.
        let clip = id;
        let clip_start = timing.start;
        // A re-run replaces this clip's earlier auto zooms through delete, so the result
        // reports them and anything anchored to them cascades.
        let stale: Vec<Target> = self
            .project
            .items
            .order()
            .iter()
            .copied()
            .filter(|&slot| {
                let data = self.project.items.data(slot).expect("live item");
                let timing = self.project.items.timing(slot).expect("live item");
                matches!(data.kind, ItemKind::Zoom(_))
                    && data.appearance.name.as_deref() == Some(AUTO_ZOOM)
                    && matches!(timing.anchor, Anchor::ClipSource { clip: c, .. } if c == clip)
                    && splice_model::timing::anchor_start(&self.project, timing.anchor)
                        .is_ok_and(|at| (window[0]..window[1]).contains(&(at - clip_start)))
            })
            .filter_map(|slot| self.project.items.id(slot))
            .map(|id| Target::Timeline(TimelineId::Item(id)))
            .collect();
        if !stale.is_empty() {
            for target in &stale {
                if let Target::Timeline(TimelineId::Item(id)) = *target {
                    self.report.side_effects.push(SideEffect::Removed {
                        id,
                        cascade_from: None,
                        cause: Cause::Replaced,
                    });
                }
            }
            self.delete(&stale, false, false, None)?;
        }
        let mut created = Vec::with_capacity(segments.len());
        for &(start, end, _, amount, focus) in &segments {
            if end - start < MIN_LENGTH {
                continue;
            }
            let at = FrameArg::Frame(timing.start + start);
            let outcome = self.add_item(
                OverlayContent::Zoom(Zoom {
                    amount,
                    focus: (!shape.follow).then_some(focus),
                    mode: CameraMode::Auto,
                    spring: Spring::Focused,
                    follow: shape.follow,
                    motion_blur: shape.motion_blur,
                }),
                Some(&at),
                None,
                None,
                Some(end - start),
            )?;
            if let Some(Target::Timeline(TimelineId::Item(zoom))) = outcome.created.first() {
                let slot = item_slot(&self.project, *zoom)?;
                self.project
                    .items
                    .data_mut(slot)
                    .expect("live item")
                    .appearance
                    .name = Some(AUTO_ZOOM.into());
            }
            created.extend(outcome.created);
        }
        let count = created.len();
        Ok(Outcome {
            summary: Summary::Text(if outside > 0 {
                format!(
                    "Added {count} camera zoom(s) on {id} from its clicks; {outside} click(s) outside the crop moved none"
                )
            } else {
                format!("Added {count} camera zoom(s) on {id} from its clicks")
            }),
            primary: created.first().cloned(),
            created,
        })
    }

    // ── Captions ──

    pub(super) fn caption_lane(
        &mut self,
        lane: Option<&Target>,
        name: &str,
    ) -> Result<LaneId, CommandError> {
        match lane {
            Some(target) => {
                let Target::Lane(id) = self.resolve(target)? else {
                    return Err(wrong_kind("Pass a captions lane ID."));
                };
                let lane = self
                    .project
                    .lanes
                    .iter()
                    .find(|lane| lane.id == id)
                    .ok_or_else(|| not_found(&id, "project.info lists lanes."))?;
                if lane.kind != LaneKind::Captions {
                    return Err(wrong_kind("Pass a captions lane ID."));
                }
                Ok(id)
            }
            None => self.new_lane(LaneKind::Captions, Some(name.into())),
        }
    }

    fn translate_captions(
        &mut self,
        lane: &Target,
        language: &str,
        texts: &[String],
    ) -> Result<Outcome, CommandError> {
        let Target::Lane(source) = self.resolve(lane)? else {
            return Err(wrong_kind("Pass a captions lane ID."));
        };
        let mut cues: Vec<(Frame, Slot)> = Vec::new();
        for &slot in self.project.items.order() {
            let data = self.project.items.data(slot).expect("live item");
            if data.lane == source && matches!(data.kind, ItemKind::Caption { .. }) {
                cues.push((time::item_span(&self.project, slot)?.start, slot));
            }
        }
        if cues.is_empty() {
            return Err(error(
                "NO_CAPTIONS",
                &format!("{source} has no caption cues."),
                "Run captions.generate first.",
            ));
        }
        if texts.len() != cues.len() {
            return Err(error(
                if texts.is_empty() {
                    "NEEDS_TEXT"
                } else {
                    "BAD_ARGS"
                },
                &format!(
                    "{source} has {} cues; got {} texts.",
                    cues.len(),
                    texts.len()
                ),
                "Pass texts: one translated string per cue in cue order (transcript.get {captions:\"json\"} lists them).",
            ));
        }
        cues.sort_by_key(|&(start, _)| start);
        let style = self
            .project
            .lanes
            .iter()
            .find(|lane| lane.id == source)
            .map(|lane| lane.effects.clone())
            .unwrap_or_default();
        let lane = self.new_lane(LaneKind::Captions, Some(format!("Captions ({language})")))?;
        if let Some(index) = self.project.lanes.iter().position(|l| l.id == lane) {
            for effect in style {
                let id = EffectId::from_serial(self.mint("fx")?);
                self.project.lanes[index]
                    .effects
                    .push(Effect { id, ..effect });
            }
        }
        let mut created = Vec::new();
        for ((_, slot), text) in cues.iter().zip(texts) {
            let timing = self.project.items.timing(*slot).expect("live item");
            let ItemKind::Caption {
                words, text: cue, ..
            } = &self.project.items.data(*slot).expect("live item").kind
            else {
                continue;
            };
            let kind = if cue.is_some() {
                ItemKind::Caption {
                    words: Vec::new(),
                    edits: Vec::new(),
                    text: Some(text.clone()),
                }
            } else {
                let edits = words
                    .iter()
                    .enumerate()
                    .map(|(n, &word)| CaptionEdit {
                        word,
                        text: Some(if n == 0 { text.clone() } else { String::new() }),
                        emph: false,
                        emoji: None,
                    })
                    .collect();
                ItemKind::Caption {
                    words: words.clone(),
                    edits,
                    text: None,
                }
            };
            let id = ItemId::from_serial(self.mint("ovl")?);
            self.project
                .items
                .insert(Item {
                    id,
                    timing,
                    data: ItemData {
                        lane,
                        kind,
                        appearance: Appearance::default(),
                    },
                })
                .ok_or_else(arena_full)?;
            self.explicit_lanes.push(id);
            created.push(Target::Timeline(TimelineId::Item(id)));
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Translated {} cue(s) into {language} on {lane}",
                created.len()
            )),
            primary: Some(Target::Lane(lane)),
            created,
        })
    }

    fn import_captions(
        &mut self,
        lane: Option<&Target>,
        text: &str,
    ) -> Result<Outcome, CommandError> {
        let cues = parse_cues(text).ok_or_else(|| {
            CommandError::bad("text", "SRT or WebVTT cues (HH:MM:SS,mmm --> HH:MM:SS,mmm)")
        })?;
        let fps = f64::from(self.project.fps);
        let words = self.spoken()?;
        let lane = self.caption_lane(lane, "Captions (imported)")?;
        let mut created = Vec::new();
        let mut free = 0;
        for (from_ms, to_ms, cue) in cues {
            let from = (from_ms as f64 * fps / 1000.0).round() as Frame;
            let to = (to_ms as f64 * fps / 1000.0).round() as Frame;
            let inside: Vec<&Spoken> = words
                .iter()
                .filter(|w| {
                    let middle = (w.start + w.end) / 2;
                    middle >= from && middle < to
                })
                .collect();
            let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
                // No speech under the cue (a silent take): it keeps its own timing (T41.5).
                let end = self.spine_end()?;
                let (from, to) = (from.max(0), to.min(end));
                if to > from {
                    let cue = self.free_cue(lane, cue, from, to)?;
                    if let Target::Timeline(TimelineId::Item(id)) = cue {
                        self.explicit_lanes.push(id);
                    }
                    created.push(cue);
                    free += 1;
                }
                continue;
            };
            let edits = inside
                .iter()
                .enumerate()
                .map(|(n, w)| CaptionEdit {
                    word: w.id.expect("transcript word"),
                    text: Some(if n == 0 { cue.clone() } else { String::new() }),
                    emph: false,
                    emoji: None,
                })
                .collect();
            let anchor = anchor_at(&self.project, first.start)?;
            let id = ItemId::from_serial(self.mint("ovl")?);
            self.project
                .items
                .insert(Item {
                    id,
                    timing: ItemTiming {
                        anchor,
                        length: (last.end - first.start).max(1),
                    },
                    data: ItemData {
                        lane,
                        kind: ItemKind::Caption {
                            words: inside
                                .iter()
                                .map(|w| w.id.expect("transcript word"))
                                .collect(),
                            edits,
                            text: None,
                        },
                        appearance: Appearance::default(),
                    },
                })
                .ok_or_else(arena_full)?;
            self.explicit_lanes.push(id);
            created.push(Target::Timeline(TimelineId::Item(id)));
        }
        if created.is_empty() {
            return Err(error(
                "NO_MATCH",
                "Every cue starts after the end of the timeline.",
                "Check the file's timing against timeline.snapshot.",
            ));
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Imported {} cue(s) onto {lane}{}",
                created.len(),
                if free > 0 {
                    format!(", {free} as free text without speech")
                } else {
                    String::new()
                }
            )),
            primary: Some(Target::Lane(lane)),
            created,
        })
    }

    // ── Audio ──

    fn unlink_audio(
        &mut self,
        item: &Target,
        lane: Option<&Target>,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let media_id = self
            .project
            .clips
            .data(slot)
            .expect("live clip")
            .media
            .ok_or_else(|| wrong_kind("A gap has no audio."))?;
        let descriptor = media(&self.project, media_id)?;
        if descriptor.probe.audio.is_empty() {
            return Err(error(
                "NO_AUDIO",
                &format!("{id} has no audio to unlink."),
                "Unlink a clip whose media has audio tracks.",
            ));
        }
        if timing.speed != 1.0 {
            return Err(error(
                "RETIMED",
                "Retimed clips keep their audio linked.",
                "Set clip.speed to 1 first.",
            ));
        }
        let lane = match lane {
            Some(target) => {
                let Target::Lane(lane) = self.resolve(target)? else {
                    return Err(wrong_kind("Pass an audio lane ID."));
                };
                lane
            }
            None => self.default_lane(LaneKind::Audio)?,
        };
        let sound = ItemId::from_serial(self.mint("ovl")?);
        self.project
            .items
            .insert(Item {
                id: sound,
                timing: ItemTiming {
                    anchor: Anchor::ClipSource {
                        clip: id,
                        source: timing.source,
                    },
                    length: timing.length,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Sound(Sound {
                        media: media_id,
                        source: timing.source,
                        volume: 1.0,
                        duck: None,
                    }),
                    appearance: Appearance {
                        name: Some(format!("Audio of {id}")),
                        ..Appearance::default()
                    },
                },
            })
            .ok_or_else(arena_full)?;
        self.explicit_lanes.push(sound);
        let mut created = vec![Target::Timeline(TimelineId::Item(sound))];
        self.upsert(
            Owner::Clip(slot),
            "volume",
            vec![flag("mute", true)],
            None,
            &mut created,
        )?;
        created.truncate(1);
        Ok(Outcome::create_many(
            format!("Unlinked the audio of {id} into {sound}; trim it for J/L cuts"),
            Target::Timeline(TimelineId::Item(sound)),
            created,
        ))
    }

    fn link_audio(&mut self, item: &Target) -> Result<Outcome, CommandError> {
        let TimelineId::Item(id) = self.timeline(item)? else {
            return Err(wrong_kind("Pass a sound made by audio.unlink."));
        };
        let slot = item_slot(&self.project, id)?;
        let timing = self.project.items.timing(slot).expect("live item");
        let data = self.project.items.data(slot).expect("live item");
        let (Anchor::ClipSource { clip, .. }, ItemKind::Sound(sound)) = (timing.anchor, &data.kind)
        else {
            return Err(wrong_kind("Pass a sound made by audio.unlink."));
        };
        let clip_slot = clip_slot(&self.project, clip)?;
        if self.project.clips.data(clip_slot).expect("live clip").media != Some(sound.media) {
            return Err(wrong_kind(
                "This sound does not come from the clip it is attached to.",
            ));
        }
        self.project.items.remove(slot);
        let list = effects_vec_mut(&mut self.project, Owner::Clip(clip_slot));
        for effect in list.iter_mut().filter(|e| e.kind == "volume") {
            effect.params.retain(|p| p.name != "mute");
        }
        list.retain(|e| !(e.kind == "volume" && e.params.is_empty() && e.enabled.is_none()));
        Ok(Outcome::edit(
            format!("Linked {id} back into {clip}"),
            Target::Timeline(TimelineId::Clip(clip)),
        ))
    }

    fn sync_audio(
        &mut self,
        item: &Target,
        to: &Target,
        offset: Frame,
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Item(id) = self.timeline(item)? else {
            return Err(wrong_kind("Pass a sound or media overlay."));
        };
        let (clip, clip_at) = self.spine_clip(to)?;
        let clip_timing = self.project.clips.timing(clip_at).expect("live clip");
        let slot = item_slot(&self.project, id)?;
        let length = self.project.items.timing(slot).expect("live item").length;
        let source = match &self.project.items.data(slot).expect("live item").kind {
            ItemKind::Sound(sound) => sound.source,
            ItemKind::Media(overlay) => overlay.source,
            _ => return Err(wrong_kind("Pass a sound or media overlay.")),
        };
        // The item's source `offset` plays with the clip's first source frame.
        let mut anchor_source = clip_timing.source + (source - offset);
        let mut new_source = source;
        let mut new_length = length;
        if anchor_source < clip_timing.source {
            let skip = clip_timing.source - anchor_source;
            new_source += skip;
            new_length -= skip;
            anchor_source = clip_timing.source;
        }
        if new_length < 1 {
            return Err(CommandError::bad(
                "offset",
                "an offset that leaves the item on the clip",
            ));
        }
        let data = self.project.items.data_mut(slot).expect("live item");
        match &mut data.kind {
            ItemKind::Sound(sound) => sound.source = new_source,
            ItemKind::Media(overlay) => overlay.source = new_source,
            _ => {}
        }
        self.project
            .items
            .timing_mut(slot)
            .expect("live item")
            .length = new_length;
        self.reanchor(
            slot,
            Anchor::ClipSource {
                clip,
                source: anchor_source,
            },
        );
        Ok(Outcome::edit(
            format!("Synced {id} to {clip} at offset {offset}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    /// Replaces computed ducking with visible `volume.gain` keys on each sound (T19.1): down by
    /// `attack` before each speech span, back up over `release` after it.
    pub(super) fn bake_duck(&mut self, items: &[Target], duck: Duck) -> Result<(), CommandError> {
        let mut speech: Vec<(Frame, Frame)> = Vec::new();
        for word in self.spoken()? {
            match speech.last_mut() {
                Some(last) if word.start - last.1 <= 12 => last.1 = last.1.max(word.end),
                _ => speech.push((word.start, word.end)),
            }
        }
        for target in items {
            let TimelineId::Item(id) = self.timeline(target)? else {
                continue;
            };
            let slot = item_slot(&self.project, id)?;
            let span = time::item_span(&self.project, slot)?;
            let mut keys: Vec<Key> = Vec::new();
            let mut push = |at: Frame, value: f64| {
                let at = (at - span.start).clamp(0, span.end - span.start - 1);
                match keys.last_mut() {
                    Some(last) if last.at >= at => last.value = last.value.min(value),
                    _ => keys.push(Key {
                        at,
                        value,
                        ease: Ease::Linear,
                    }),
                }
            };
            push(span.start, 1.0);
            for &(from, to) in speech.iter().filter(|s| s.1 > span.start && s.0 < span.end) {
                push(from - duck.attack, 1.0);
                push(from, duck.amount);
                push(to, duck.amount);
                push(to + duck.release, 1.0);
            }
            if let ItemKind::Sound(sound) =
                &mut self.project.items.data_mut(slot).expect("live item").kind
            {
                sound.duck = None;
            }
            let mut created = Vec::new();
            self.upsert(
                Owner::Item(slot),
                "volume",
                vec![keyed("gain", keys)],
                None,
                &mut created,
            )?;
        }
        Ok(())
    }

    // ── Templates ──

    fn brand_value(&self, name: &str) -> Option<String> {
        self.project
            .effects
            .iter()
            .find(|e| e.kind == "brand")?
            .params
            .iter()
            .find_map(|p| match &p.value {
                EffectValue::Text(t) if p.name == name => Some(t.clone()),
                _ => None,
            })
    }

    fn add_template(
        &mut self,
        name: &str,
        at: Frame,
        length: Option<Frame>,
        fields: &[(String, String)],
    ) -> Result<Outcome, CommandError> {
        let layout = TEMPLATES
            .iter()
            .find(|template| template.name == name)
            .ok_or_else(|| CommandError::bad("name", "a template from commands.describe"))?;
        let length = length.unwrap_or(4 * self.project.fps as Frame);
        // Brand kit colors (T42.6), falling back to the brand effect's own defaults.
        let brand = |key: &str| {
            self.brand_value(key).unwrap_or_else(|| {
                crate::commands::effect_schemas()
                    .field("brand")
                    .and_then(|schema| schema.field("params"))
                    .and_then(|params| params.field(key))
                    .and_then(|param| param.field("def"))
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_owned()
            })
        };
        let swatch = |role: &str| match role {
            "primary" => brand("color"),
            "accent" => brand("accent"),
            _ => brand("color2"),
        };
        let (fill, fill2) = (swatch(layout.fill[0]), swatch(layout.fill[1]));
        // Text in the brand text color unless it would vanish on the fill: then the brand's
        // dark or light swatch, whichever reads better.
        let text = brand("textColor");
        let ink = if contrast(&text, &fill) >= 3.0 {
            text
        } else {
            let (dark, light) = (brand("color2"), brand("color"));
            if contrast(&dark, &fill) >= contrast(&light, &fill) {
                dark
            } else {
                light
            }
        };
        let font = brand("font");
        // Inside the safe zone (T42.6): panels keep their size and move in; text boxes sit in
        // the panel, or in the safe area on full-canvas cards.
        let [top, bottom, left, right] = safe_zone(&self.project);
        let full = layout.rect.w >= 1.0 && layout.rect.h >= 1.0;
        let rect = if full {
            layout.rect
        } else {
            Rect {
                x: layout
                    .rect
                    .x
                    .clamp(left, (1.0 - right - layout.rect.w).max(left)),
                y: layout
                    .rect
                    .y
                    .clamp(top, (1.0 - bottom - layout.rect.h).max(top)),
                ..layout.rect
            }
        };
        let area = if full {
            Rect {
                x: left,
                y: top,
                w: 1.0 - left - right,
                h: 1.0 - top - bottom,
            }
        } else {
            rect
        };
        let at_arg = FrameArg::Frame(at);
        let lane = self.new_lane(LaneKind::Graphics, Some(format!("Template: {name}")))?;
        let panel = self.add_item(
            OverlayContent::Shape {
                shape: layout.shape,
                rect,
            },
            Some(&at_arg),
            None,
            Some(&Target::Lane(lane)),
            Some(length),
        )?;
        let panel = panel.primary.expect("shape");
        let mut created = vec![panel.clone()];
        let owner = self.owner(&panel)?;
        let mut scratch = Vec::new();
        self.upsert(
            owner,
            "shape",
            vec![
                text_param("fill", &fill),
                text_param("fill2", &fill2),
                number("corner", layout.corner),
            ],
            None,
            &mut scratch,
        )?;
        for line in layout.lines {
            let text = fields
                .iter()
                .find(|(key, _)| key == line.role)
                .map_or(line.default, |(_, value)| value.as_str());
            if text.is_empty() {
                continue;
            }
            let lane = self.new_lane(
                LaneKind::Graphics,
                Some(format!("Template: {name} {}", line.role)),
            )?;
            let title = self
                .add_item(
                    OverlayContent::Title(Some(text.to_owned())),
                    Some(&at_arg),
                    None,
                    Some(&Target::Lane(lane)),
                    Some(length),
                )?
                .primary
                .expect("title");
            let owner = self.owner(&title)?;
            if let Owner::Item(slot) = owner {
                self.project
                    .items
                    .data_mut(slot)
                    .expect("live item")
                    .appearance
                    .name = Some(line.role.into());
            }
            self.upsert(
                owner,
                "text",
                vec![
                    text_param("font", &font),
                    text_param("color", &ink),
                    text_param("align", line.align),
                    number("size", line.size),
                ],
                None,
                &mut scratch,
            )?;
            let [x, y, w, h] = line.text_box;
            self.upsert(
                owner,
                "layout",
                vec![
                    number("x", round4(area.x + x * area.w)),
                    number("y", round4(area.y + y * area.h)),
                    number("w", round4(w * area.w)),
                    number("h", round4(h * area.h)),
                ],
                None,
                &mut scratch,
            )?;
            created.push(title);
        }
        let group = match &panel {
            Target::Timeline(id) => id.to_string(),
            _ => unreachable!("items are timeline objects"),
        };
        for target in &created {
            let owner = self.owner(target)?;
            self.upsert(
                owner,
                "template",
                vec![text_param("name", name), text_param("group", &group)],
                None,
                &mut scratch,
            )?;
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Added a {name} template ({group})")),
            primary: Some(panel),
            created,
        })
    }

    fn set_template(
        &mut self,
        item: &Target,
        fields: &[(String, String)],
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Item(id) = self.timeline(item)? else {
            return Err(wrong_kind("Pass an item made by template.add."));
        };
        let slot = item_slot(&self.project, id)?;
        let tag = |slot: Slot| {
            let data = self.project.items.data(slot)?;
            let effect = data
                .appearance
                .effects
                .iter()
                .find(|e| e.kind == "template")?;
            let get = |name: &str| {
                effect.params.iter().find_map(|p| match &p.value {
                    EffectValue::Text(t) if p.name == name => Some(t.clone()),
                    _ => None,
                })
            };
            Some((get("name")?, get("group")?))
        };
        let (name, group) =
            tag(slot).ok_or_else(|| wrong_kind("Pass an item made by template.add."))?;
        let mut members = Vec::new();
        let mut merged: Vec<(String, String)> = Vec::new();
        let mut start = Frame::MAX;
        let mut length = 0;
        for &slot in self.project.items.order() {
            if tag(slot).is_some_and(|(_, g)| g == group) {
                let span = time::item_span(&self.project, slot)?;
                start = start.min(span.start);
                length = length.max(span.end - span.start);
                let data = self.project.items.data(slot).expect("live item");
                if let (ItemKind::Title { text }, Some(role)) = (&data.kind, &data.appearance.name)
                {
                    merged.push((role.clone(), text.clone()));
                }
                members.push((slot, data.lane));
            }
        }
        for (key, value) in fields {
            match merged.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value.clone(),
                None => merged.push((key.clone(), value.clone())),
            }
        }
        let lanes: Vec<LaneId> = members.iter().map(|&(_, lane)| lane).collect();
        for (slot, _) in members {
            self.project.items.remove(slot);
        }
        self.project.lanes.retain(|lane| {
            !lanes.contains(&lane.id) || !lane.name.as_str().starts_with("Template: ")
        });
        self.add_template(&name, start, Some(length), &merged)
    }

    // ── Compound clips ──

    fn compound(&mut self, items: &[Target]) -> Result<Outcome, CommandError> {
        let mut slots = Vec::with_capacity(items.len());
        for target in items {
            slots.push(self.spine_clip(target)?.1);
        }
        slots.sort_by_key(|&slot| self.project.clips.timing(slot).expect("live clip").start);
        slots.dedup();
        for pair in slots.windows(2) {
            if self.next_adjacent(pair[0]) != Some(pair[1]) {
                return Err(error(
                    "NOT_ADJACENT",
                    "Compound clips need adjacent spine clips.",
                    "Pass a run of neighboring clips from timeline.snapshot.",
                ));
            }
        }
        let first = self.project.clips.timing(slots[0]).expect("live clip");
        let last = self
            .project
            .clips
            .timing(*slots.last().expect("clips"))
            .expect("live clip");
        let (start, end) = (first.start, last.start + last.length);
        let deepest = slots
            .iter()
            .filter_map(|&slot| self.project.clips.data(slot).expect("live clip").media)
            .map(|media| compound_depth(&self.project, media))
            .max()
            .unwrap_or(0);
        if deepest >= MAX_COMPOUND_DEPTH {
            return Err(error(
                "TOO_DEEP",
                &format!("Compound clips nest at most {MAX_COMPOUND_DEPTH} deep."),
                "Uncompound an inner compound clip first with clip.uncompound.",
            ));
        }
        let media_id = MediaId::from_serial(self.mint("med")?);
        let mut clips = Vec::with_capacity(slots.len());
        let mut ids = Vec::with_capacity(slots.len());
        for &slot in &slots {
            let id = self.project.clips.id(slot).expect("live clip");
            let mut timing = self.project.clips.timing(slot).expect("live clip");
            timing.start -= start;
            clips.push(Clip {
                id,
                timing,
                data: self.project.clips.data(slot).expect("live clip").clone(),
            });
            ids.push(id);
        }
        // Items anchored inside move with the clips, pinned to the compound's clock. One that
        // runs past the compound stays outside, anchored to the compound clip, so it still plays
        // whole (the nested timeline would cut it at the compound's end).
        // Any anchor counts (M5): clip-source, a clip's edges, and items anchored to an item
        // that moves inside, found transitively.
        let mut parents: Vec<TimelineId> = ids.iter().map(|&id| TimelineId::Clip(id)).collect();
        let mut inner = Vec::new();
        let mut outside = Vec::new();
        let mut found = true;
        while found {
            found = false;
            for &slot in self.project.items.order() {
                if inner.iter().chain(&outside).any(|&(s, _)| s == slot) {
                    continue;
                }
                let timing = self.project.items.timing(slot).expect("live item");
                if !edits::anchor_parent(timing.anchor).is_some_and(|p| parents.contains(&p)) {
                    continue;
                }
                found = true;
                let span = time::item_span(&self.project, slot)?;
                if span.start < start || span.end > end {
                    outside.push((slot, span.start - start));
                } else {
                    inner.push((slot, span.start - start));
                    parents.push(TimelineId::Item(
                        self.project.items.id(slot).expect("live item"),
                    ));
                }
            }
        }
        let mut inner_items = Vec::with_capacity(inner.len());
        for &(slot, at) in &inner {
            inner_items.push(Item {
                id: self.project.items.id(slot).expect("live item"),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(at),
                    length: self.project.items.timing(slot).expect("live item").length,
                },
                data: self.project.items.data(slot).expect("live item").clone(),
            });
        }
        inner_items.sort_by_key(|item| {
            self.project
                .items
                .order()
                .iter()
                .position(|&s| self.project.items.id(s) == Some(item.id))
        });
        for &(slot, _) in &inner {
            self.project.items.remove(slot);
        }
        let lanes = self.project.lanes.clone();
        for &slot in &slots {
            self.project.clips.remove(slot);
        }
        let fps = self.project.fps;
        self.project.media.push(Media {
            id: media_id,
            kind: MediaKind::Compound,
            // Virtual: the timeline lives in `Project::compounds`.
            path: format!("compound:{media_id}"),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: f64::from(fps),
                frames: end - start,
                width: self.project.size[0],
                height: self.project.size[1],
                audio: vec![AudioTrack {
                    name: "mix".into(),
                    channels: Some(2),
                }],
                codec: None,
                loudness: None,
            },
            take: None,
        });
        self.project.compounds.push(Compound {
            media: media_id,
            clips,
            items: inner_items,
            lanes,
        });
        let clip = ClipId::from_serial(self.mint("clp")?);
        self.project
            .clips
            .insert(Clip {
                id: clip,
                timing: ClipTiming {
                    start,
                    length: end - start,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media_id),
                    appearance: Appearance {
                        name: Some("Compound clip".into()),
                        ..Appearance::default()
                    },
                    ..ClipData::default()
                },
            })
            .ok_or_else(arena_full)?;
        self.project.clips.sort_by_start();
        for (slot, offset) in outside {
            self.reanchor(
                slot,
                Anchor::Item {
                    item: TimelineId::Clip(clip),
                    edge: Edge::Start,
                    offset,
                },
            );
        }
        Ok(Outcome::create(
            format!("Made compound clip {clip} from {} clip(s)", slots.len()),
            Target::Timeline(TimelineId::Clip(clip)),
        ))
    }

    fn uncompound(&mut self, item: &Target) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let media_id = self.project.clips.data(slot).expect("live clip").media;
        let index = self
            .project
            .compounds
            .iter()
            .position(|c| Some(c.media) == media_id)
            .ok_or_else(|| wrong_kind("Pass a compound clip made by clip.compound."))?;
        if timing.speed != 1.0 {
            return Err(error(
                "RETIMED",
                "Retimed compound clips can't break apart.",
                "Set clip.speed to 1 first.",
            ));
        }
        // Outer items on the compound clip land on the clips it held (M5).
        let kids = self.children(&[TimelineId::Clip(id)])?;
        let compound = self.project.compounds.remove(index);
        self.project.media.retain(|m| Some(m.id) != media_id);
        self.project.clips.remove(slot);
        let (window_from, window_to) = (timing.source, timing.source + timing.length);
        let offset = timing.start - timing.source;
        let mut created = Vec::new();
        for mut clip in compound.clips {
            let (from, to) = (clip.timing.start, clip.timing.start + clip.timing.length);
            let (from_kept, to_kept) = (from.max(window_from), to.min(window_to));
            if to_kept <= from_kept {
                continue;
            }
            let rate = clip.timing.speed
                * clip
                    .data
                    .media
                    .and_then(|m| media(&self.project, m).ok())
                    .map_or(1.0, |m| m.probe.fps / f64::from(self.project.fps));
            clip.timing.source += rounded(f64::from(from_kept - from) * rate)?;
            clip.timing.start = from_kept + offset;
            clip.timing.length = to_kept - from_kept;
            clamp_fades(&mut clip.data.appearance, clip.timing.length);
            created.push(Target::Timeline(TimelineId::Clip(clip.id)));
            self.project.clips.insert(clip).ok_or_else(arena_full)?;
        }
        self.project.clips.sort_by_start();
        self.reanchor_at(&kids)?;
        for mut inner in compound.items {
            let Anchor::Pinned(at) = inner.timing.anchor else {
                continue;
            };
            if at + inner.timing.length <= window_from || at >= window_to {
                continue;
            }
            let start = at + offset;
            inner.timing.anchor = anchor_at(&self.project, start.max(0))?;
            if !self
                .project
                .lanes
                .iter()
                .any(|lane| lane.id == inner.data.lane)
            {
                inner.data.lane = self.default_lane(inner.data.kind.lane_kind())?;
            }
            self.project.items.insert(inner).ok_or_else(arena_full)?;
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Broke {id} apart into {} clip(s)", created.len())),
            primary: created.first().cloned(),
            created,
        })
    }
}

impl Outcome {
    fn create_many(summary: String, primary: Target, created: Vec<Target>) -> Self {
        Self {
            summary: Summary::Text(summary),
            primary: Some(primary),
            created,
        }
    }
}

impl Editor {
    /// `media.relink` (M14): the file is probed through the batch's media loader and must match
    /// the media it replaces (length and kind, unless `force`); its probe (fps, size, tracks)
    /// replaces the stale one. Without a prober (Linux, or a journal written before relink
    /// probed) the agent's `frames` is trusted as before.
    pub(super) fn relink(
        &mut self,
        target: &Target,
        path: String,
        frames: Option<Frame>,
        force: bool,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<Outcome, CommandError> {
        let Target::Media(id) = self.resolve(target)? else {
            return Err(wrong_kind("Pass a media ID (med_…)."));
        };
        let index = self
            .project
            .media
            .iter()
            .position(|m| m.id == id)
            .ok_or_else(|| not_found(&id, "project.info lists media."))?;
        let probed = match load(&path) {
            Ok(mut probed) => {
                probed.id = id;
                Some(probed)
            }
            Err(error) if error.message == UNPROBED => None,
            Err(error) => return Err(error),
        };
        let old = &self.project.media[index];
        let measured = probed.as_ref().map(|m| m.probe.frames);
        if let (Some(given), Some(measured)) = (frames, measured)
            && given != measured
        {
            return Err(CommandError::rule(
                "MEDIA_MISMATCH",
                "frames",
                Some(Json::Signed(i64::from(given))),
                &format!("{measured}, the probed length of {path}"),
                "Omit frames: Splice probes the file.",
            ));
        }
        let length = measured.or(frames);
        let kind = probed.as_ref().map_or(old.kind, |m| m.kind);
        if !force && (length.is_some_and(|length| length != old.probe.frames) || kind != old.kind) {
            return Err(CommandError::rule(
                "MEDIA_MISMATCH",
                "path",
                Some(Json::String(path.clone())),
                &format!(
                    "a {:?} file with {} frames like {id}",
                    old.kind, old.probe.frames
                ),
                "Relink the same media (same length and kind), or pass force:true.",
            ));
        }
        let descriptor = &mut self.project.media[index];
        match probed {
            Some(probed) => {
                descriptor.kind = probed.kind;
                descriptor.path.clone_from(&probed.path);
                descriptor.bookmark.clone_from(&probed.bookmark);
                descriptor.size = probed.size;
                descriptor.mtime = probed.mtime;
                descriptor.probe = probed.probe.clone();
                // Replay rebuilds the relink from this descriptor without opening the file.
                self.imports.push(MediaImport {
                    request: path,
                    descriptor: probed,
                });
            }
            None => {
                if let Some(frames) = frames {
                    descriptor.probe.frames = frames;
                }
                descriptor.path = path;
            }
        }
        Ok(Outcome::edit(format!("Relinked {id}"), Target::Media(id)))
    }
}

/// The package-relative path consolidated media lands at (splice mcp copies the file there
/// before the batch applies).
pub fn consolidated_path(id: MediaId, path: &str) -> String {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    format!("media/{id}.{extension}")
}

fn no_handles() -> CommandError {
    error(
        "NO_HANDLES",
        "The source has no media beyond this edit.",
        "Use a smaller offset, or trim the neighbor first.",
    )
}

/// A clip's transition length in frames (the schema default when unset).
fn transition_length(project: &Project, owner: Owner) -> Frame {
    effects(project, owner)
        .iter()
        .find(|effect| effect.kind == "transition")
        .and_then(|effect| effect.params.iter().find(|p| p.name == "length"))
        .and_then(|p| match p.value {
            EffectValue::Number(n) => Some(n),
            _ => None,
        })
        .unwrap_or(15.0)
        .round() as Frame
}

fn no_transcript() -> CommandError {
    error(
        "NO_TRANSCRIPT",
        "No transcript words are on the timeline.",
        "Call transcript.get {transcribe:true} first, or pass basis:\"activity\" to cut a silent take by its recorded input (intent.steps trims a demo to its steps).",
    )
}

fn no_transition() -> CommandError {
    error(
        "NO_TRANSITION",
        "That clip has no transition.",
        "Use transition.add first.",
    )
}

/// SRT or WebVTT cues as (from ms, to ms, text with lines joined by spaces).
pub fn parse_cues(text: &str) -> Option<Vec<(u64, u64, String)>> {
    let time = |s: &str| -> Option<u64> {
        let s = s.trim().replace(',', ".");
        let mut parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            parts.insert(0, "0");
        }
        if parts.len() != 3 {
            return None;
        }
        let (h, m) = (parts[0].parse::<u64>().ok()?, parts[1].parse::<u64>().ok()?);
        let (s, ms) = parts[2].split_once('.').unwrap_or((parts[2], "0"));
        let s = s.parse::<u64>().ok()?;
        let ms = format!("{ms:0<3}");
        if m >= 60 || s >= 60 {
            return None;
        }
        // A huge hour field is a malformed file, not an overflow panic.
        h.checked_mul(3600)?
            .checked_add(m * 60 + s)?
            .checked_mul(1000)?
            .checked_add(ms.get(..3)?.parse::<u64>().ok()?)
    };
    let mut cues = Vec::new();
    let mut lines = text.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        let Some((from, rest)) = line.split_once("-->") else {
            continue;
        };
        let to = rest.split_whitespace().next()?;
        let (from, to) = (time(from)?, time(to)?);
        let mut words = String::new();
        while let Some(&next) = lines.peek() {
            if next.is_empty() {
                break;
            }
            if !words.is_empty() {
                words.push(' ');
            }
            words.push_str(next);
            lines.next();
        }
        if to > from && !words.is_empty() {
            cues.push((from, to, words));
        }
    }
    (!cues.is_empty()).then_some(cues)
}

/// One text line of a template.
struct TemplateLine {
    role: &'static str,
    default: &'static str,
    /// Text box as fractions of the panel (or of the safe area on full-canvas cards).
    text_box: [f64; 4],
    size: f64,
    align: &'static str,
}

/// The safe-zone margins (top, bottom, left, right) templates keep inside: the project's
/// caption safe zone when it sets one, else TikTok's on portrait canvases and 5% elsewhere.
fn safe_zone(project: &Project) -> [f64; 4] {
    let named = project
        .effects
        .iter()
        .filter(|e| e.kind == "captions" && e.enabled != Some(false))
        .flat_map(|e| e.params.iter())
        .find_map(|p| match &p.value {
            EffectValue::Text(name) if p.name == "safe" => Some(name.as_str()),
            _ => None,
        });
    let portrait = project.size[1] > project.size[0];
    match named {
        Some("reels") => [0.14, 0.2, 0.05, 0.12],
        Some("shorts") => [0.12, 0.18, 0.05, 0.14],
        Some("none") => [0.04; 4],
        Some(_) => [0.1, 0.2, 0.05, 0.15],
        None if portrait => [0.1, 0.2, 0.05, 0.15],
        None => [0.05; 4],
    }
}

/// A layout number at the precision a file needs (1/10,000 of the canvas).
fn round4(value: f64) -> f64 {
    (value * 1e4).round() / 1e4
}

/// WCAG contrast ratio of two `#rrggbb` colors (1–21); unreadable input counts as 1.
fn contrast(a: &str, b: &str) -> f64 {
    let luminance = |hex: &str| {
        let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).ok()?;
        let channel = |shift: u32| {
            let c = f64::from((value >> shift) & 0xff) / 255.0;
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        Some(0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0))
    };
    match (luminance(a), luminance(b)) {
        (Some(a), Some(b)) => (a.max(b) + 0.05) / (a.min(b) + 0.05),
        _ => 1.0,
    }
}

/// A template: a backing shape in brand colors plus text lines (T18.5).
struct Template {
    name: &'static str,
    shape: ShapeKind,
    rect: Rect,
    /// Brand roles for the two fills: primary, secondary or accent.
    fill: [&'static str; 2],
    corner: f64,
    lines: &'static [TemplateLine],
}

const fn line(
    role: &'static str,
    default: &'static str,
    text_box: [f64; 4],
    size: f64,
    align: &'static str,
) -> TemplateLine {
    TemplateLine {
        role,
        default,
        text_box,
        size,
        align,
    }
}

const fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "lower-third",
        shape: ShapeKind::Rect,
        rect: rect(0.05, 0.72, 0.6, 0.15),
        fill: ["secondary", "secondary"],
        corner: 0.15,
        lines: &[
            line("title", "Name", [0.05, 0.08, 0.9, 0.5], 0.05, "left"),
            line("subtitle", "Title", [0.05, 0.58, 0.9, 0.34], 0.032, "left"),
        ],
    },
    Template {
        name: "callout",
        shape: ShapeKind::Callout,
        rect: rect(0.3, 0.38, 0.4, 0.16),
        fill: ["primary", "primary"],
        corner: 0.2,
        lines: &[line(
            "title",
            "Look here",
            [0.08, 0.15, 0.84, 0.7],
            0.045,
            "center",
        )],
    },
    Template {
        name: "cta",
        shape: ShapeKind::Rect,
        rect: rect(0.2, 0.8, 0.6, 0.1),
        fill: ["accent", "accent"],
        corner: 0.5,
        lines: &[line(
            "title",
            "Follow for more",
            [0.05, 0.1, 0.9, 0.8],
            0.045,
            "center",
        )],
    },
    Template {
        name: "title-card",
        shape: ShapeKind::Solid,
        rect: rect(0.0, 0.0, 1.0, 1.0),
        fill: ["secondary", "secondary"],
        corner: 0.0,
        lines: &[
            line("title", "Title", [0.0, 0.3, 1.0, 0.22], 0.08, "center"),
            line("subtitle", "", [0.0, 0.54, 1.0, 0.12], 0.04, "center"),
        ],
    },
    Template {
        name: "chapter-card",
        shape: ShapeKind::Gradient,
        rect: rect(0.0, 0.0, 1.0, 1.0),
        fill: ["primary", "secondary"],
        corner: 0.0,
        lines: &[line(
            "title",
            "Chapter",
            [0.0, 0.38, 1.0, 0.24],
            0.07,
            "center",
        )],
    },
];

/// Shortest time the auto-zoom camera holds one target before moving to the next.
const DWELL_SECONDS: f64 = 1.2;
/// Room around a segment's clicks inside the viewport, as a fraction of the content.
const FRAMING_MARGIN: f64 = 0.2;
/// A zoom below this magnification reads as a wobble, not a zoom: the segment is dropped.
const MIN_AMOUNT: f64 = 1.15;

/// `bounds` grown to include point `p`.
fn grow([x0, y0, x1, y1]: [f64; 4], [x, y]: [f64; 2]) -> [f64; 4] {
    [x0.min(x), y0.min(y), x1.max(x), y1.max(y)]
}

/// How `intent.autoZoom` shapes its zooms (defaults applied in `auto_zoom`).
#[derive(Clone, Copy, Debug)]
struct AutoZoomShape {
    amount: f64,
    lead: Option<Frame>,
    tail: Option<Frame>,
    window: [Option<Frame>; 2],
    follow: bool,
    motion_blur: f64,
}

/// Name that marks camera segments `intent.autoZoom` made (and replaces on a re-run).
const AUTO_ZOOM: &str = "Auto zoom";

/// The deepest nesting `clip.compound` makes; the evaluator flattens this many levels.
const MAX_COMPOUND_DEPTH: usize = 4;

/// How many compound levels a media holds: 0 for ordinary media.
fn compound_depth(project: &Project, media: MediaId) -> usize {
    project
        .compounds
        .iter()
        .find(|compound| compound.media == media)
        .map_or(0, |compound| {
            1 + compound
                .clips
                .iter()
                .filter_map(|clip| clip.data.media)
                .map(|inner| compound_depth(project, inner))
                .max()
                .unwrap_or(0)
        })
}

#[cfg(test)]
mod retake_tests {
    use super::*;

    #[test]
    fn restarted_phrases_are_retakes_and_fillers_are_found() {
        let words = [
            ("So", 0, 5),
            ("today", 6, 12),
            ("we", 13, 15),
            // pause, then a restart
            ("So", 40, 45),
            ("today", 46, 52),
            ("we", 53, 55),
            ("build", 56, 60),
            ("um", 80, 84),
            ("Next", 100, 106),
        ];
        assert_eq!(retakes(&words, 12), [(0, 3)]);
        assert!(is_filler("Um,") && is_filler("uh") && !is_filler("umbrella"));
    }
}
