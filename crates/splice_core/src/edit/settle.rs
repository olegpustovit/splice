use super::*;

#[derive(Default)]
pub(super) struct Scratch {
    order: Vec<OrderedItem>,
    lanes: Vec<LaneIntervals>,
    owners: Vec<(bool, usize, Owner)>,
    seen: Vec<u64>,
    captions: Vec<Slot>,
}

struct OrderedItem {
    touched: bool,
    slot: Slot,
    id: ItemId,
    span: Span,
}

struct LaneIntervals {
    id: LaneId,
    spans: Vec<Span>,
}

impl LaneIntervals {
    fn insertion(&self, span: Span) -> Option<usize> {
        if self
            .spans
            .last()
            .is_none_or(|prior| prior.end <= span.start)
        {
            return Some(self.spans.len());
        }
        let index = self.spans.partition_point(|prior| prior.end <= span.start);
        if self
            .spans
            .get(index)
            .is_none_or(|prior| prior.start >= span.end)
        {
            Some(index)
        } else {
            None
        }
    }
}

impl Editor {
    pub(super) fn settle(&mut self, base: &Project) -> Result<(), CommandError> {
        self.project.clips.sort_by_start();
        self.sync_captions()?;
        self.fix_source_anchors()?;
        self.settle_lanes(base)?;
        self.rekey_effects(base)?;
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
        let mut captions = std::mem::take(&mut self.settle_scratch.captions);
        captions.clear();
        captions.extend(self.project.items.order().iter().copied().filter(|&slot| {
            matches!(
                self.project.items.data(slot).expect("live item").kind,
                ItemKind::Caption { .. }
            )
        }));
        let result = self.settle_captions(&captions);
        self.settle_scratch.captions = captions;
        result
    }

    fn settle_captions(&mut self, captions: &[Slot]) -> Result<(), CommandError> {
        for &slot in captions {
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
        let mut order = std::mem::take(&mut self.settle_scratch.order);
        let mut lanes = std::mem::take(&mut self.settle_scratch.lanes);
        let result = self.place_items(base, &mut order, &mut lanes);
        self.settle_scratch.order = order;
        self.settle_scratch.lanes = lanes;
        result
    }

    fn place_items(
        &mut self,
        base: &Project,
        order: &mut Vec<OrderedItem>,
        lanes: &mut Vec<LaneIntervals>,
    ) -> Result<(), CommandError> {
        order.clear();
        lanes.truncate(self.project.lanes.len());
        for (index, lane) in self.project.lanes.iter().enumerate() {
            if let Some(intervals) = lanes.get_mut(index) {
                intervals.id = lane.id;
                intervals.spans.clear();
            } else {
                lanes.push(LaneIntervals {
                    id: lane.id,
                    spans: Vec::new(),
                });
            }
        }
        for &slot in self.project.items.order() {
            let id = self.project.items.id(slot).expect("live item");
            let span = time::item_span(&self.project, slot)?;
            if span.start < 0 || span.start >= span.end {
                return Err(error(
                    "OUT_OF_RANGE",
                    "An item has a negative start or an empty range.",
                    "Move it onto the timeline and give it a positive length.",
                ));
            }
            let previous = time::find_item(&base.items, id, slot)
                .and_then(|slot| time::item_span(base, slot).ok());
            let touched = previous != Some(span);
            order.push(OrderedItem {
                touched,
                slot,
                id,
                span,
            });
        }
        order.sort_unstable_by(|a, b| {
            (a.touched, a.span.start)
                .cmp(&(b.touched, b.span.start))
                .then_with(|| serial_key(a.id.serial()).cmp(&serial_key(b.id.serial())))
        });
        for &OrderedItem { slot, id, span, .. } in order.iter() {
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
            let index = interval_index(lanes, lane);
            if let Some(at) = lanes[index].insertion(span) {
                lanes[index].spans.insert(at, span);
                continue;
            }
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
                .position(|cand| cand.id == lane)
                .expect("current lane");
            let available = self.project.lanes[..index]
                .iter()
                .rev()
                .find_map(|candidate| {
                    if candidate.kind != kind {
                        return None;
                    }
                    let index = lanes.iter().position(|entry| entry.id == candidate.id)?;
                    lanes[index].insertion(span).map(|at| (index, at))
                });
            let (index, at) = match available {
                Some(pair) => pair,
                None => {
                    let to = self.new_lane(kind, None)?;
                    (interval_index(lanes, to), 0)
                }
            };
            let to = lanes[index].id;
            lanes[index].spans.insert(at, span);
            self.project.items.data_mut(slot).expect("live item").lane = to;
            self.report
                .side_effects
                .push(SideEffect::Bumped { id, from: lane, to });
        }
        Ok(())
    }

    fn rekey_effects(&mut self, base: &Project) -> Result<(), CommandError> {
        let mut owners = std::mem::take(&mut self.settle_scratch.owners);
        let mut seen = std::mem::take(&mut self.settle_scratch.seen);
        let result = self.rekey(base, &mut owners, &mut seen);
        self.settle_scratch.owners = owners;
        self.settle_scratch.seen = seen;
        result
    }

    fn rekey(
        &mut self,
        base: &Project,
        owners: &mut Vec<(bool, usize, Owner)>,
        seen: &mut Vec<u64>,
    ) -> Result<(), CommandError> {
        owners.clear();
        seen.clear();
        let all = [Owner::Project, Owner::Spine]
            .into_iter()
            .chain((0..self.project.lanes.len()).map(Owner::Lane))
            .chain(self.project.clips.order().iter().copied().map(Owner::Clip))
            .chain(self.project.items.order().iter().copied().map(Owner::Item));
        for (index, owner) in all.enumerate() {
            if effects(&self.project, owner).is_empty() {
                continue;
            }
            let new = match owner {
                Owner::Project | Owner::Spine => false,
                Owner::Lane(index) => !base
                    .lanes
                    .iter()
                    .any(|lane| lane.id == self.project.lanes[index].id),
                Owner::Clip(slot) => base
                    .clips
                    .find(self.project.clips.id(slot).expect("live clip"))
                    .is_none(),
                Owner::Item(slot) => time::find_item(
                    &base.items,
                    self.project.items.id(slot).expect("live item"),
                    slot,
                )
                .is_none(),
            };
            owners.push((new, index, owner));
        }
        // Existing owners keep their IDs ahead of copies. Preserve traversal order within each group.
        owners.sort_unstable_by_key(|&(new, index, _)| (new, index));
        for &(_, _, owner) in owners.iter() {
            for index in 0..effects(&self.project, owner).len() {
                let mut serial = effects(&self.project, owner)[index].id.serial();
                let at = match seen.binary_search(&serial) {
                    Err(at) => at,
                    Ok(_) => {
                        serial = self.mint("fx")?;
                        effects_mut(&mut self.project, owner)[index].id =
                            EffectId::from_serial(serial);
                        seen.binary_search(&serial).expect_err("fresh effect ID")
                    }
                };
                seen.insert(at, serial);
            }
        }
        Ok(())
    }
}

fn interval_index(lanes: &mut Vec<LaneIntervals>, id: LaneId) -> usize {
    if let Some(index) = lanes.iter().position(|lane| lane.id == id) {
        return index;
    }
    lanes.push(LaneIntervals {
        id,
        spans: Vec::new(),
    });
    lanes.len() - 1
}

// Base36 wire IDs sort lexically, including different digit lengths.
fn serial_key(mut serial: u64) -> [u8; 13] {
    let mut digits = [0; 13];
    let mut index = digits.len();
    loop {
        index -= 1;
        digits[index] = (serial % 36) as u8 + 1;
        serial /= 36;
        if serial == 0 {
            digits.rotate_left(index);
            return digits;
        }
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
