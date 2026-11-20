use super::time::{anchor_at, clip_slot, item_slot, media, rounded, source_rate};
use super::*;
use crate::commands::{AnchorArg, Connected, FrameArg, OverlayContent};

impl Editor {
    pub(super) fn run(
        &mut self,
        op: Op,
        _cmd: &str,
        load: &mut dyn FnMut(&str) -> Result<Media, CommandError>,
    ) -> Result<Outcome, CommandError> {
        match op {
            Op::MediaRelink {
                media,
                path,
                frames,
                force,
            } => self.relink(&media, path, frames, force, load),
            Op::ItemSplit { item, at } => {
                let item = self.timeline(&item)?;
                let at = self.frame(&at)?;
                self.split(item, at)
            }
            Op::ItemDelete {
                items,
                ripple,
                keep_overlays,
                connected,
            } => self.delete(&items, ripple, keep_overlays, connected),
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
                look,
            } => {
                let outcome = self.add_item(
                    OverlayContent::Title(text),
                    Some(&at),
                    None,
                    lane.as_ref(),
                    Some(length),
                )?;
                self.title_look(outcome, look.as_deref())
            }
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
            Op::HtmlAdd {
                movie,
                at,
                length,
                placement,
                lane,
            } => {
                let media = Target::Media(self.import_path(movie, load)?);
                let content = OverlayContent::Media {
                    media,
                    source: 0,
                    placement,
                    behind_presenter: false,
                    muted: true,
                    crop: None,
                    fit: splice_model::Fit::Fill,
                };
                self.add_item(content, Some(&at), None, lane.as_ref(), Some(length))
            }
            Op::ClipInsert {
                source,
                at,
                ripple,
                insert,
                lane,
            } => self.insert_clip(source, at.as_ref(), ripple, insert, lane.as_ref(), load),
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
            Op::TitleSet { item, text, look } => self.edit_title(&item, text, look.as_deref()),
            Op::MarkerSet { marker, name, at } => self.set_marker(&marker, name, at.as_ref()),
            Op::ProjectSet { aspect, access } => self.set_project(aspect, access),
            op => self.run_tier_b(op),
        }
    }

    fn split(&mut self, id: TimelineId, at: Frame) -> Result<Outcome, CommandError> {
        let span = time::span(&self.project, id)?;
        if matches!(id, TimelineId::Marker(_)) {
            return Err(wrong_kind("Split a clip or overlay."));
        }
        check_split(span.start, span.end, at)?;
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

    pub(super) fn split_clip(
        &mut self,
        id: ClipId,
        at: Frame,
        cut: Frame,
    ) -> Result<ClipId, CommandError> {
        let slot = clip_slot(&self.project, id)?;
        let old = self.project.clips.timing(slot).expect("live clip");
        let mut data = self.project.clips.data(slot).expect("live clip").clone();
        let length = old.length - cut;
        // Join seams stay on the content they mark (M4).
        data.joins.retain(|&seam| seam > cut);
        data.joins.iter_mut().for_each(|seam| *seam -= cut);
        self.project
            .clips
            .data_mut(slot)
            .expect("live clip")
            .joins
            .retain(|&seam| seam < cut);
        let source = super::spine::source_for_interval(&self.project, slot, at, length)?;
        let right = ClipId::from_serial(self.mint("clp")?);
        data.appearance.fade_in = 0;
        // The right piece starts `cut` frames into the content: its keys follow, and the transition
        // into the original clip stays on the left piece.
        shift_keys(&mut data.appearance.effects, cut);
        data.appearance
            .effects
            .retain(|effect| effect.kind != "transition");
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
        self.splits.push((id, right));
        self.project.clips.sort_by_start();
        for index in 0..self.project.items.order().len() {
            let item = self.project.items.order()[index];
            let anchor = self.project.items.timing(item).expect("live item").anchor;
            match anchor {
                Anchor::ClipSource {
                    clip,
                    source: frame,
                } if clip == id && frame >= source => self.reanchor(
                    item,
                    Anchor::ClipSource {
                        clip: right,
                        source: frame,
                    },
                ),
                // The clip's end is now the right piece's end (M6).
                Anchor::Item {
                    item: TimelineId::Clip(clip),
                    edge: Edge::End,
                    offset,
                } if clip == id => self.reanchor(
                    item,
                    Anchor::Item {
                        item: TimelineId::Clip(right),
                        edge: Edge::End,
                        offset,
                    },
                ),
                _ => {}
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
        shift_keys(&mut data.appearance.effects, cut);
        if let ItemKind::Media(overlay) = &mut data.kind {
            // The source is on the media clock (M2).
            let skipped = time::media_frames(&self.project, overlay.media, cut)?;
            overlay.source = frame_add(overlay.source, skipped)?;
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

    pub(super) fn add_item(
        &mut self,
        content: OverlayContent,
        at: Option<&FrameArg>,
        anchor: Option<&AnchorArg>,
        lane: Option<&Target>,
        length: Option<Frame>,
    ) -> Result<Outcome, CommandError> {
        let (kind, default_length, mut appearance) = match content {
            // Titles cut in (T36.2): a fade is an explicit fadeIn/fadeOut, never a surprise.
            OverlayContent::Title(text) => (
                ItemKind::Title {
                    text: text.unwrap_or_else(|| "Your next move".into()),
                },
                90,
                Appearance::default(),
            ),
            OverlayContent::Zoom(zoom) => (ItemKind::Zoom(zoom), 90, Appearance::default()),
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
            OverlayContent::Shape { shape, rect } => {
                (ItemKind::Shape { shape, rect }, 90, Appearance::default())
            }
        };
        let length = length.unwrap_or(default_length);
        if let ItemKind::Media(overlay) = kind {
            super::spine::overlay_fits(&self.project, overlay, length)?;
        }
        if length < MIN_LENGTH {
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
        self.check_marker_frame(at)?;
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

    /// A new marker frame lies on the timeline (at most the spine end) and is free.
    pub(super) fn check_marker_frame(&self, at: Frame) -> Result<(), CommandError> {
        if at < 0 {
            return Err(CommandError::bad("at", "a nonnegative frame"));
        }
        let end = self.spine_end()?;
        if !self.project.clips.order().is_empty() && at > end {
            return Err(CommandError::rule(
                "PAST_END",
                "at",
                Some(Json::Signed(i64::from(at))),
                &format!("a frame at or before the spine end, frame {end}"),
                "The timeline ends with its last clip; mark a frame on it.",
            ));
        }
        if let Some(existing) = self.project.markers.iter().find(|m| m.at == at) {
            return Err(error(
                "MARKER_EXISTS",
                &format!(
                    "{} \"{}\" is already at frame {at}.",
                    existing.id, existing.name
                ),
                &format!(
                    "Rename it with marker.set {{marker:\"{}\", name}}, or add the marker at another frame.",
                    existing.id
                ),
            ));
        }
        Ok(())
    }

    pub(super) fn delete(
        &mut self,
        selection: &[Target],
        ripple: bool,
        keep: bool,
        connected: Option<Connected>,
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
            self.delete_clip(id, ripple, keep, connected)?;
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

    fn delete_clip(
        &mut self,
        id: ClipId,
        ripple: bool,
        keep: bool,
        connected: Option<Connected>,
    ) -> Result<(), CommandError> {
        let slot = clip_slot(&self.project, id)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let rate = source_rate(&self.project, slot)?;
        let kids = if keep {
            Vec::new()
        } else {
            self.children(&[TimelineId::Clip(id)])?
        };
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
        let end = timing.end().ok_or_else(frame_range)?;
        if ripple {
            self.shift_after(end, -timing.length, None)?;
        }
        // A deleted clip takes its items unless they re-parent (D21).
        self.settle_connected(&kids, &[], connected, timing.start, end, ripple)
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
                            cause: Cause::Cascade,
                        });
                    }
                });
            index += 1;
        }
    }

    /// Moves an item's anchor so it follows its content.
    pub(super) fn reanchor(&mut self, slot: Slot, to: Anchor) {
        self.reanchor_because(slot, to, Cause::Follow);
    }

    pub(super) fn reanchor_because(&mut self, slot: Slot, to: Anchor, cause: Cause) {
        let timing = self.project.items.timing_mut(slot).expect("live item");
        let from = timing.anchor;
        if from != to {
            timing.anchor = to;
            let id = self.project.items.id(slot).expect("live item");
            self.report.side_effects.push(SideEffect::Reanchored {
                id,
                from,
                to,
                cause,
            });
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
        // Markers are timeline frames: they ripple with the spine. A removal (delta < 0) folds
        // the markers inside the removed span onto its start, so none is left past the end.
        for marker in &mut self.project.markers {
            if marker.at >= at {
                marker.at = frame_add(marker.at, delta)?.max(0);
            } else if delta < 0 && marker.at > at + delta {
                marker.at = (at + delta).max(0);
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

/// Both pieces of a split keep at least `MIN_LENGTH` frames (M1).
pub(super) fn check_split(start: Frame, end: Frame, at: Frame) -> Result<(), CommandError> {
    if i64::from(at) - i64::from(start) < i64::from(MIN_LENGTH)
        || i64::from(end) - i64::from(at) < i64::from(MIN_LENGTH)
    {
        return Err(CommandError::rule(
            "TOO_SHORT",
            "at",
            Some(Json::Signed(i64::from(at))),
            &format!(
                "a frame at least {MIN_LENGTH} frames inside {start}–{end}, so both pieces keep {MIN_LENGTH} frames"
            ),
            &format!(
                "Split between frames {} and {}.",
                start + MIN_LENGTH,
                end - MIN_LENGTH
            ),
        ));
    }
    Ok(())
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

/// Stretches owner-relative keys by `factor` when the owner's clock does (a speed change), keeping
/// them strictly increasing.
pub(super) fn scale_keys(effects: &mut [splice_model::Effect], factor: f64) {
    for param in effects
        .iter_mut()
        .flat_map(|effect| effect.params.iter_mut())
    {
        if let splice_model::EffectValue::Keys(keys) = &mut param.value {
            let mut previous = Frame::MIN;
            for key in keys {
                let at = (f64::from(key.at) * factor).round() as Frame;
                key.at = at.max(previous.saturating_add(1));
                previous = key.at;
            }
        }
    }
}

/// Keeps owner-relative keys on the same content when the owner's start moves `by` frames into it
/// (a split's right piece, an in-edge trim, a roll).
pub(super) fn shift_keys(effects: &mut [splice_model::Effect], by: Frame) {
    for param in effects
        .iter_mut()
        .flat_map(|effect| effect.params.iter_mut())
    {
        if let splice_model::EffectValue::Keys(keys) = &mut param.value {
            for key in keys {
                key.at -= by;
            }
        }
    }
}
