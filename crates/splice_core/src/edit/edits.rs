use super::time::{anchor_at, clip_slot, item_slot, media, rounded, source_rate};
use super::*;
use crate::commands::{AnchorArg, FrameArg, OverlayContent};

impl Editor {
    pub(super) fn run(
        &mut self,
        op: Op,
        cmd: &str,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<Outcome, CommandError> {
        match op {
            Op::ItemSplit { item, at } => {
                let item = self.timeline(&item)?;
                let at = self.frame(&at)?;
                self.split(item, at)
            }
            Op::ItemDelete {
                items,
                ripple,
                keep_overlays,
            } => self.delete(&items, ripple, keep_overlays),
            Op::OverlayAdd {
                content,
                at,
                anchor,
                lane,
                length,
            } => self.add_item(content, at.as_ref(), anchor.as_ref(), lane.as_ref(), length),
            Op::TitleAdd {
                at,
                length,
                text,
                lane,
            } => self.add_item(
                OverlayContent::Title(text),
                Some(&at),
                None,
                lane.as_ref(),
                Some(length),
            ),
            Op::LaneCreate { kind, name } => {
                let lane = self.new_lane(kind, name)?;
                Ok(Outcome::create(
                    format!("Created lane {lane}"),
                    Target::Lane(lane),
                ))
            }
            Op::MarkerAdd { at, name } => {
                let at = self.frame(&at)?;
                self.add_marker(at, name)
            }
            Op::ItemTrim {
                item,
                edge,
                position,
                ripple,
            } => self.trim(&item, edge, &position, ripple),
            Op::ItemMove {
                items,
                position,
                carry_overlays,
                ripple,
            } => {
                let result = self.move_items(&items, &position, carry_overlays, ripple);
                self.decode_scratch.targets = items;
                result
            }
            Op::ItemSet {
                items,
                name,
                color,
                fade_in,
                fade_out,
            } => {
                let result = self.set_items(&items, &name, &color, fade_in, fade_out);
                self.decode_scratch.targets = items;
                if let Some(name) = name.flatten() {
                    self.decode_scratch.name = name;
                }
                if let Some(color) = color.flatten() {
                    self.decode_scratch.color = color;
                }
                result
            }
            Op::ItemDuplicate { items, at } => self.duplicate(&items, at.as_ref()),
            Op::ClipSpeed {
                items,
                speed,
                ripple,
            } => self.speed(&items, speed, ripple),
            Op::ClipInsert { source, at, ripple } => {
                self.insert_clip(source, at.as_ref(), ripple, load)
            }
            Op::LaneDelete { lane } => self.delete_lane(&lane),
            Op::LaneRename { lane, name } => self.rename_lane(&lane, name),
            Op::ItemLane { items, destination } => self.move_to_lane(&items, &destination),
            Op::ItemAnchor { item, mode } => self.set_anchor(&item, &mode),
            Op::OverlaySet {
                item,
                placement,
                behind_presenter,
                muted,
            } => self.set_overlay(&item, placement, behind_presenter, muted),
            Op::TitleSet { item, text } => self.set_title(&item, text),
            Op::MarkerSet { marker, name, at } => self.set_marker(&marker, name, at.as_ref()),
            Op::ProjectSet { aspect, access } => self.set_project(aspect, access),
            _ => Err(error(
                "PLANNED",
                &format!("{cmd} does not have an apply handler yet."),
                "Use an available command from commands.describe.",
            )),
        }
    }

    fn split(&mut self, id: TimelineId, at: Frame) -> Result<Outcome, CommandError> {
        let span = time::span(&self.project, id)?;
        if matches!(id, TimelineId::Marker(_)) {
            return Err(wrong_kind("Split a clip or overlay."));
        }
        if i64::from(at) - i64::from(span.start) < 2 || i64::from(span.end) - i64::from(at) < 2 {
            return Err(CommandError::bad(
                "at",
                "a frame at least two frames inside the item",
            ));
        }
        let cut = at - span.start;
        let created = match id {
            TimelineId::Clip(id) => TimelineId::Clip(self.split_clip(id, at, cut)?),
            TimelineId::Item(id) => TimelineId::Item(self.split_item(id, at, cut)?),
            TimelineId::Marker(_) => return Err(wrong_kind("Split a clip or overlay.")),
        };
        Ok(Outcome {
            summary: Summary::Text(format!("Split {id} at frame {at}")),
            primary: Some(Target::Timeline(id)),
            created: vec![Target::Timeline(created)],
        })
    }

    fn split_clip(&mut self, id: ClipId, at: Frame, cut: Frame) -> Result<ClipId, CommandError> {
        let slot = clip_slot(&self.project, id)?;
        let old = self.project.clips.timing(slot).expect("live clip");
        let mut data = self.project.clips.data(slot).expect("live clip").clone();
        let length = old.length - cut;
        let source = super::spine::source_for_interval(&self.project, slot, at, length)?;
        let right = ClipId::from_serial(self.mint("clp")?);
        data.appearance.fade_in = 0;
        clamp_fades(&mut data.appearance, length);
        self.project
            .clips
            .timing_mut(slot)
            .expect("live clip")
            .length = cut;
        let appearance = &mut self
            .project
            .clips
            .data_mut(slot)
            .expect("live clip")
            .appearance;
        appearance.fade_out = 0;
        clamp_fades(appearance, cut);
        self.project
            .clips
            .insert(Clip {
                id: right,
                timing: ClipTiming {
                    start: at,
                    length,
                    source,
                    speed: old.speed,
                },
                data,
            })
            .ok_or_else(arena_full)?;
        self.project.clips.sort_by_start();
        for index in 0..self.project.items.order().len() {
            let item = self.project.items.order()[index];
            let anchor = self.project.items.timing(item).expect("live item").anchor;
            if let Anchor::ClipSource {
                clip,
                source: frame,
            } = anchor
                && clip == id
                && frame >= source
            {
                self.reanchor(
                    item,
                    Anchor::ClipSource {
                        clip: right,
                        source: frame,
                    },
                );
            }
        }
        Ok(right)
    }

    fn split_item(&mut self, id: ItemId, at: Frame, cut: Frame) -> Result<ItemId, CommandError> {
        let slot = item_slot(&self.project, id)?;
        let timing = self.project.items.timing(slot).expect("live item");
        let mut data = self.project.items.data(slot).expect("live item").clone();
        if matches!(data.kind, ItemKind::Caption { .. }) {
            return Err(wrong_kind("Use caption.split to regroup caption words."));
        }
        let anchor = if matches!(timing.anchor, Anchor::Pinned(_)) {
            Anchor::Pinned(at)
        } else {
            anchor_at(&self.project, at)?
        };
        let length = timing.length - cut;
        self.project
            .items
            .timing_mut(slot)
            .expect("live item")
            .length = cut;
        let original = self.project.items.data_mut(slot).expect("live item");
        original.appearance.fade_out = 0;
        clamp_fades(&mut original.appearance, cut);
        data.appearance.fade_in = 0;
        clamp_fades(&mut data.appearance, length);
        match (&mut original.kind, &mut data.kind) {
            (ItemKind::Motion { from, to }, ItemKind::Motion { from: right, .. }) => {
                let midpoint = *from + (*to - *from) * f64::from(cut) / f64::from(timing.length);
                *to = midpoint;
                *right = midpoint;
            }
            (_, ItemKind::Media(overlay)) => overlay.source = frame_add(overlay.source, cut)?,
            _ => {}
        }
        let right = ItemId::from_serial(self.mint("ovl")?);
        self.project
            .items
            .insert(Item {
                id: right,
                timing: ItemTiming { anchor, length },
                data,
            })
            .ok_or_else(arena_full)?;
        Ok(right)
    }

    fn add_item(
        &mut self,
        content: OverlayContent,
        at: Option<&FrameArg>,
        anchor: Option<&AnchorArg>,
        lane: Option<&Target>,
        length: Option<Frame>,
    ) -> Result<Outcome, CommandError> {
        let (kind, default_length, mut appearance) = match content {
            OverlayContent::Title(text) => (
                ItemKind::Title {
                    text: text.unwrap_or_else(|| "Your next move".into()),
                },
                90,
                Appearance {
                    fade_in: 6,
                    fade_out: 6,
                    ..Appearance::default()
                },
            ),
            OverlayContent::Motion { from, to } => {
                (ItemKind::Motion { from, to }, 120, Appearance::default())
            }
            OverlayContent::Media {
                media: id,
                source,
                placement,
                behind_presenter,
                muted,
                crop,
                fit,
            } => {
                let Target::Media(id) = self.resolve(&id)? else {
                    return Err(wrong_kind("Use a media ID from project.info."));
                };
                let descriptor = media(&self.project, id)?;
                if source < 0 || source >= descriptor.probe.frames {
                    return Err(error(
                        "OUT_OF_SOURCE",
                        "The overlay starts outside its media.",
                        "Use a source frame within the media duration.",
                    ));
                }
                let remaining = f64::from(descriptor.probe.frames - source)
                    * f64::from(self.project.fps)
                    / descriptor.probe.fps;
                (
                    ItemKind::Media(Overlay {
                        media: id,
                        source,
                        placement,
                        behind_presenter,
                        muted,
                        framing: Framing { crop, fit },
                    }),
                    remaining.floor().min(90.0) as Frame,
                    Appearance::default(),
                )
            }
        };
        let length = length.unwrap_or(default_length);
        if length < 3 {
            return Err(error(
                "TOO_SHORT",
                "An overlay needs at least three frames.",
                "Increase the length to at least three frames.",
            ));
        }
        let anchor = match (anchor, at) {
            (Some(explicit @ (AnchorArg::Pinned(_) | AnchorArg::ClipSource { .. })), _) => {
                self.explicit_anchor(explicit, 0)?
            }
            (_, Some(at)) => anchor_at(&self.project, self.frame(at)?)?,
            (Some(explicit), None) => self.explicit_anchor(explicit, 0)?,
            (None, None) => anchor_at(&self.project, 0)?,
        };
        if time::anchor_start(&self.project, anchor)? < 0 {
            return Err(CommandError::bad(
                "anchor",
                "a position at or after frame zero",
            ));
        }
        let lane_kind = kind.lane_kind();
        let lane_id = if let Some(lane) = lane {
            let Target::Lane(lane_id) = self.resolve(lane)? else {
                return Err(wrong_kind("Pass a lane ID or omit lane."));
            };
            let existing = self
                .project
                .lanes
                .iter()
                .find(|lane| lane.id == lane_id)
                .ok_or_else(|| not_found(&lane_id, "Call project.info for lane IDs."))?;
            if existing.kind != lane_kind {
                return Err(wrong_kind("Use a lane of the overlay's kind or omit lane."));
            }
            lane_id
        } else {
            self.default_lane(lane_kind)?
        };
        let id = ItemId::from_serial(self.mint("ovl")?);
        clamp_fades(&mut appearance, length);
        self.project
            .items
            .insert(Item {
                id,
                timing: ItemTiming { anchor, length },
                data: ItemData {
                    lane: lane_id,
                    kind,
                    appearance,
                },
            })
            .ok_or_else(arena_full)?;
        if lane.is_some() {
            self.explicit_lanes.push(id);
        }
        Ok(Outcome::create(
            format!("Added {id} on {lane_id}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    fn add_marker(&mut self, at: Frame, name: Option<String>) -> Result<Outcome, CommandError> {
        if at < 0 || self.project.markers.iter().any(|m| m.at == at) {
            return Err(CommandError::bad(
                "at",
                "a nonnegative frame without an existing marker",
            ));
        }
        let id = MarkerId::from_serial(self.mint("mrk")?);
        let name = name
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("Marker {}", self.project.markers.len() + 1));
        self.project.markers.push(Marker {
            id,
            at,
            name,
            color: None,
        });
        self.project.markers.sort_by_key(|marker| marker.at);
        Ok(Outcome::create(
            format!("Added marker {id} at frame {at}"),
            Target::Timeline(TimelineId::Marker(id)),
        ))
    }

    fn delete(
        &mut self,
        selection: &[Target],
        ripple: bool,
        keep: bool,
    ) -> Result<Outcome, CommandError> {
        let ids = self.selection(selection)?;
        let mut clips: Vec<_> = ids
            .iter()
            .filter_map(|id| match id {
                TimelineId::Clip(id) => {
                    let slot = self.project.clips.find(*id).expect("checked clip");
                    Some((
                        *id,
                        self.project.clips.timing(slot).expect("live clip").start,
                    ))
                }
                _ => None,
            })
            .collect();
        clips.sort_by_key(|&(_, start)| std::cmp::Reverse(start));
        for (id, _) in clips {
            self.delete_clip(id, ripple, keep)?;
        }
        for id in &ids {
            match *id {
                TimelineId::Clip(_) => {}
                TimelineId::Item(item) => {
                    if let Some(slot) = self.project.items.find(item) {
                        self.project.items.remove(slot);
                        self.cascade(*id);
                    }
                }
                TimelineId::Marker(marker) => {
                    self.project.markers.retain(|m| m.id != marker);
                    self.cascade(*id);
                }
            }
        }
        let primary = ids.first().copied().map(Target::Timeline);
        Ok(Outcome {
            summary: Summary::Text(format!("Deleted {} timeline item(s)", ids.len())),
            primary,
            created: Vec::new(),
        })
    }

    fn delete_clip(&mut self, id: ClipId, ripple: bool, keep: bool) -> Result<(), CommandError> {
        let slot = clip_slot(&self.project, id)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let rate = source_rate(&self.project, slot)?;
        self.project.clips.remove(slot);
        if keep {
            let gap = ClipId::from_serial(self.mint("clp")?);
            self.project
                .clips
                .insert(Clip {
                    id: gap,
                    timing: ClipTiming {
                        source: 0,
                        speed: 1.0,
                        ..timing
                    },
                    data: ClipData::default(),
                })
                .ok_or_else(arena_full)?;
            for index in 0..self.project.items.order().len() {
                let slot = self.project.items.order()[index];
                let anchor = self.project.items.timing(slot).expect("live item").anchor;
                let next = match anchor {
                    Anchor::ClipSource { clip, source } if clip == id => Anchor::ClipSource {
                        clip: gap,
                        source: rounded((f64::from(source) - f64::from(timing.source)) / rate)?
                            .max(0),
                    },
                    Anchor::Item {
                        item: TimelineId::Clip(clip),
                        edge,
                        offset,
                    } if clip == id => Anchor::Item {
                        item: TimelineId::Clip(gap),
                        edge,
                        offset,
                    },
                    _ => continue,
                };
                self.reanchor(slot, next);
            }
            self.project.clips.sort_by_start();
            return Ok(());
        }
        self.cascade(TimelineId::Clip(id));
        if ripple {
            self.shift_after(timing.end().ok_or_else(frame_range)?, -timing.length, None)?;
        }
        Ok(())
    }

    pub(super) fn cascade(&mut self, root: TimelineId) {
        let mut removed = vec![root];
        let mut index = 0;
        while let Some(&parent) = removed.get(index) {
            self.project
                .items
                .order()
                .to_vec()
                .into_iter()
                .for_each(|slot| {
                    let timing = self.project.items.timing(slot).expect("live item");
                    if anchor_parent(timing.anchor) == Some(parent) {
                        let id = self.project.items.id(slot).expect("live item");
                        self.project.items.remove(slot);
                        removed.push(TimelineId::Item(id));
                        self.report.side_effects.push(SideEffect::Removed {
                            id,
                            cascade_from: Some(parent),
                        });
                    }
                });
            index += 1;
        }
    }

    pub(super) fn reanchor(&mut self, slot: Slot, to: Anchor) {
        let timing = self.project.items.timing_mut(slot).expect("live item");
        let from = timing.anchor;
        if from != to {
            timing.anchor = to;
            let id = self.project.items.id(slot).expect("live item");
            self.report
                .side_effects
                .push(SideEffect::Reanchored { id, from, to });
        }
    }

    pub(super) fn shift_after(
        &mut self,
        at: Frame,
        delta: Frame,
        except: Option<Slot>,
    ) -> Result<(), CommandError> {
        for index in 0..self.project.clips.order().len() {
            let slot = self.project.clips.order()[index];
            let timing = self.project.clips.timing_mut(slot).expect("live clip");
            if timing.start >= at && Some(slot) != except {
                timing.start = frame_add(timing.start, delta)?;
            }
        }
        Ok(())
    }
}

pub(super) fn anchor_parent(anchor: Anchor) -> Option<TimelineId> {
    match anchor {
        Anchor::ClipSource { clip, .. } => Some(TimelineId::Clip(clip)),
        Anchor::Item { item, .. } => Some(item),
        Anchor::Marker { marker, .. } => Some(TimelineId::Marker(marker)),
        Anchor::Pinned(_) | Anchor::Word { .. } => None,
    }
}

pub(super) fn clamp_fades(appearance: &mut Appearance, length: Frame) {
    appearance.fade_in = appearance.fade_in.clamp(0, length);
    appearance.fade_out = appearance.fade_out.clamp(0, length - appearance.fade_in);
}

pub(super) fn arena_full() -> CommandError {
    error(
        "OUT_OF_RANGE",
        "The timeline has exhausted its arena slots.",
        "Use a smaller project.",
    )
}
