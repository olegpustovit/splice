use super::edits::{anchor_parent, arena_full};
use super::time::{anchor_at, item_slot};
use super::*;
use crate::commands::{AnchorMode, FrameArg, LaneMove};

impl Editor {
    pub(super) fn set_items(
        &mut self,
        selection: &[Target],
        name: &Option<Option<String>>,
        color: &Option<Option<String>>,
        fade_in: Option<Frame>,
        fade_out: Option<Frame>,
    ) -> Result<Outcome, CommandError> {
        let mut ids = std::mem::take(&mut self.selection_scratch);
        let result = self
            .selection_into(selection, &mut ids)
            .and_then(|()| self.set_selected(&ids, name, color, fade_in, fade_out));
        self.selection_scratch = ids;
        result
    }

    fn set_selected(
        &mut self,
        ids: &[TimelineId],
        name: &Option<Option<String>>,
        color: &Option<Option<String>>,
        fade_in: Option<Frame>,
        fade_out: Option<Frame>,
    ) -> Result<Outcome, CommandError> {
        for &id in ids {
            match id {
                TimelineId::Clip(id) => {
                    let slot = time::clip_slot(&self.project, id)?;
                    let length = self.project.clips.timing(slot).expect("live clip").length;
                    let appearance = &mut self
                        .project
                        .clips
                        .data_mut(slot)
                        .expect("live clip")
                        .appearance;
                    set_appearance(appearance, length, name, color, fade_in, fade_out)?;
                }
                TimelineId::Item(id) => {
                    let slot = item_slot(&self.project, id)?;
                    let length = self.project.items.timing(slot).expect("live item").length;
                    let appearance = &mut self
                        .project
                        .items
                        .data_mut(slot)
                        .expect("live item")
                        .appearance;
                    set_appearance(appearance, length, name, color, fade_in, fade_out)?;
                }
                TimelineId::Marker(id) => {
                    if fade_in.is_some() || fade_out.is_some() {
                        return Err(wrong_kind(
                            "Fades apply to clips and overlays with a duration.",
                        ));
                    }
                    let marker = self
                        .project
                        .markers
                        .iter_mut()
                        .find(|marker| marker.id == id)
                        .expect("checked marker");
                    if let Some(name) = name {
                        marker.name.clear();
                        marker.name.push_str(name.as_deref().unwrap_or_default());
                    }
                    if let Some(color) = color {
                        marker.color.clone_from(color);
                    }
                }
            }
        }
        Ok(Outcome::with_summary(
            Summary::Update(ids.len()),
            Target::Timeline(ids[0]),
        ))
    }

    pub(super) fn set_anchor(
        &mut self,
        target: &Target,
        mode: &AnchorMode,
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            return Err(wrong_kind("Use an overlay ID for item.anchor."));
        };
        let slot = self.editable_item(id)?;
        let current = time::span(&self.project, TimelineId::Item(id))?.start;
        let anchor = match mode {
            AnchorMode::Current(true) => Anchor::Pinned(current),
            AnchorMode::Current(false) => anchor_at(&self.project, current)?,
            AnchorMode::Explicit(anchor) => self.explicit_anchor(anchor, current)?,
        };
        self.project
            .items
            .timing_mut(slot)
            .expect("live item")
            .anchor = anchor;
        if time::span(&self.project, TimelineId::Item(id))?.start < 0 {
            return Err(CommandError::bad(
                "anchor",
                "a position at or after frame zero",
            ));
        }
        Ok(Outcome::edit(
            format!("Set anchor on {id}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    pub(super) fn set_overlay(
        &mut self,
        target: &Target,
        placement: Option<Placement>,
        behind: Option<bool>,
        muted: Option<bool>,
    ) -> Result<Outcome, CommandError> {
        let id = self.overlay_id(target)?;
        let slot = item_slot(&self.project, id)?;
        let data = self.project.items.data_mut(slot).expect("live item");
        let ItemKind::Media(overlay) = &mut data.kind else {
            return Err(wrong_kind("Use a media overlay for overlay.set."));
        };
        if let Some(placement) = placement {
            overlay.placement = placement;
        }
        if let Some(behind) = behind {
            overlay.behind_presenter = behind;
        }
        if let Some(muted) = muted {
            overlay.muted = muted;
        }
        Ok(Outcome::edit(
            format!("Updated overlay {id}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    pub(super) fn set_title(
        &mut self,
        target: &Target,
        text: String,
    ) -> Result<Outcome, CommandError> {
        let id = self.overlay_id(target)?;
        let slot = item_slot(&self.project, id)?;
        let ItemKind::Title { text: title } =
            &mut self.project.items.data_mut(slot).expect("live item").kind
        else {
            return Err(wrong_kind("Use a title item for title.set."));
        };
        *title = text;
        Ok(Outcome::edit(
            format!("Updated title {id}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    fn overlay_id(&self, target: &Target) -> Result<ItemId, CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            return Err(wrong_kind("Use an overlay item ID from timeline.snapshot."));
        };
        Ok(id)
    }

    pub(super) fn set_marker(
        &mut self,
        target: &Target,
        name: Option<String>,
        at: Option<&FrameArg>,
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Marker(id) = self.timeline(target)? else {
            return Err(wrong_kind("Use a marker ID for marker.set."));
        };
        let at = at.map(|at| self.frame(at)).transpose()?;
        let marker = self
            .project
            .markers
            .iter_mut()
            .find(|marker| marker.id == id)
            .ok_or_else(|| not_found(&id, "Call timeline.find for markers."))?;
        if let Some(name) = name {
            marker.name = if name.is_empty() {
                "Marker".into()
            } else {
                name
            };
        }
        if let Some(at) = at {
            marker.at = at.max(0);
        }
        self.project.markers.sort_by_key(|marker| marker.at);
        Ok(Outcome::edit(
            format!("Updated marker {id}"),
            Target::Timeline(TimelineId::Marker(id)),
        ))
    }

    pub(super) fn set_project(
        &mut self,
        aspect: Option<Aspect>,
        access: Option<Access>,
    ) -> Result<Outcome, CommandError> {
        if let Some(access) = access {
            if self.source == Source::Agent {
                return Err(error(
                    "PERMISSION_DENIED",
                    "Agents cannot change project access.",
                    "Ask the human editor to change access.",
                ));
            }
            self.project.access = access;
        }
        if let Some(aspect) = aspect {
            let [width, height] = self.project.size;
            if aspect != self.project.aspect {
                let short = width.min(height);
                let scaled = |numerator: u64, denominator: u64| {
                    u32::try_from((u64::from(short) * numerator + denominator / 2) / denominator)
                        .map_err(|_| {
                            error(
                                "OUT_OF_RANGE",
                                "The new canvas is too large.",
                                "Reduce the canvas size before changing aspect.",
                            )
                        })
                };
                self.project.size = match aspect {
                    Aspect::Portrait => [short, scaled(16, 9)?],
                    Aspect::Social => [short, scaled(5, 4)?],
                    Aspect::Square => [short, short],
                    Aspect::Standard => [scaled(4, 3)?, short],
                    Aspect::Landscape => [scaled(16, 9)?, short],
                };
            }
            self.project.aspect = aspect;
        }
        Ok(Outcome::edit(
            "Updated project settings".into(),
            Target::Project,
        ))
    }

    pub(super) fn rename_lane(
        &mut self,
        target: &Target,
        name: String,
    ) -> Result<Outcome, CommandError> {
        let index = self.lane_index(target)?;
        let lane = &mut self.project.lanes[index];
        if !name.is_empty() {
            lane.name = name;
        }
        Ok(Outcome::edit(
            format!("Renamed lane to {}", lane.name),
            Target::Lane(lane.id),
        ))
    }

    pub(super) fn delete_lane(&mut self, target: &Target) -> Result<Outcome, CommandError> {
        let index = self.lane_index(target)?;
        let lane = &self.project.lanes[index];
        let id = lane.id;
        let kind = lane.kind;
        let other = self.project.lanes[index + 1..]
            .iter()
            .find(|lane| lane.kind == kind)
            .or_else(|| {
                self.project.lanes[..index]
                    .iter()
                    .rev()
                    .find(|lane| lane.kind == kind)
            })
            .map(|lane| lane.id);
        for index in 0..self.project.items.order().len() {
            let slot = self.project.items.order()[index];
            let data = self.project.items.data_mut(slot).expect("live item");
            if data.lane == id {
                let to = other.ok_or_else(|| {
                    error(
                        "LANE_NOT_EMPTY",
                        "The last lane of its kind still contains items.",
                        "Move or delete the items before removing the lane.",
                    )
                })?;
                data.lane = to;
                let item = self.project.items.id(slot).expect("live item");
                self.report.side_effects.push(SideEffect::Bumped {
                    id: item,
                    from: id,
                    to,
                    cause: Cause::LaneDeleted,
                });
            }
        }
        self.project.lanes.remove(index);
        Ok(Outcome::edit(
            format!("Deleted lane {id}"),
            Target::Lane(id),
        ))
    }

    pub(super) fn move_to_lane(
        &mut self,
        items: &[Target],
        destination: &LaneMove,
    ) -> Result<Outcome, CommandError> {
        let ids = self.selection(items)?;
        for &id in &ids {
            let TimelineId::Item(id) = id else {
                return Err(wrong_kind("Use overlay items for item.lane."));
            };
            let slot = item_slot(&self.project, id)?;
            let item = self.project.items.data(slot).expect("live item");
            let kind = item.kind.lane_kind();
            let lane = match destination {
                LaneMove::To(lane) => {
                    let lane = &self.project.lanes[self.lane_index(lane)?];
                    if lane.kind != kind {
                        return Err(wrong_kind("Select a lane of the item's kind."));
                    }
                    self.explicit_lanes.push(id);
                    lane.id
                }
                LaneMove::Up | LaneMove::Down => {
                    let up = matches!(destination, LaneMove::Up);
                    match self.free_lane(id, item.lane, kind, up)? {
                        Some(id) => id,
                        None if up => self.new_lane(kind, None)?,
                        None => {
                            return Err(error(
                                "LANE_CONFLICT",
                                "There is no free lane below the item.",
                                "Move to an explicit empty lane or choose up.",
                            ));
                        }
                    }
                }
            };
            self.project.items.data_mut(slot).expect("live item").lane = lane;
        }
        Ok(Outcome::edit(
            format!("Moved {} item(s) to another lane", ids.len()),
            Target::Timeline(ids[0]),
        ))
    }

    fn lane_index(&self, target: &Target) -> Result<usize, CommandError> {
        let Target::Lane(id) = self.resolve(target)? else {
            return Err(wrong_kind("Use a lane ID from project.info."));
        };
        self.project
            .lanes
            .iter()
            .position(|lane| lane.id == id)
            .ok_or_else(|| not_found(&id, "Call project.info for secondary lane IDs."))
    }

    fn free_lane(
        &self,
        id: ItemId,
        lane: LaneId,
        kind: LaneKind,
        up: bool,
    ) -> Result<Option<LaneId>, CommandError> {
        let index = self.lane_index(&Target::Lane(lane))?;
        let span = time::span(&self.project, TimelineId::Item(id))?;
        let candidates = if up {
            &self.project.lanes[..index]
        } else {
            &self.project.lanes[index + 1..]
        };
        for index in 0..candidates.len() {
            let candidate = &candidates[if up {
                candidates.len() - 1 - index
            } else {
                index
            }];
            if candidate.kind != kind {
                continue;
            }
            let mut free = true;
            for &slot in self.project.items.order() {
                let other = self.project.items.id(slot).expect("live item");
                if other != id
                    && self.project.items.data(slot).expect("live item").lane == candidate.id
                {
                    let other = time::span(&self.project, TimelineId::Item(other))?;
                    if other.start < span.end && other.end > span.start {
                        free = false;
                        break;
                    }
                }
            }
            if free {
                return Ok(Some(candidate.id));
            }
        }
        Ok(None)
    }

    pub(super) fn duplicate(
        &mut self,
        selection: &[Target],
        at: Option<&FrameArg>,
    ) -> Result<Outcome, CommandError> {
        let ids = self.selection(selection)?;
        let at = at.map(|at| self.frame(at)).transpose()?;
        if at.is_some_and(|at| at < 0) {
            return Err(CommandError::bad("at", "a nonnegative frame"));
        }
        let mut created = Vec::new();
        self.duplicate_clips(&ids, at, &mut created)?;
        self.duplicate_items(&ids, at, &mut created)?;
        for id in &ids {
            if let TimelineId::Marker(id) = id {
                let mut marker = self
                    .project
                    .markers
                    .iter()
                    .find(|marker| marker.id == *id)
                    .expect("checked marker")
                    .clone();
                marker.id = MarkerId::from_serial(self.mint("mrk")?);
                marker.at = match at {
                    Some(at) => at,
                    None => frame_add(marker.at, 30)?,
                };
                self.check_marker_frame(marker.at)?;
                created.push(Target::Timeline(TimelineId::Marker(marker.id)));
                self.project.markers.push(marker);
            }
        }
        self.project.markers.sort_by_key(|marker| marker.at);
        Ok(Outcome {
            summary: Summary::Text(format!("Duplicated {} item(s)", ids.len())),
            primary: created.first().cloned(),
            created,
        })
    }

    fn duplicate_clips(
        &mut self,
        ids: &[TimelineId],
        at: Option<Frame>,
        created: &mut Vec<Target>,
    ) -> Result<(), CommandError> {
        let mut clips = Vec::new();
        for &id in ids {
            if let TimelineId::Clip(id) = id {
                let slot = time::clip_slot(&self.project, id)?;
                clips.push((
                    id,
                    slot,
                    self.project.clips.timing(slot).expect("live clip"),
                ));
            }
        }
        clips.sort_by_key(|&(_, _, timing)| timing.start);
        let (Some(&(_, _, first)), Some(&(_, _, last))) = (clips.first(), clips.last()) else {
            return Ok(());
        };
        let end = last.end().ok_or_else(frame_range)?;
        let position = self.snap_insertion(at.unwrap_or(end))?;
        self.shift_after(position, end - first.start, None)?;
        let original_items = self.project.items.order().to_vec();
        for (id, slot, mut timing) in clips {
            timing.start = frame_add(position, timing.start - first.start)?;
            let copied = ClipId::from_serial(self.mint("clp")?);
            let data = self.project.clips.data(slot).expect("live clip").clone();
            self.project
                .clips
                .insert(Clip {
                    id: copied,
                    timing,
                    data,
                })
                .ok_or_else(arena_full)?;
            created.push(Target::Timeline(TimelineId::Clip(copied)));
            self.duplicate_children(
                TimelineId::Clip(id),
                TimelineId::Clip(copied),
                &original_items,
                created,
            )?;
        }
        self.project.clips.sort_by_start();
        Ok(())
    }

    fn duplicate_children(
        &mut self,
        from: TimelineId,
        to: TimelineId,
        original_items: &[Slot],
        created: &mut Vec<Target>,
    ) -> Result<(), CommandError> {
        let mut parents = vec![(from, to)];
        let mut index = 0;
        while let Some(&(from, to)) = parents.get(index) {
            for &slot in original_items {
                let mut timing = self.project.items.timing(slot).expect("live item");
                if anchor_parent(timing.anchor) != Some(from) {
                    continue;
                }
                timing.anchor = match (timing.anchor, to) {
                    (Anchor::ClipSource { source, .. }, TimelineId::Clip(clip)) => {
                        Anchor::ClipSource { clip, source }
                    }
                    (Anchor::Item { edge, offset, .. }, _) => Anchor::Item {
                        item: to,
                        edge,
                        offset,
                    },
                    _ => unreachable!("a copied parent is a clip or item"),
                };
                let original = self.project.items.id(slot).expect("live item");
                let copied = self.duplicate_item(slot, timing, created)?;
                parents.push((TimelineId::Item(original), TimelineId::Item(copied)));
            }
            index += 1;
        }
        Ok(())
    }

    fn duplicate_items(
        &mut self,
        ids: &[TimelineId],
        at: Option<Frame>,
        created: &mut Vec<Target>,
    ) -> Result<(), CommandError> {
        let mut items = Vec::new();
        for &id in ids {
            if let TimelineId::Item(item) = id {
                let slot = self.editable_item(item)?;
                items.push((slot, time::span(&self.project, id)?));
            }
        }
        let Some(start) = items.iter().map(|(_, span)| span.start).min() else {
            return Ok(());
        };
        let position = at.unwrap_or_else(|| {
            items
                .iter()
                .map(|(_, span)| span.end)
                .max()
                .expect("selected item")
        });
        for (slot, span) in items {
            let frame = frame_add(position, span.start - start)?;
            let mut timing = self.project.items.timing(slot).expect("live item");
            timing.anchor = if matches!(timing.anchor, Anchor::Pinned(_)) {
                Anchor::Pinned(frame)
            } else {
                anchor_at(&self.project, frame)?
            };
            self.duplicate_item(slot, timing, created)?;
        }
        Ok(())
    }

    fn duplicate_item(
        &mut self,
        slot: Slot,
        timing: ItemTiming,
        created: &mut Vec<Target>,
    ) -> Result<ItemId, CommandError> {
        let id = ItemId::from_serial(self.mint("ovl")?);
        let data = self.project.items.data(slot).expect("live item").clone();
        self.project
            .items
            .insert(Item { id, timing, data })
            .ok_or_else(arena_full)?;
        created.push(Target::Timeline(TimelineId::Item(id)));
        Ok(id)
    }
}

fn set_appearance(
    appearance: &mut Appearance,
    length: Frame,
    name: &Option<Option<String>>,
    color: &Option<Option<String>>,
    fade_in: Option<Frame>,
    fade_out: Option<Frame>,
) -> Result<(), CommandError> {
    // Fades are refused, not clamped, when they don't fit the item (Low list).
    let (fade_in_to, fade_out_to) = (
        fade_in.unwrap_or(appearance.fade_in),
        fade_out.unwrap_or(appearance.fade_out),
    );
    for (param, value) in [("fadeIn", fade_in), ("fadeOut", fade_out)] {
        if let Some(value) = value
            && (value < 0 || fade_in_to + fade_out_to > length)
        {
            return Err(CommandError::rule(
                "BAD_ARGS",
                param,
                Some(Json::Signed(i64::from(value))),
                &format!("0 or more frames, with fadeIn + fadeOut at most the length, {length}"),
                "Shorten the fades or lengthen the item first.",
            ));
        }
    }
    if let Some(name) = name {
        if let Some(name) = name.as_ref().filter(|name| !name.is_empty()) {
            appearance
                .name
                .get_or_insert_with(String::new)
                .clone_from(name);
        } else {
            appearance.name = None;
        }
    }
    if let Some(color) = color {
        appearance.color.clone_from(color);
    }
    appearance.fade_in = fade_in_to;
    appearance.fade_out = fade_out_to;
    Ok(())
}
