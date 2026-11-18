use super::*;
use std::collections::BTreeSet;

impl Editor {
    pub(super) fn settle(&mut self, base: &Project) -> Result<(), CommandError> {
        self.project.clips.sort_by_start();
        self.sync_captions()?;
        self.fix_source_anchors()?;
        self.settle_lanes(base)?;
        self.rekey_effects()?;
        Ok(())
    }

    pub(super) fn default_lane(&mut self, kind: LaneKind) -> Result<LaneId, CommandError> {
        if let Some(lane) = self
            .project
            .lanes
            .iter()
            .rev()
            .find(|lane| lane.kind == kind)
        {
            return Ok(lane.id);
        }
        self.new_lane(kind, None)
    }

    pub(super) fn new_lane(
        &mut self,
        kind: LaneKind,
        name: Option<String>,
    ) -> Result<LaneId, CommandError> {
        let id = LaneId::from_serial(self.mint("lan")?);
        let lanes = &mut self.project.lanes;
        let count = lanes.iter().filter(|lane| lane.kind == kind).count();
        let name = name.filter(|name| !name.is_empty()).unwrap_or_else(|| {
            let base = match kind {
                LaneKind::Captions => "Captions",
                LaneKind::Graphics => "Graphics",
                LaneKind::Overlay => "Overlay",
                LaneKind::Motion => "Motion",
            };
            if count > 0 || kind == LaneKind::Overlay {
                format!("{base} {}", count + 1)
            } else {
                base.into()
            }
        });
        let index = lanes
            .iter()
            .position(|lane| lane.kind == kind)
            .unwrap_or_else(|| {
                lanes
                    .iter()
                    .take_while(|lane| rank(lane.kind) < rank(kind))
                    .count()
            });
        lanes.insert(
            index,
            Lane {
                id,
                name,
                kind,
                effects: Vec::new(),
            },
        );
        Ok(id)
    }

    fn sync_captions(&mut self) -> Result<(), CommandError> {
        for slot in self.project.items.order().to_vec() {
            let Some(data) = self.project.items.data(slot) else {
                continue;
            };
            let ItemKind::Caption { words, .. } = &data.kind else {
                continue;
            };
            let timing = self.project.items.timing(slot).expect("live caption");
            let hash = caption_hash(&self.project, timing.anchor);
            let mut first: Option<time::WordSpan> = None;
            let mut end = 0;
            for &word in words {
                if let Some(span) = time::word_span(&self.project, word, hash)? {
                    end = end.max(span.end);
                    if first.as_ref().is_none_or(|first| span.start < first.start) {
                        first = Some(span);
                    }
                }
            }
            if let Some(first) = first {
                let length = (end - first.start).max(3);
                *self.project.items.timing_mut(slot).expect("live caption") = ItemTiming {
                    anchor: Anchor::ClipSource {
                        clip: first.clip,
                        source: first.source,
                    },
                    length,
                };
                edits::clamp_fades(
                    &mut self
                        .project
                        .items
                        .data_mut(slot)
                        .expect("live caption")
                        .appearance,
                    length,
                );
            } else {
                let id = self.project.items.id(slot).expect("live caption");
                self.project.items.remove(slot);
                self.report.side_effects.push(SideEffect::Removed {
                    id,
                    cascade_from: None,
                });
                self.cascade(TimelineId::Item(id));
            }
        }
        Ok(())
    }

    fn fix_source_anchors(&mut self) -> Result<(), CommandError> {
        for index in 0..self.project.items.order().len() {
            let slot = self.project.items.order()[index];
            let anchor = self.project.items.timing(slot).expect("live item").anchor;
            let Anchor::ClipSource { clip, source } = anchor else {
                continue;
            };
            let clip_slot = time::clip_slot(&self.project, clip)?;
            let lo = self
                .project
                .clips
                .timing(clip_slot)
                .expect("live clip")
                .source;
            let hi = time::last_source(&self.project, clip_slot)?;
            if hi < lo {
                return Err(frame_range());
            }
            self.reanchor(
                slot,
                Anchor::ClipSource {
                    clip,
                    source: source.clamp(lo, hi),
                },
            );
        }
        Ok(())
    }

    fn settle_lanes(&mut self, base: &Project) -> Result<(), CommandError> {
        let mut order = Vec::with_capacity(self.project.items.order().len());
        for &slot in self.project.items.order() {
            let id = self.project.items.id(slot).expect("live item");
            let span = time::span(&self.project, TimelineId::Item(id))?;
            if span.start < 0 || span.start >= span.end {
                return Err(error(
                    "OUT_OF_RANGE",
                    "An item has a negative start or an empty range.",
                    "Move it onto the timeline and give it a positive length.",
                ));
            }
            let touched = time::span(base, TimelineId::Item(id)).ok() != Some(span);
            order.push((touched, span.start, id.to_string(), slot, span));
        }
        order.sort_unstable_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
        let mut placed: Vec<(LaneId, Span)> = Vec::with_capacity(order.len());
        for (_, _, _, slot, span) in order {
            let id = self.project.items.id(slot).expect("live item");
            let data = self.project.items.data(slot).expect("live item");
            let kind = data.kind.lane_kind();
            let mut lane = data.lane;
            if !self
                .project
                .lanes
                .iter()
                .any(|candidate| candidate.id == lane && candidate.kind == kind)
            {
                lane = self.default_lane(kind)?;
                self.project.items.data_mut(slot).expect("live item").lane = lane;
            }
            let occupied = |lane| {
                placed.iter().any(|&(other, range)| {
                    other == lane && range.start < span.end && range.end > span.start
                })
            };
            if occupied(lane) {
                if self.explicit_lanes.contains(&id) {
                    return Err(error(
                        "LANE_CONFLICT",
                        &format!("{lane} is already occupied at frame {}.", span.start),
                        "Omit lane to choose a free lane or select another lane.",
                    ));
                }
                let index = self
                    .project
                    .lanes
                    .iter()
                    .position(|candidate| candidate.id == lane)
                    .expect("current lane");
                let available = self.project.lanes[..index]
                    .iter()
                    .rev()
                    .find(|candidate| candidate.kind == kind && !occupied(candidate.id))
                    .map(|candidate| candidate.id);
                let to = match available {
                    Some(lane) => lane,
                    None => self.new_lane(kind, None)?,
                };
                self.project.items.data_mut(slot).expect("live item").lane = to;
                self.report
                    .side_effects
                    .push(SideEffect::Bumped { id, from: lane, to });
                lane = to;
            }
            placed.push((lane, span));
        }
        Ok(())
    }

    fn rekey_effects(&mut self) -> Result<(), CommandError> {
        let mut owners = vec![Owner::Project, Owner::Spine];
        owners.extend((0..self.project.lanes.len()).map(Owner::Lane));
        owners.extend(self.project.clips.order().iter().copied().map(Owner::Clip));
        owners.extend(self.project.items.order().iter().copied().map(Owner::Item));
        let mut seen = BTreeSet::new();
        for owner in owners {
            for index in 0..effects(&self.project, owner).len() {
                if !seen.insert(effects(&self.project, owner)[index].id.serial()) {
                    let id = EffectId::from_serial(self.mint("fx")?);
                    effects_mut(&mut self.project, owner)[index].id = id;
                    seen.insert(id.serial());
                }
            }
        }
        Ok(())
    }
}

fn rank(kind: LaneKind) -> u8 {
    match kind {
        LaneKind::Captions => 0,
        LaneKind::Graphics => 1,
        LaneKind::Overlay => 2,
        LaneKind::Motion => 3,
    }
}

fn caption_hash(project: &Project, anchor: Anchor) -> Option<u64> {
    match anchor {
        Anchor::ClipSource { clip, .. } => {
            let media = project.clips.data(project.clips.find(clip)?)?.media?;
            project
                .transcripts
                .iter()
                .find(|transcript| transcript.media == media)
                .map(|t| t.hash)
        }
        Anchor::Word {
            transcript_hash, ..
        } => Some(transcript_hash),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Owner {
    Project,
    Spine,
    Lane(usize),
    Clip(Slot),
    Item(Slot),
}

fn effects(project: &Project, owner: Owner) -> &[Effect] {
    match owner {
        Owner::Project => &project.effects,
        Owner::Spine => &project.spine.effects,
        Owner::Lane(index) => &project.lanes[index].effects,
        Owner::Clip(slot) => {
            &project
                .clips
                .data(slot)
                .expect("live clip")
                .appearance
                .effects
        }
        Owner::Item(slot) => {
            &project
                .items
                .data(slot)
                .expect("live item")
                .appearance
                .effects
        }
    }
}

fn effects_mut(project: &mut Project, owner: Owner) -> &mut [Effect] {
    match owner {
        Owner::Project => &mut project.effects,
        Owner::Spine => &mut project.spine.effects,
        Owner::Lane(index) => &mut project.lanes[index].effects,
        Owner::Clip(slot) => {
            &mut project
                .clips
                .data_mut(slot)
                .expect("live clip")
                .appearance
                .effects
        }
        Owner::Item(slot) => {
            &mut project
                .items
                .data_mut(slot)
                .expect("live item")
                .appearance
                .effects
        }
    }
}
