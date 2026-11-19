//! Editing takes without speech (D15, PLAN.md T41): recorded activity as the signal, steps,
//! typing speed-ups and free-text caption cues. The core never opens media: activity and typing
//! ranges arrive in the op (splice mcp fills them from the take's events), so a batch replays
//! without the take.
use super::edits::{arena_full, clamp_fades};
use super::tier_c::{Spoken, done};
use super::time::{anchor_at, source_rate};
use super::*;

/// A step of `intent.steps`: where it starts, its name, and the timeline frames it keeps.
struct Step {
    at: Frame,
    name: String,
    keep: Option<(Frame, Frame)>,
}

impl Editor {
    /// Busy source ranges of takes as timeline spans on every spine clip that shows them,
    /// merged per clip and in time order.
    pub(super) fn activity_spans(
        &self,
        activity: &[(MediaId, Frame, Frame)],
    ) -> Result<Vec<Spoken>, CommandError> {
        let mut spans = Vec::new();
        for &slot in self.project.clips.order() {
            let Some(media) = self.project.clips.data(slot).expect("live clip").media else {
                continue;
            };
            let id = self.project.clips.id(slot).expect("live clip");
            let ranges: Vec<(Frame, Frame)> = activity
                .iter()
                .filter(|range| range.0 == media)
                .map(|&(_, from, to)| (from, to))
                .collect();
            for (start, end) in self.on_clip(slot, &ranges)? {
                spans.push(Spoken {
                    start,
                    end,
                    clip: id,
                    filler: false,
                    id: None,
                });
            }
        }
        spans.sort_by_key(|span| span.start);
        Ok(spans)
    }

    /// Source frame ranges as timeline ranges inside the clip at `slot`, sorted and merged.
    fn on_clip(
        &self,
        slot: Slot,
        ranges: &[(Frame, Frame)],
    ) -> Result<Vec<(Frame, Frame)>, CommandError> {
        let timing = self.project.clips.timing(slot).expect("live clip");
        let rate = source_rate(&self.project, slot)?;
        let at = |source: Frame| {
            let offset = ((f64::from(source) - f64::from(timing.source)) / rate).round();
            timing.start + offset.clamp(0.0, f64::from(timing.length)) as Frame
        };
        let mut out: Vec<(Frame, Frame)> = ranges
            .iter()
            .map(|&(from, to)| (at(from), at(to)))
            .filter(|&(from, to)| to > from)
            .collect();
        out.sort_unstable();
        Ok(merge(out, 0))
    }

    /// `intent.steps` (T41.2): a screen demo trimmed to its steps. Steps start at the clip's
    /// markers, or at bursts of activity when it has none. Each keeps `lead` frames before it and
    /// `settle` after its last activity; quiet stretches longer than `max_gap` inside a step are
    /// cut. A marker named with the retake prefix drops the attempt since the step before it.
    pub(super) fn steps(
        &mut self,
        item: &Target,
        [lead, settle, max_gap]: [Option<Frame>; 3],
        prefix: &str,
        activity: &[(Frame, Frame)],
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let (start, end) = (timing.start, timing.start + timing.length);
        let lead = lead.unwrap_or(self.seconds(0.4));
        let settle = settle.unwrap_or(self.seconds(1.0));
        let max_gap = max_gap.unwrap_or(self.seconds(1.5));
        let busy = self.on_clip(slot, activity)?;
        if busy.is_empty() {
            return Err(error(
                "NO_ACTIVITY",
                &format!("{id} has no recorded activity inside it."),
                "Pass activity:[{from, to}] busy source frames, or omit it so splice mcp reads the take's events.",
            ));
        }
        let mut markers: Vec<(Frame, &str)> = self
            .project
            .markers
            .iter()
            .filter(|marker| (start..end).contains(&marker.at))
            .map(|marker| (marker.at, marker.name.as_str()))
            .collect();
        markers.sort_by_key(|marker| marker.0);
        let mut steps: Vec<Step> = Vec::new();
        let mut dropped: Vec<(Frame, Frame)> = Vec::new();
        for (at, name) in markers {
            let name = match name.strip_prefix(prefix) {
                Some(rest) if !prefix.is_empty() => {
                    // The attempt since the previous step failed; the step restarts here.
                    let from = steps.pop().map_or(start, |step| step.at);
                    dropped.push((from, at));
                    rest.trim()
                }
                _ => name,
            };
            steps.push(Step {
                at,
                name: name.into(),
                keep: None,
            });
        }
        if steps.is_empty() {
            // No markers: every burst after a quiet stretch over max_gap starts a step.
            let mut last: Option<Frame> = None;
            for &(from, to) in &busy {
                if last.is_none_or(|last| from - last > max_gap) {
                    steps.push(Step {
                        at: from,
                        name: "activity".into(),
                        keep: None,
                    });
                }
                last = Some(last.map_or(to, |last| last.max(to)));
            }
        }
        let mut keep: Vec<(Frame, Frame)> = Vec::new();
        for index in 0..steps.len() {
            let at = steps[index].at;
            let next = steps.get(index + 1).map_or(end, |step| step.at);
            let first = keep.len();
            let mut run: Option<(Frame, Frame)> = None;
            for &(from, to) in busy.iter().filter(|&&(from, to)| to > at && from < next) {
                let (from, to) = (from.max(at), to.min(next));
                run = match run {
                    Some((lo, hi)) if from - hi > max_gap => {
                        keep.push((lo, hi + settle));
                        Some((from - lead, to))
                    }
                    Some((lo, hi)) => Some((lo, hi.max(to))),
                    None => Some((at - lead, to)),
                };
            }
            let (lo, hi) = run.unwrap_or((at - lead, at));
            keep.push((lo, hi + settle));
            let span = keep[first..]
                .iter()
                .fold((Frame::MAX, Frame::MIN), |(lo, hi), &(a, b)| {
                    (lo.min(a.max(start)), hi.max(b.min(end)))
                });
            steps[index].keep = Some(span);
        }
        // Clip, merge, and take the dropped attempts out of what is kept.
        let mut kept: Vec<(Frame, Frame)> = keep
            .into_iter()
            .map(|(from, to)| (from.max(start), to.min(end)))
            .filter(|&(from, to)| to > from)
            .collect();
        kept.sort_unstable();
        let mut kept = merge(kept, 0);
        for &(from, to) in &dropped {
            kept = kept
                .into_iter()
                .flat_map(|(a, b)| [(a, b.min(from)), (a.max(to), b)])
                .filter(|&(a, b)| b > a)
                .collect();
        }
        let mut cuts = Vec::new();
        let mut at = start;
        for &(from, to) in &kept {
            if from > at {
                cuts.push((at, from));
            }
            at = at.max(to);
        }
        if at < end {
            cuts.push((at, end));
        }
        let frames = self.cut_ranges(cuts)?;
        let mut summary = format!("Kept {} step(s) of {id}, cut {frames} frames", steps.len());
        if !dropped.is_empty() {
            let _ = write!(summary, ", dropped {} retake attempt(s)", dropped.len());
        }
        summary.push(':');
        for step in &steps {
            let (mut from, to) = step.keep.unwrap_or((step.at, step.at));
            // A lead into a dropped attempt is not kept.
            for &(lo, hi) in &dropped {
                if (lo..hi).contains(&from) {
                    from = hi;
                }
            }
            let _ = write!(
                summary,
                " {:?} at {} keeps {from}–{to};",
                step.name, step.at
            );
        }
        summary.pop();
        Ok(done(summary))
    }

    /// `intent.speedTyping` (T41.4): typing runs of at least `min_run` frames play at `factor`
    /// through eased `speed.rate` keys; the clip shortens so it shows the same source, never
    /// more (the media-length invariant).
    // ponytail: items anchored inside a ramped clip resolve at its constant rate (as with any
    // speed ramp); map anchors through the rate curve once ramps carry captions or overlays.
    pub(super) fn speed_typing(
        &mut self,
        item: &Target,
        factor: f64,
        min_run: Option<Frame>,
        typing: &[(Frame, Frame)],
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.spine_clip(item)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let ramped = self
            .project
            .clips
            .data(slot)
            .expect("live clip")
            .appearance
            .effects
            .iter()
            .any(|effect| effect.kind == "speed" && effect.enabled != Some(false));
        if ramped {
            return Err(error(
                "RETIMED",
                &format!("{id} already has a speed ramp."),
                "Remove its speed effect with effect.remove first, then speed typing up.",
            ));
        }
        let min_run = min_run.unwrap_or(self.seconds(1.0));
        let ramp = self.seconds(0.2).max(1);
        // Runs on the clip's own clock at rate 1 (frames from its start).
        let runs: Vec<(Frame, Frame)> = self
            .on_clip(slot, typing)?
            .into_iter()
            .map(|(from, to)| (from - timing.start, to - timing.start))
            .filter(|&(from, to)| to - from >= min_run)
            .collect();
        let total = f64::from(timing.length);
        let mut keys: Vec<Key> = Vec::new();
        // `t` is the new clock (integer frames), `u` the content shown so far at rate 1.
        let (mut t, mut u) = (0, 0.0);
        let mut sped = 0;
        for (from, to) in runs {
            let lead_in = f64::from(from) - u;
            if lead_in < 0.0 {
                continue;
            }
            let span = f64::from(to - from);
            // Ramps ease 1 → factor → 1; each covers ramp × (1 + factor) / 2 of content.
            let ramp = ramp.min((span / (1.0 + factor)).floor() as Frame);
            let middle = ((span - f64::from(ramp) * (1.0 + factor)) / factor).round() as Frame;
            if ramp < 1 || middle < 1 {
                continue;
            }
            let k0 = t + lead_in.round() as Frame;
            let u0 = u + f64::from(k0 - t);
            match keys.last_mut() {
                Some(last) if last.at == k0 => last.ease = Ease::InOut,
                _ => keys.push(key(k0, 1.0, Ease::InOut)),
            }
            keys.push(key(k0 + ramp, factor, Ease::Linear));
            keys.push(key(k0 + ramp + middle, factor, Ease::InOut));
            t = k0 + 2 * ramp + middle;
            keys.push(key(t, 1.0, Ease::Linear));
            u = u0 + f64::from(ramp) * (1.0 + factor) + f64::from(middle) * factor;
            sped += 1;
        }
        if sped == 0 {
            return Err(error(
                "NO_TYPING",
                &format!("{id} has no typing run of at least {min_run} frames."),
                "Lower minRun, or check takes.events for key presses inside the clip.",
            ));
        }
        // The longest length whose integral stays within the content the clip showed.
        let mut length = (f64::from(t) + (total - u)).floor() as Frame;
        while length > 3 && keys::integrate(&keys, f64::from(length)) > total + 1e-9 {
            length -= 1;
        }
        while keys::integrate(&keys, f64::from(length + 1)) <= total + 1e-9 {
            length += 1;
        }
        let owner = self.spine_owner(item)?;
        let mut created = Vec::new();
        self.upsert(owner, "speed", vec![keyed_rate(keys)], None, &mut created)?;
        let old_end = timing.start + timing.length;
        self.shift_after(old_end, length - timing.length, Some(slot))?;
        self.project
            .clips
            .timing_mut(slot)
            .expect("live clip")
            .length = length;
        clamp_fades(
            &mut self
                .project
                .clips
                .data_mut(slot)
                .expect("live clip")
                .appearance,
            length,
        );
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Sped up {sped} typing run(s) on {id} to {factor}×: {} → {length} frames",
                timing.length
            )),
            primary: Some(Target::Timeline(TimelineId::Clip(id))),
            created,
        })
    }

    /// `caption.add` (T41.5): a free-text cue on the timeline, anchored to the spine clip under
    /// its start so it moves with that clip.
    pub(super) fn add_caption(
        &mut self,
        text: String,
        from: Frame,
        to: Frame,
        lane: Option<&Target>,
    ) -> Result<Outcome, CommandError> {
        let end = self.spine_end()?;
        if to > end {
            return Err(CommandError::rule(
                "OUT_OF_RANGE",
                "to",
                Some(Json::Signed(i64::from(to))),
                &format!("a frame ≤ {end}, the end of the timeline"),
                "Keep the cue inside the timeline (timeline.snapshot gives its length).",
            ));
        }
        // Without a lane the cue may settle onto a free captions lane.
        let explicit = lane.is_some();
        let lane = match lane {
            Some(_) => self.caption_lane(lane, "Captions")?,
            None => match self
                .project
                .lanes
                .iter()
                .find(|lane| lane.kind == LaneKind::Captions)
            {
                Some(lane) => lane.id,
                None => self.caption_lane(None, "Captions")?,
            },
        };
        let created = self.free_cue(lane, text, from, to)?;
        if explicit && let Target::Timeline(TimelineId::Item(id)) = created {
            self.explicit_lanes.push(id);
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Added a caption on {lane} at {from}–{to}")),
            primary: Some(created.clone()),
            created: vec![created],
        })
    }

    /// A free-text cue item on `lane` over timeline frames `[from, to)`; the caller decides
    /// whether its lane is explicit.
    pub(super) fn free_cue(
        &mut self,
        lane: LaneId,
        text: String,
        from: Frame,
        to: Frame,
    ) -> Result<Target, CommandError> {
        let anchor = anchor_at(&self.project, from)?;
        let id = ItemId::from_serial(self.mint("ovl")?);
        self.project
            .items
            .insert(Item {
                id,
                timing: ItemTiming {
                    anchor,
                    length: to - from,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Caption {
                        words: Vec::new(),
                        edits: Vec::new(),
                        text: Some(text),
                    },
                    appearance: Appearance::default(),
                },
            })
            .ok_or_else(arena_full)?;
        Ok(Target::Timeline(TimelineId::Item(id)))
    }
}

fn key(at: Frame, value: f64, ease: Ease) -> Key {
    Key { at, value, ease }
}

fn keyed_rate(keys: Vec<Key>) -> EffectParam {
    EffectParam {
        name: "rate".into(),
        value: EffectValue::Keys(keys),
    }
}

/// Sorted ranges with those closer than `gap` joined.
fn merge(ranges: Vec<(Frame, Frame)>, gap: Frame) -> Vec<(Frame, Frame)> {
    let mut out: Vec<(Frame, Frame)> = Vec::with_capacity(ranges.len());
    for (from, to) in ranges {
        match out.last_mut() {
            Some(last) if from - last.1 <= gap => last.1 = last.1.max(to),
            _ => out.push((from, to)),
        }
    }
    out
}
