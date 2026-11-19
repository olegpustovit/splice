use super::edits::{anchor_parent, arena_full, clamp_fades};
use super::time::{anchor_at, clip_slot, item_slot, media, rounded, source_rate, source_value};
use super::*;
use crate::commands::{FrameArg, InsertSource, Position, TrimEdge};

#[derive(Default)]
pub(super) struct MoveScratch {
    clips: Vec<Slot>,
    kept: Vec<(Slot, Frame)>,
    ordered: Vec<(Slot, i64)>,
    gaps: Vec<Frame>,
    markers: Vec<(Frame, usize)>,
}

impl Editor {
    pub(super) fn trim(
        &mut self,
        target: &Target,
        edge: TrimEdge,
        position: &Position,
        ripple: bool,
    ) -> Result<Outcome, CommandError> {
        let id = self.timeline(target)?;
        match id {
            TimelineId::Marker(_) => return Err(wrong_kind("Use marker.set to move a marker.")),
            TimelineId::Item(id) => {
                self.editable_item(id)?;
            }
            TimelineId::Clip(_) => {}
        }
        let span = time::span(&self.project, id)?;
        let from = if edge == TrimEdge::In {
            span.start
        } else {
            span.end
        };
        let to = self.destination(position, from)?;
        let length = match edge {
            TrimEdge::In => subtract(span.end, to)?,
            TrimEdge::Out => subtract(to, span.start)?,
        };
        ensure_length(length)?;
        match id {
            TimelineId::Clip(clip) => {
                let slot = clip_slot(&self.project, clip)?;
                let old = self.project.clips.timing(slot).expect("live clip");
                let mut timing = ClipTiming { length, ..old };
                if edge == TrimEdge::In {
                    timing.source = source_for_interval(&self.project, slot, to, length)?;
                    if !ripple {
                        timing.start = to;
                    }
                }
                self.resize_clip(slot, timing, ripple)?;
            }
            TimelineId::Item(id) => {
                let slot = self.editable_item(id)?;
                if edge == TrimEdge::In {
                    let anchor = self.anchor_at_move(slot, to)?;
                    self.project
                        .items
                        .timing_mut(slot)
                        .expect("live item")
                        .anchor = anchor;
                    let data = self.project.items.data(slot).expect("live item");
                    if let ItemKind::Media(overlay) = data.kind {
                        let descriptor = media(&self.project, overlay.media)?;
                        let delta = f64::from(subtract(to, from)?) * descriptor.probe.fps
                            / f64::from(self.project.fps);
                        let source = rounded((f64::from(overlay.source) + delta).max(0.0))?;
                        if source >= descriptor.probe.frames {
                            return Err(out_of_source());
                        }
                        if let ItemKind::Media(overlay) =
                            &mut self.project.items.data_mut(slot).expect("live item").kind
                        {
                            overlay.source = source;
                        }
                    }
                }
                self.project
                    .items
                    .timing_mut(slot)
                    .expect("live item")
                    .length = length;
                clamp_fades(
                    &mut self
                        .project
                        .items
                        .data_mut(slot)
                        .expect("live item")
                        .appearance,
                    length,
                );
            }
            TimelineId::Marker(_) => return Err(wrong_kind("Use marker.set to move a marker.")),
        }
        Ok(Outcome::with_summary(
            Summary::Trim(id, to),
            Target::Timeline(id),
        ))
    }

    pub(super) fn speed(
        &mut self,
        selection: &[Target],
        speed: f64,
        ripple: bool,
    ) -> Result<Outcome, CommandError> {
        let ids = self.selection(selection)?;
        let mut clips = Vec::with_capacity(ids.len());
        for id in &ids {
            let TimelineId::Clip(clip) = *id else {
                return Err(wrong_kind("Use spine clips for clip.speed."));
            };
            let slot = clip_slot(&self.project, clip)?;
            clips.push((
                self.project.clips.timing(slot).expect("live clip").start,
                slot,
            ));
        }
        clips.sort_by_key(|&(start, _)| std::cmp::Reverse(start));
        for (_, slot) in clips {
            let old = self.project.clips.timing(slot).expect("live clip");
            let mut length = rounded(f64::from(old.length) * old.speed / speed)?.max(3);
            if let Some(id) = self.project.clips.data(slot).expect("live clip").media {
                let descriptor = media(&self.project, id)?;
                let rate = source_rate(&self.project, slot)? / old.speed * speed;
                // Integer timeline frames may round past the media's last source frame.
                let available =
                    (f64::from(descriptor.probe.frames - old.source) / rate + 0.000_001).floor();
                if available < f64::from(length) {
                    length = rounded(available)?;
                }
            }
            ensure_length(length)?;
            self.resize_clip(
                slot,
                ClipTiming {
                    length,
                    speed,
                    ..old
                },
                ripple,
            )?;
        }
        Ok(Outcome::edit(
            format!("Set speed to {speed}× on {} clip(s)", ids.len()),
            Target::Timeline(ids[0]),
        ))
    }

    fn resize_clip(
        &mut self,
        slot: Slot,
        timing: ClipTiming,
        ripple: bool,
    ) -> Result<(), CommandError> {
        let old = self.project.clips.timing(slot).expect("live clip");
        validate_source(&self.project, slot, timing)?;
        let end = timing.end().ok_or_else(frame_range)?;
        if timing.start < 0 {
            return Err(CommandError::bad(
                "to",
                "a nonnegative resulting clip start",
            ));
        }
        if ripple {
            self.shift_after(
                old.end().ok_or_else(frame_range)?,
                timing.length - old.length,
                Some(slot),
            )?;
        } else {
            self.check_overlap(timing.start, end, Some(slot))?;
        }
        *self.project.clips.timing_mut(slot).expect("live clip") = timing;
        clamp_fades(
            &mut self
                .project
                .clips
                .data_mut(slot)
                .expect("live clip")
                .appearance,
            timing.length,
        );
        Ok(())
    }

    pub(super) fn insert_clip(
        &mut self,
        source: InsertSource,
        at: Option<&FrameArg>,
        ripple: bool,
        insert: bool,
        lane: Option<&Target>,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<Outcome, CommandError> {
        let media_id = match source {
            InsertSource::Media(target) => {
                let Target::Media(id) = self.resolve(&target)? else {
                    return Err(wrong_kind("Use a media ID from project.info."));
                };
                id
            }
            InsertSource::Path(path) => self.import_path(path, load)?,
        };
        if !insert {
            return Ok(Outcome::create(
                format!("Imported {media_id}"),
                Target::Media(media_id),
            ));
        }
        let descriptor = media(&self.project, media_id)?;
        if descriptor.kind == MediaKind::Audio || lane.is_some() {
            return self.add_sound(media_id, at, lane);
        }
        let length = rounded(
            (f64::from(descriptor.probe.frames) * f64::from(self.project.fps)
                / descriptor.probe.fps
                + 0.000_001)
                .floor(),
        )?;
        ensure_length(length)?;
        let enabled = descriptor
            .take
            .as_ref()
            .map_or_else(ComponentStates::default, |take| ComponentStates {
                screen: take.screen.as_ref().map(|_| true),
                camera: take.camera.as_ref().map(|_| true),
                mic: take.mic.as_ref().map(|_| true),
                system: take.system.as_ref().map(|_| true),
                pointer: take.events.as_ref().map(|_| true),
                keys: take.events.as_ref().map(|_| true),
            });
        let at = at
            .map(|at| self.frame(at))
            .transpose()?
            .unwrap_or(self.spine_end()?);
        let start = self.snap_insertion(at)?;
        let end = frame_add(start, length)?;
        if ripple {
            self.shift_after(start, length, None)?;
        } else {
            self.check_overlap(start, end, None)?;
        }
        let id = ClipId::from_serial(self.mint("clp")?);
        self.project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start,
                    length,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media_id),
                    enabled,
                    ..ClipData::default()
                },
            })
            .ok_or_else(arena_full)?;
        self.project.clips.sort_by_start();
        Ok(Outcome::create(
            format!("Inserted {id} at frame {start}"),
            Target::Timeline(TimelineId::Clip(id)),
        ))
    }

    fn import_path(
        &mut self,
        request: String,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<MediaId, CommandError> {
        if let Some(media) = self
            .project
            .media
            .iter()
            .find(|media| media.path == request)
        {
            return Ok(media.id);
        }
        if let Some(import) = self.imports.iter().find(|import| import.request == request) {
            return Ok(import.descriptor.id);
        }

        let mut descriptor = load(&request)?;
        if let Some(existing) = self
            .project
            .media
            .iter()
            .find(|media| media.path == descriptor.path)
        {
            descriptor = existing.clone();
        } else {
            descriptor.id = MediaId::from_serial(self.mint("med")?);
            self.project.media.push(descriptor.clone());
        }
        let id = descriptor.id;
        self.imports.push(MediaImport {
            request,
            descriptor,
        });
        Ok(id)
    }

    pub(super) fn move_items(
        &mut self,
        selection: &[Target],
        position: &Position,
        carry_overlays: bool,
        ripple: bool,
    ) -> Result<Outcome, CommandError> {
        let mut ids = std::mem::take(&mut self.selection_scratch);
        let mut scratch = std::mem::take(&mut self.move_scratch);
        let result = self.selection_into(selection, &mut ids).and_then(|()| {
            self.move_selected(&ids, position, carry_overlays, ripple, &mut scratch)
        });
        self.selection_scratch = ids;
        self.move_scratch = scratch;
        result
    }

    fn move_selected(
        &mut self,
        ids: &[TimelineId],
        position: &Position,
        carry_overlays: bool,
        ripple: bool,
        scratch: &mut MoveScratch,
    ) -> Result<Outcome, CommandError> {
        let start = time::span(&self.project, ids[0])?.start;
        let delta = subtract(self.destination(position, start)?, start)?;
        for id in ids {
            if let TimelineId::Item(item) = *id {
                self.editable_item(item)?;
            }
        }
        let MoveScratch {
            clips,
            kept,
            ordered,
            gaps,
            markers,
        } = scratch;
        clips.clear();
        clips.extend(ids.iter().filter_map(|id| match id {
            TimelineId::Clip(id) => self.project.clips.find(*id),
            _ => None,
        }));
        kept.clear();
        if !clips.is_empty() {
            if !carry_overlays {
                for &slot in self.project.items.order() {
                    let timing = self.project.items.timing(slot).expect("live item");
                    if anchor_parent(timing.anchor).is_some_and(|parent| {
                        ids.contains(&parent) && matches!(parent, TimelineId::Clip(_))
                    }) {
                        kept.push((slot, time::anchor_start(&self.project, timing.anchor)?));
                    }
                }
            }
            if let ([slot], true) = (clips.as_slice(), ripple) {
                self.move_magnetic(*slot, delta, ordered, gaps)?;
            } else {
                for &slot in clips.iter() {
                    let timing = self.project.clips.timing_mut(slot).expect("live clip");
                    let start = frame_add(timing.start, delta)?;
                    if start < 0 {
                        return Err(CommandError::bad("to", "a nonnegative clip start"));
                    }
                    timing.start = start;
                }
                for &slot in clips.iter() {
                    let timing = self.project.clips.timing(slot).expect("live clip");
                    self.check_overlap(
                        timing.start,
                        timing.end().ok_or_else(frame_range)?,
                        Some(slot),
                    )?;
                }
            }
            self.project.clips.sort_by_start();
            for &(slot, frame) in kept.iter() {
                let anchor = anchor_at(&self.project, frame)?;
                self.reanchor_if_parent_changed(slot, anchor);
            }
        }
        for &id in ids {
            match id {
                TimelineId::Clip(_) => {}
                TimelineId::Item(id) => {
                    let slot = item_slot(&self.project, id)?;
                    let start = time::span(&self.project, TimelineId::Item(id))?.start;
                    let at = frame_add(start, delta)?;
                    let anchor = self.anchor_at_move(slot, at)?;
                    self.reanchor_if_parent_changed(slot, anchor);
                }
                TimelineId::Marker(id) => {
                    let marker = self
                        .project
                        .markers
                        .iter_mut()
                        .find(|marker| marker.id == id)
                        .expect("checked marker");
                    marker.at = frame_add(marker.at, delta)?.max(0);
                }
            }
        }
        sort_markers(&mut self.project.markers, markers);
        Ok(Outcome::with_summary(
            Summary::Move(ids.len(), delta),
            Target::Timeline(ids[0]),
        ))
    }

    fn move_magnetic(
        &mut self,
        slot: Slot,
        delta: Frame,
        ordered: &mut Vec<(Slot, i64)>,
        gaps: &mut Vec<Frame>,
    ) -> Result<(), CommandError> {
        let original = self.project.clips.timing(slot).expect("live clip");
        let start = frame_add(original.start, delta)?.max(0);
        let end = frame_add(start, original.length)?;
        if self.overlaps(start, end, Some(slot)) {
            ordered.clear();
            gaps.clear();
            let mut last_end = 0;
            for &other in self.project.clips.order() {
                let timing = self.project.clips.timing(other).expect("live clip");
                gaps.push(subtract(timing.start, last_end)?);
                last_end = timing.end().ok_or_else(frame_range)?;
                if other != slot {
                    ordered.push((
                        other,
                        i64::from(timing.start) * 2 + i64::from(timing.length),
                    ));
                }
            }
            let center = i64::from(start) * 2 + i64::from(original.length);
            let index = ordered
                .iter()
                .position(|&(_, other)| center < other)
                .unwrap_or(ordered.len());
            ordered.insert(index, (slot, center));
            let mut at = 0;
            for (&(slot, _), &gap) in ordered.iter().zip(gaps.iter()) {
                let timing = self.project.clips.timing_mut(slot).expect("live clip");
                timing.start = frame_add(at, gap)?;
                at = timing.end().ok_or_else(frame_range)?;
            }
        } else {
            self.project
                .clips
                .timing_mut(slot)
                .expect("live clip")
                .start = start;
        }
        Ok(())
    }

    fn anchor_at_move(&self, slot: Slot, at: Frame) -> Result<Anchor, CommandError> {
        if at < 0 {
            return Err(CommandError::bad("to", "a nonnegative timeline frame"));
        }
        let anchor = self.project.items.timing(slot).expect("live item").anchor;
        if matches!(anchor, Anchor::Pinned(_)) {
            Ok(Anchor::Pinned(at))
        } else {
            anchor_at(&self.project, at)
        }
    }

    fn reanchor_if_parent_changed(&mut self, slot: Slot, to: Anchor) {
        let timing = self.project.items.timing_mut(slot).expect("live item");
        if anchor_parent(timing.anchor) != anchor_parent(to) {
            self.reanchor(slot, to);
        } else {
            timing.anchor = to;
        }
    }

    fn destination(&self, position: &Position, from: Frame) -> Result<Frame, CommandError> {
        match position {
            Position::To(value) => self.frame(value),
            Position::Delta(delta) => frame_add(from, *delta),
        }
    }

    pub(super) fn snap_insertion(&self, frame: Frame) -> Result<Frame, CommandError> {
        if frame < 0 {
            return Err(CommandError::bad("at", "a nonnegative timeline frame"));
        }
        for &slot in self.project.clips.order() {
            let timing = self.project.clips.timing(slot).expect("live clip");
            if timing.contains(frame) {
                let offset = i64::from(frame) - i64::from(timing.start);
                return if offset * 2 < i64::from(timing.length) {
                    Ok(timing.start)
                } else {
                    timing.end().ok_or_else(frame_range)
                };
            }
        }
        Ok(frame)
    }

    pub(super) fn spine_end(&self) -> Result<Frame, CommandError> {
        self.project.clips.order().last().map_or(Ok(0), |&slot| {
            self.project
                .clips
                .timing(slot)
                .expect("live clip")
                .end()
                .ok_or_else(frame_range)
        })
    }

    fn overlaps(&self, start: Frame, end: Frame, except: Option<Slot>) -> bool {
        self.project.clips.order().iter().any(|&slot| {
            let clip = self.project.clips.timing(slot).expect("live clip");
            Some(slot) != except
                && clip.start < end
                && i64::from(clip.start) + i64::from(clip.length) > i64::from(start)
        })
    }

    fn check_overlap(
        &self,
        start: Frame,
        end: Frame,
        except: Option<Slot>,
    ) -> Result<(), CommandError> {
        if self.overlaps(start, end, except) {
            return Err(error(
                "OVERLAP",
                "The edit overlaps another spine clip.",
                "Enable ripple or move the edit into a gap.",
            ));
        }
        Ok(())
    }

    pub(super) fn editable_item(&self, id: ItemId) -> Result<Slot, CommandError> {
        let slot = item_slot(&self.project, id)?;
        if matches!(
            self.project.items.data(slot).expect("live item").kind,
            ItemKind::Caption { .. }
        ) {
            return Err(wrong_kind(
                "Use caption commands to edit word-derived timing.",
            ));
        }
        Ok(slot)
    }
}

fn subtract(a: Frame, b: Frame) -> Result<Frame, CommandError> {
    a.checked_sub(b).ok_or_else(frame_range)
}

fn ensure_length(length: Frame) -> Result<(), CommandError> {
    if length < 3 {
        return Err(error(
            "TOO_SHORT",
            "The result is shorter than three frames.",
            "Choose an interval of at least three frames.",
        ));
    }
    Ok(())
}

fn validate_source(project: &Project, slot: Slot, timing: ClipTiming) -> Result<(), CommandError> {
    if timing.source < 0 {
        return Err(out_of_source());
    }
    if let Some(id) = project.clips.data(slot).expect("live clip").media {
        let descriptor = media(project, id)?;
        let rate = descriptor.probe.fps / f64::from(project.fps) * timing.speed;
        let end = f64::from(timing.source) + f64::from(timing.length) * rate;
        if end > f64::from(descriptor.probe.frames) + 0.000_001 {
            return Err(out_of_source());
        }
    }
    Ok(())
}

fn out_of_source() -> CommandError {
    error(
        "OUT_OF_SOURCE",
        "The edit needs source frames outside the media.",
        "Use a smaller trim or choose a longer source.",
    )
}

pub(super) fn source_for_interval(
    project: &Project,
    slot: Slot,
    at: Frame,
    length: Frame,
) -> Result<Frame, CommandError> {
    let exact = source_value(project, slot, at)?;
    if exact < 0.0 {
        return Err(out_of_source());
    }
    let mut source = rounded(exact)?;
    if let Some(id) = project.clips.data(slot).expect("live clip").media {
        let frames = media(project, id)?.probe.frames;
        let limit = f64::from(frames) - f64::from(length) * source_rate(project, slot)?;
        if exact > limit + 0.000_001 {
            return Err(out_of_source());
        }
        // Preserve a valid interval when its source start rounds past the end.
        if f64::from(source) > limit + 0.000_001 {
            source = rounded((limit + 0.000_001).floor())?;
        }
    }
    Ok(source)
}

fn sort_markers(markers: &mut [Marker], order: &mut Vec<(Frame, usize)>) {
    if markers.is_sorted_by_key(|marker| marker.at) {
        return;
    }
    order.clear();
    order.extend(
        markers
            .iter()
            .enumerate()
            .map(|(index, marker)| (marker.at, index)),
    );
    order.sort_unstable();
    // Follow each cycle in the permutation. The original index breaks frame ties.
    for start in 0..order.len() {
        let mut destination = start;
        loop {
            let source = order[destination].1;
            order[destination].1 = destination;
            if source == start {
                break;
            }
            markers.swap(destination, source);
            destination = source;
        }
    }
}
