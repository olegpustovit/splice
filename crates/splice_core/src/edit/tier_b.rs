//! Tier B commands (0.2): framing, take components, effects, cutting, lift/replace and captions.
//! Behavior follows `docs/reference/core.js`.
use super::settle::{Owner, effects, effects_vec_mut};
use super::time::{anchor_at, clip_slot, item_slot, media, rounded};
use super::*;
use crate::commands::{
    CaptionWordPatch, Component, EffectSelection, FrameArg, OverlayContent, effect_schemas,
};

/// Effect types that are designed but not built (core.js PLANNED_EFFECTS).
const PLANNED_EFFECTS: &[&str] = &[
    "motionBlur",
    "blurRegion",
    "spotlight",
    "deviceFrame",
    "backgroundBlur",
    "clickSound",
    "grade",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Applies {
    Visual,
    Take,
    Caption,
}

fn applies(kind: &str) -> Result<Applies, CommandError> {
    match kind {
        "window" => Ok(Applies::Visual),
        "cursor" | "clickZoom" | "camera" | "keystrokes" => Ok(Applies::Take),
        "captions" => Ok(Applies::Caption),
        _ => Err(error(
            if PLANNED_EFFECTS.contains(&kind) {
                "PLANNED"
            } else {
                "BAD_ARGS"
            },
            &format!("No effect type {kind}."),
            "Types: window, cursor, clickZoom, camera, captions, keystrokes. commands.describe {id:\"effect.add\"} lists parameters.",
        )),
    }
}

fn check_params(kind: &str, params: &[EffectParam]) -> Result<(), CommandError> {
    let schemas = effect_schemas();
    let schema = schemas
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .expect("checked effect type");
    for param in params {
        let names = || {
            schema
                .as_object()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        let spec = schema.field(&param.name).ok_or_else(|| {
            error(
                "BAD_ARGS",
                &format!("{kind} has no parameter {}.", param.name),
                &format!("Parameters: {}.", names()),
            )
        })?;
        let number = |key| spec.field(key).and_then(Json::as_f64).unwrap_or(0.0);
        let ok = match (spec.field("type").and_then(Json::as_str), &param.value) {
            (Some("number"), EffectValue::Number(n)) => *n >= number("min") && *n <= number("max"),
            (Some("bool"), EffectValue::Bool(_)) => true,
            (Some("enum"), EffectValue::Text(text)) => spec
                .field("options")
                .and_then(Json::as_array)
                .is_some_and(|options| options.iter().any(|o| o.as_str() == Some(text))),
            (Some("color"), EffectValue::Text(text)) => {
                text.len() == 7
                    && text.starts_with('#')
                    && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
            }
            _ => false,
        };
        if !ok {
            let expected = match spec.field("type").and_then(Json::as_str) {
                Some("number") => format!("a number from {} to {}", number("min"), number("max")),
                Some("bool") => "true or false".into(),
                Some("color") => "a #rrggbb color".into(),
                _ => {
                    let mut options = String::new();
                    splice_json::write(spec.field("options").unwrap_or(&Json::Null), &mut options)
                        .expect("finite schema");
                    format!("one of {options}")
                }
            };
            return Err(CommandError::bad(&param.name, &expected));
        }
    }
    Ok(())
}

fn merge(into: &mut Vec<EffectParam>, params: Vec<EffectParam>) {
    for param in params {
        match into.iter_mut().find(|old| old.name == param.name) {
            Some(old) => old.value = param.value,
            None => into.push(param),
        }
    }
}

impl Editor {
    pub(super) fn run_tier_b(&mut self, op: Op) -> Result<Outcome, CommandError> {
        match op {
            Op::ClipCrop {
                items,
                rect,
                fit,
                aspect,
                focus,
            } => match aspect {
                Some(ratio) => self.reframe(&items, ratio, focus),
                None => self.crop(&items, rect, fit),
            },
            Op::ComponentSet {
                clip,
                component,
                enabled,
            } => self.set_component(&clip, component, enabled),
            Op::AudioSet {
                clip,
                items,
                volume,
                duck,
                normalize,
            } => self.set_volume(clip, &items, volume, duck, normalize),
            Op::CursorSet {
                clip,
                size,
                click_rings,
            } => {
                let mut params = Vec::new();
                if let Some(size) = size {
                    params.push(number("size", size.clamp(0.5, 4.0)));
                }
                if let Some(rings) = click_rings {
                    params.push(EffectParam {
                        name: "clickRings".into(),
                        value: EffectValue::Bool(rings),
                    });
                }
                self.clip_effect(&clip, "cursor", params, "Cursor")
            }
            Op::KeysSet { clip, size } => {
                let size =
                    size.ok_or_else(|| CommandError::bad("size", "a scale from 0.5 to 3"))?;
                let params = vec![number("size", size.clamp(0.5, 3.0))];
                self.clip_effect(&clip, "keystrokes", params, "Keystroke chips")
            }
            Op::KeysHide {
                clip,
                events,
                hidden,
            } => self.hide_keys(&clip, &events, hidden),
            Op::MotionAdd {
                at,
                length,
                from,
                to,
                focus,
                ease,
            } => {
                check_scale(from, "from")?;
                check_scale(to, "to")?;
                self.add_item(
                    OverlayContent::Motion {
                        from,
                        to,
                        focus,
                        ease,
                    },
                    Some(&at),
                    None,
                    None,
                    Some(length),
                )
            }
            Op::MotionSet {
                item,
                from,
                to,
                focus,
                ease,
            } => self.set_motion(&item, from, to, focus, ease),
            Op::EffectAdd {
                target,
                kind,
                params,
                enabled,
            } => {
                let owner = self.owner(&target)?;
                let mut created = Vec::new();
                let id = self.upsert(owner, &kind, params, enabled, &mut created)?;
                Ok(Outcome {
                    summary: Summary::Text(format!(
                        "{} {kind} on {}",
                        if enabled == Some(false) {
                            "Turned off"
                        } else {
                            "Added"
                        },
                        self.owner_label(owner)
                    )),
                    primary: Some(Target::Effect(id)),
                    created,
                })
            }
            Op::EffectSet {
                target,
                params,
                enabled,
                reset,
            } => self.set_effect(target, params, enabled, &reset),
            Op::EffectRemove {
                effects,
                target,
                kind,
            } => self.remove_effects(&effects, target.as_ref(), kind.as_deref()),
            Op::EffectPaste { from, to, replace } => self.paste_effects(&from, &to, replace),
            Op::EffectPromote {
                effect,
                to_project,
                clear_overrides,
            } => self.promote(&effect, to_project, clear_overrides),
            Op::CutRange { from, to, ripple } => {
                let from = self.frame(&from)?;
                let to = self.frame(&to)?;
                if to <= from {
                    return Err(CommandError::bad("to", "a frame after from"));
                }
                self.cut_range(from, to, ripple)?;
                Ok(Outcome {
                    summary: Summary::Text(format!("Deleted frames {from}–{to}")),
                    primary: None,
                    created: Vec::new(),
                })
            }
            Op::CutWords {
                words,
                transcript_hash,
            } => self.cut_words(&words, transcript_hash),
            Op::ClipLift { items, lane } => self.lift(&items, lane.as_ref()),
            Op::OverlayReplaceSpine { item, target } => self.replace_spine(&item, target.as_ref()),
            Op::CaptionsGenerate {
                clips,
                max_words,
                fillers,
                lane,
                style,
            } => self.generate_captions(clips.as_deref(), max_words, fillers, lane.as_ref(), style),
            Op::CaptionSet { item, text, words } => self.set_caption(&item, text, words),
            Op::CaptionSplit { item, word } => self.split_caption(&item, word),
            Op::CaptionMerge { items } => self.merge_captions(&items),
            _ => unreachable!("tier B dispatch"),
        }
    }

    // ── Framing and take components ──

    fn crop(
        &mut self,
        items: &[Target],
        rect: Option<Option<Rect>>,
        fit: Option<Fit>,
    ) -> Result<Outcome, CommandError> {
        if rect.is_none() && fit.is_none() {
            return Err(error(
                "BAD_ARGS",
                "Pass rect, fit or both.",
                "rect is normalized {x,y,w,h}; null resets to the full frame.",
            ));
        }
        let rect = rect.map(|rect| {
            rect.map(|r| Rect {
                x: round4(r.x),
                y: round4(r.y),
                w: round4(r.w),
                h: round4(r.h),
            })
        });
        let ids = self.selection(items)?;
        for &id in &ids {
            let framing = match id {
                TimelineId::Clip(clip) => {
                    let slot = clip_slot(&self.project, clip)?;
                    let media_id = self.project.clips.data(slot).expect("live clip").media;
                    let croppable = media_id
                        .and_then(|m| media(&self.project, m).ok())
                        .is_some_and(|m| m.kind != MediaKind::Audio);
                    if !croppable {
                        return Err(no_source_frame());
                    }
                    &mut self
                        .project
                        .clips
                        .data_mut(slot)
                        .expect("live clip")
                        .framing
                }
                TimelineId::Item(item) => {
                    let slot = item_slot(&self.project, item)?;
                    match &mut self.project.items.data_mut(slot).expect("live item").kind {
                        ItemKind::Media(overlay) => &mut overlay.framing,
                        _ => return Err(no_source_frame()),
                    }
                }
                TimelineId::Marker(_) => return Err(no_source_frame()),
            };
            if let Some(rect) = rect {
                framing.crop = rect;
            }
            if let Some(fit) = fit {
                framing.fit = fit;
            }
        }
        Ok(Outcome {
            summary: Summary::Update(ids.len()),
            primary: ids.first().copied().map(Target::Timeline),
            created: Vec::new(),
        })
    }

    /// The largest crop of `ratio` (the canvas shape when `None`) around `focus`, filling the frame.
    fn reframe(
        &mut self,
        items: &[Target],
        ratio: Option<f64>,
        focus: Option<[f64; 2]>,
    ) -> Result<Outcome, CommandError> {
        let ratio =
            ratio.unwrap_or(f64::from(self.project.size[0]) / f64::from(self.project.size[1]));
        let [fx, fy] = focus.unwrap_or([0.5, 0.5]);
        let ids = self.selection(items)?;
        for &id in &ids {
            let media_id = match id {
                TimelineId::Clip(clip) => {
                    let slot = clip_slot(&self.project, clip)?;
                    self.project.clips.data(slot).expect("live clip").media
                }
                TimelineId::Item(item) => {
                    let slot = item_slot(&self.project, item)?;
                    match self.project.items.data(slot).expect("live item").kind {
                        ItemKind::Media(overlay) => Some(overlay.media),
                        _ => None,
                    }
                }
                TimelineId::Marker(_) => None,
            };
            let probe = media_id
                .and_then(|m| media(&self.project, m).ok())
                .filter(|m| m.kind != MediaKind::Audio && m.probe.width > 0 && m.probe.height > 0)
                .map(|m| f64::from(m.probe.width) / f64::from(m.probe.height))
                .ok_or_else(no_source_frame)?;
            // Normalized width and height of the largest `ratio` crop inside the source.
            let (w, h) = if probe > ratio {
                (ratio / probe, 1.0)
            } else {
                (1.0, probe / ratio)
            };
            let rect = Rect {
                x: round4((fx - w / 2.0).clamp(0.0, 1.0 - w)),
                y: round4((fy - h / 2.0).clamp(0.0, 1.0 - h)),
                w: round4(w),
                h: round4(h),
            };
            self.crop(&[Target::Timeline(id)], Some(Some(rect)), Some(Fit::Fill))?;
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Reframed {} item(s) to fill the canvas", ids.len())),
            primary: ids.first().copied().map(Target::Timeline),
            created: Vec::new(),
        })
    }

    fn take_clip(&self, target: &Target) -> Result<(ClipId, Slot), CommandError> {
        match self.resolve(target)? {
            Target::Timeline(TimelineId::Clip(id)) => Ok((id, clip_slot(&self.project, id)?)),
            _ => Err(wrong_kind("Pass a spine clip ID (clp_…).")),
        }
    }

    fn set_component(
        &mut self,
        clip: &Target,
        component: Component,
        enabled: bool,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.take_clip(clip)?;
        let states = &mut self
            .project
            .clips
            .data_mut(slot)
            .expect("live clip")
            .enabled;
        let state = component_state(states, component);
        if state.is_none() {
            return Err(no_component(id, component));
        }
        *state = Some(enabled);
        Ok(Outcome::edit(
            format!(
                "{} {} on {id}",
                component_name(component),
                if enabled { "shown" } else { "hidden" }
            ),
            Target::Timeline(TimelineId::Clip(id)),
        ))
    }

    fn set_volume(
        &mut self,
        clip: Option<(Target, Component)>,
        items: &[Target],
        volume: Option<f64>,
        duck: Option<Option<Duck>>,
        normalize: Option<f64>,
    ) -> Result<Outcome, CommandError> {
        if let Some((clip, component)) = clip {
            let (id, slot) = self.take_clip(&clip)?;
            let data = self.project.clips.data_mut(slot).expect("live clip");
            let volume = volume.expect("decoded volume").clamp(0.0, 2.0);
            let gain = match component {
                Component::Mic if data.enabled.mic.is_some() => &mut data.volume.mic,
                Component::System if data.enabled.system.is_some() => &mut data.volume.system,
                _ => return Err(no_component(id, component)),
            };
            *gain = volume;
            return Ok(Outcome::edit(
                format!(
                    "{} volume {}%",
                    component_name(component),
                    (volume * 100.0).round()
                ),
                Target::Timeline(TimelineId::Clip(id)),
            ));
        }
        let ids = self.selection(items)?;
        for &id in &ids {
            let TimelineId::Item(item) = id else {
                return Err(wrong_kind("Pass sound items (ovl_…) from an audio lane."));
            };
            let slot = item_slot(&self.project, item)?;
            let ItemKind::Sound(sound) = self.project.items.data(slot).expect("live item").kind
            else {
                return Err(wrong_kind("Pass sound items (ovl_…) from an audio lane."));
            };
            let mut next = sound;
            if let Some(volume) = volume {
                next.volume = volume.clamp(0.0, 2.0);
            }
            if let Some(target) = normalize {
                let loudness = media(&self.project, sound.media)?
                    .probe
                    .loudness
                    .ok_or_else(|| {
                        error(
                            "LOUDNESS_UNKNOWN",
                            &format!("{} has no measured loudness.", sound.media),
                            "Import the file with clip.insert on macOS, which measures it.",
                        )
                    })?;
                next.volume = 10f64.powf((target - loudness) / 20.0).clamp(0.0, 2.0);
            }
            if let Some(duck) = duck {
                next.duck = duck;
            }
            let ItemKind::Sound(sound) =
                &mut self.project.items.data_mut(slot).expect("live item").kind
            else {
                unreachable!("checked sound")
            };
            *sound = next;
        }
        Ok(Outcome {
            summary: Summary::Update(ids.len()),
            primary: ids.first().copied().map(Target::Timeline),
            created: Vec::new(),
        })
    }

    /// Places an audio file on an audio lane at `at` (default frame 0), pinned to the timeline.
    pub(super) fn add_sound(
        &mut self,
        media_id: MediaId,
        at: Option<&FrameArg>,
        lane: Option<&Target>,
    ) -> Result<Outcome, CommandError> {
        let descriptor = media(&self.project, media_id)?;
        if descriptor.probe.audio.is_empty() {
            return Err(error(
                "WRONG_KIND",
                &format!("{media_id} has no audio."),
                "Place pictures on the spine or an overlay lane.",
            ));
        }
        let length = rounded(
            (f64::from(descriptor.probe.frames) * f64::from(self.project.fps)
                / descriptor.probe.fps)
                .floor(),
        )?;
        if length < 3 {
            return Err(error(
                "TOO_SHORT",
                "The audio is shorter than three frames.",
                "Use a longer file.",
            ));
        }
        let at = at.map(|at| self.frame(at)).transpose()?.unwrap_or(0).max(0);
        let lane_id = match lane {
            Some(lane) => {
                let Target::Lane(id) = self.resolve(lane)? else {
                    return Err(wrong_kind("Pass an audio lane ID or omit lane."));
                };
                let found = self
                    .project
                    .lanes
                    .iter()
                    .find(|l| l.id == id)
                    .ok_or_else(|| not_found(&id, "project.info lists lanes."))?;
                if found.kind != LaneKind::Audio {
                    return Err(wrong_kind("Pass an audio lane or omit lane."));
                }
                id
            }
            None => self.default_lane(LaneKind::Audio)?,
        };
        let id = ItemId::from_serial(self.mint("ovl")?);
        self.project
            .items
            .insert(Item {
                id,
                timing: ItemTiming {
                    anchor: Anchor::Pinned(at),
                    length,
                },
                data: ItemData {
                    lane: lane_id,
                    kind: ItemKind::Sound(Sound {
                        media: media_id,
                        source: 0,
                        volume: 1.0,
                        duck: None,
                    }),
                    appearance: Appearance::default(),
                },
            })
            .ok_or_else(edits::arena_full)?;
        if lane.is_some() {
            self.explicit_lanes.push(id);
        }
        Ok(Outcome::create(
            format!("Added {id} on {lane_id} at frame {at}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    fn clip_effect(
        &mut self,
        clip: &Target,
        kind: &str,
        params: Vec<EffectParam>,
        label: &str,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.take_clip(clip)?;
        let mut created = Vec::new();
        self.upsert(Owner::Clip(slot), kind, params, None, &mut created)?;
        Ok(Outcome {
            summary: Summary::Text(format!("{label} updated on {id}")),
            primary: Some(Target::Timeline(TimelineId::Clip(id))),
            created,
        })
    }

    fn hide_keys(
        &mut self,
        clip: &Target,
        events: &[u32],
        hidden: bool,
    ) -> Result<Outcome, CommandError> {
        let (id, slot) = self.take_clip(clip)?;
        let data = self.project.clips.data_mut(slot).expect("live clip");
        if data.enabled.keys.is_none() {
            return Err(no_component(id, Component::Keys));
        }
        // ponytail: indexes are not checked against events.json (the core never reads take files);
        // an unknown index hides nothing. Check once the core loads take event counts.
        for &event in events {
            match (data.hidden_keys.binary_search(&event), hidden) {
                (Err(at), true) => data.hidden_keys.insert(at, event),
                (Ok(at), false) => {
                    data.hidden_keys.remove(at);
                }
                _ => {}
            }
        }
        Ok(Outcome::edit(
            format!(
                "{} {} keystroke(s)",
                if hidden { "Hid" } else { "Showed" },
                events.len()
            ),
            Target::Timeline(TimelineId::Clip(id)),
        ))
    }

    fn set_motion(
        &mut self,
        item: &Target,
        from: Option<f64>,
        to: Option<f64>,
        focus: Option<Option<[f64; 2]>>,
        ease: Option<bool>,
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Item(id) = self.timeline(item)? else {
            return Err(wrong_kind("Pass a motion overlay (ovl_…)."));
        };
        let slot = item_slot(&self.project, id)?;
        let ItemKind::Motion {
            from: old_from,
            to: old_to,
            focus: old_focus,
            ease: old_ease,
        } = &mut self.project.items.data_mut(slot).expect("live item").kind
        else {
            return Err(wrong_kind("Pass a motion overlay (ovl_…)."));
        };
        if let Some(from) = from {
            check_scale(from, "from")?;
            *old_from = from;
        }
        if let Some(to) = to {
            check_scale(to, "to")?;
            *old_to = to;
        }
        if let Some(focus) = focus {
            *old_focus = focus;
        }
        if let Some(ease) = ease {
            *old_ease = ease;
        }
        Ok(Outcome::edit(
            format!("Motion {id} {old_from}→{old_to}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    // ── Effects ──

    fn owner(&self, target: &Target) -> Result<Owner, CommandError> {
        match self.resolve(target)? {
            Target::Project => Ok(Owner::Project),
            Target::Lane(id) if id == LaneId::SPINE => Ok(Owner::Spine),
            Target::Lane(id) => self
                .project
                .lanes
                .iter()
                .position(|lane| lane.id == id)
                .map(Owner::Lane)
                .ok_or_else(|| not_found(&id, "project.info lists lanes.")),
            Target::Timeline(TimelineId::Clip(id)) => {
                Ok(Owner::Clip(clip_slot(&self.project, id)?))
            }
            Target::Timeline(TimelineId::Item(id)) => {
                Ok(Owner::Item(item_slot(&self.project, id)?))
            }
            _ => Err(wrong_kind(
                "Targets: a clip (clp_), an overlay (ovl_), a lane (lan_…, lan_spine) or \"project\".",
            )),
        }
    }

    fn owner_label(&self, owner: Owner) -> String {
        match owner {
            Owner::Project => "the project".into(),
            Owner::Spine => "lan_spine".into(),
            Owner::Lane(index) => self.project.lanes[index].id.to_string(),
            Owner::Clip(slot) => self.project.clips.id(slot).expect("live clip").to_string(),
            Owner::Item(slot) => self.project.items.id(slot).expect("live item").to_string(),
        }
    }

    fn fits(&self, kind: Applies, owner: Owner) -> bool {
        match owner {
            Owner::Project => true,
            Owner::Spine => kind != Applies::Caption,
            Owner::Lane(index) => {
                (self.project.lanes[index].kind == LaneKind::Captions) == (kind == Applies::Caption)
            }
            Owner::Clip(slot) => {
                let media_id = self.project.clips.data(slot).expect("live clip").media;
                let descriptor = media_id.and_then(|id| media(&self.project, id).ok());
                match kind {
                    Applies::Take => descriptor.is_some_and(|m| m.kind == MediaKind::Take),
                    Applies::Caption => false,
                    Applies::Visual => descriptor.is_some_and(|m| m.kind != MediaKind::Audio),
                }
            }
            Owner::Item(slot) => matches!(
                (
                    kind,
                    &self.project.items.data(slot).expect("live item").kind
                ),
                (Applies::Caption, ItemKind::Caption { .. })
                    | (Applies::Visual, ItemKind::Media(_))
            ),
        }
    }

    /// Writes params into the owner's instance of a type, creating it when missing.
    fn upsert(
        &mut self,
        owner: Owner,
        kind: &str,
        params: Vec<EffectParam>,
        enabled: Option<bool>,
        created: &mut Vec<Target>,
    ) -> Result<EffectId, CommandError> {
        let class = applies(kind)?;
        if !self.fits(class, owner) {
            return Err(wrong_kind(match class {
                Applies::Take => "It needs a screen take with recorded events.",
                Applies::Caption => {
                    "Caption Style applies to a Captions lane, a caption cue or the project."
                }
                Applies::Visual => {
                    "Window applies to spine clips, media overlays, lanes and the project."
                }
            }));
        }
        check_params(kind, &params)?;
        let index = match effects(&self.project, owner)
            .iter()
            .position(|effect| effect.kind == kind)
        {
            Some(index) => index,
            None => {
                let id = EffectId::from_serial(self.mint("fx")?);
                let list = effects_vec_mut(&mut self.project, owner);
                list.push(Effect {
                    id,
                    kind: kind.into(),
                    enabled: None,
                    params: Vec::new(),
                });
                created.push(Target::Effect(id));
                list.len() - 1
            }
        };
        let effect = &mut effects_vec_mut(&mut self.project, owner)[index];
        merge(&mut effect.params, params);
        if enabled.is_some() {
            effect.enabled = enabled;
        }
        Ok(effect.id)
    }

    fn owners(&self) -> Vec<Owner> {
        [Owner::Project, Owner::Spine]
            .into_iter()
            .chain((0..self.project.lanes.len()).map(Owner::Lane))
            .chain(self.project.clips.order().iter().copied().map(Owner::Clip))
            .chain(self.project.items.order().iter().copied().map(Owner::Item))
            .collect()
    }

    fn find_effect(&self, target: &Target) -> Result<(Owner, usize), CommandError> {
        let Target::Effect(id) = self.resolve(target)? else {
            return Err(wrong_kind("Pass an effect ID (fx_…)."));
        };
        self.owners()
            .into_iter()
            .find_map(|owner| {
                effects(&self.project, owner)
                    .iter()
                    .position(|effect| effect.id == id)
                    .map(|index| (owner, index))
            })
            .ok_or_else(|| not_found(&id, "items.inspect lists effects with IDs."))
    }

    fn set_effect(
        &mut self,
        target: EffectSelection,
        params: Vec<EffectParam>,
        enabled: Option<bool>,
        reset: &[String],
    ) -> Result<Outcome, CommandError> {
        let mut created = Vec::new();
        let (owner, index) = match target {
            EffectSelection::Id(id) => {
                let (owner, index) = self.find_effect(&id)?;
                let kind = effects(&self.project, owner)[index].kind.clone();
                check_params(&kind, &params)?;
                let effect = &mut effects_vec_mut(&mut self.project, owner)[index];
                merge(&mut effect.params, params);
                if enabled.is_some() {
                    effect.enabled = enabled;
                }
                (owner, index)
            }
            EffectSelection::Owner { target, kind } => {
                let owner = self.owner(&target)?;
                let id = self.upsert(owner, &kind, params, enabled, &mut created)?;
                let index = effects(&self.project, owner)
                    .iter()
                    .position(|effect| effect.id == id)
                    .expect("upserted effect");
                (owner, index)
            }
        };
        let list = effects_vec_mut(&mut self.project, owner);
        let id = list[index].id;
        let kind = list[index].kind.clone();
        list[index]
            .params
            .retain(|param| !reset.contains(&param.name));
        let removed =
            !reset.is_empty() && list[index].params.is_empty() && list[index].enabled.is_none();
        if removed {
            list.remove(index);
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "{kind} on {}{}",
                self.owner_label(owner),
                if removed { " reset" } else { "" }
            )),
            primary: (!removed).then_some(Target::Effect(id)),
            created,
        })
    }

    fn remove_effects(
        &mut self,
        ids: &[Target],
        target: Option<&Target>,
        kind: Option<&str>,
    ) -> Result<Outcome, CommandError> {
        if ids.is_empty() && target.is_none() {
            return Err(CommandError::bad("effects", "fx_ IDs, or pass target"));
        }
        let mut count = 0;
        for id in ids {
            let (owner, index) = self.find_effect(id)?;
            effects_vec_mut(&mut self.project, owner).remove(index);
            count += 1;
        }
        if let Some(target) = target {
            let owner = self.owner(target)?;
            let list = effects_vec_mut(&mut self.project, owner);
            let before = list.len();
            list.retain(|effect| kind.is_some_and(|kind| effect.kind != kind));
            count += before - list.len();
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Removed {count} effect(s)")),
            primary: None,
            created: Vec::new(),
        })
    }

    fn paste_effects(
        &mut self,
        from: &Target,
        to: &[Target],
        replace: bool,
    ) -> Result<Outcome, CommandError> {
        let source = self.owner(from)?;
        let copied = effects(&self.project, source).to_vec();
        if copied.is_empty() {
            return Err(error(
                "BAD_ARGS",
                &format!("{} has no effects of its own.", self.owner_label(source)),
                "Copy from an item with local effects.",
            ));
        }
        let mut created = Vec::new();
        for target in to {
            let owner = self.owner(target)?;
            if replace {
                effects_vec_mut(&mut self.project, owner).clear();
            }
            for effect in &copied {
                if self.fits(applies(&effect.kind)?, owner) {
                    self.upsert(
                        owner,
                        &effect.kind,
                        effect.params.clone(),
                        effect.enabled,
                        &mut created,
                    )?;
                }
            }
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Pasted {} effect(s) onto {} item(s)",
                copied.len(),
                to.len()
            )),
            primary: None,
            created,
        })
    }

    fn promote(
        &mut self,
        effect: &Target,
        to_project: bool,
        clear: bool,
    ) -> Result<Outcome, CommandError> {
        let (owner, index) = self.find_effect(effect)?;
        let destination = match (to_project, owner) {
            (true, Owner::Project) => None,
            (true, _) => Some(Owner::Project),
            (false, Owner::Clip(_)) => Some(Owner::Spine),
            (false, Owner::Item(slot)) => {
                let lane = self.project.items.data(slot).expect("live item").lane;
                self.project
                    .lanes
                    .iter()
                    .position(|l| l.id == lane)
                    .map(Owner::Lane)
            }
            (false, _) => None,
        }
        .ok_or_else(|| {
            error(
                "BAD_ARGS",
                "Nothing to promote to.",
                "Promote clip effects to their lane, or lane effects to the project.",
            )
        })?;
        let original = effects(&self.project, owner)[index].clone();
        let mut created = Vec::new();
        self.upsert(
            destination,
            &original.kind,
            original.params.clone(),
            original.enabled,
            &mut created,
        )?;
        let list = effects_vec_mut(&mut self.project, owner);
        let at = list
            .iter()
            .position(|e| e.id == original.id)
            .expect("promoted effect");
        list.remove(at);
        if clear {
            let siblings: Vec<Owner> = match destination {
                Owner::Project | Owner::Spine => self
                    .project
                    .clips
                    .order()
                    .iter()
                    .copied()
                    .map(Owner::Clip)
                    .collect(),
                Owner::Lane(index) => {
                    let lane = self.project.lanes[index].id;
                    self.project
                        .items
                        .order()
                        .iter()
                        .copied()
                        .filter(|&slot| self.project.items.data(slot).expect("live").lane == lane)
                        .map(Owner::Item)
                        .collect()
                }
                _ => Vec::new(),
            };
            for sibling in siblings {
                effects_vec_mut(&mut self.project, sibling).retain(|e| e.kind != original.kind);
            }
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "{} now applies to {}",
                original.kind,
                self.owner_label(destination)
            )),
            primary: created.first().cloned(),
            created,
        })
    }

    // ── Cutting ──

    /// Items anchored to a clip, with their current start frames.
    fn children(&self, parents: &[TimelineId]) -> Result<Vec<(Slot, Frame)>, CommandError> {
        let mut kids = Vec::new();
        for &slot in self.project.items.order() {
            let anchor = self.project.items.timing(slot).expect("live item").anchor;
            if edits::anchor_parent(anchor).is_some_and(|parent| parents.contains(&parent)) {
                kids.push((slot, time::item_span(&self.project, slot)?.start));
            }
        }
        Ok(kids)
    }

    fn reanchor_at(&mut self, kids: &[(Slot, Frame)]) -> Result<(), CommandError> {
        for &(slot, at) in kids {
            if self.project.items.id(slot).is_some() {
                let anchor = anchor_at(&self.project, at.max(0))?;
                self.reanchor(slot, anchor);
            }
        }
        Ok(())
    }

    pub(super) fn cut_range(
        &mut self,
        from: Frame,
        to: Frame,
        ripple: bool,
    ) -> Result<(), CommandError> {
        for at in [from, to] {
            let inside = self.project.clips.order().iter().find_map(|&slot| {
                let timing = self.project.clips.timing(slot).expect("live clip");
                (timing.start < at && at < timing.start + timing.length).then(|| {
                    (
                        self.project.clips.id(slot).expect("live clip"),
                        timing.start,
                    )
                })
            });
            if let Some((id, start)) = inside {
                self.split_clip(id, at, at - start)?;
            }
        }
        let inside: Vec<(Slot, ClipId)> = self
            .project
            .clips
            .order()
            .iter()
            .filter_map(|&slot| {
                let timing = self.project.clips.timing(slot).expect("live clip");
                (timing.start >= from && timing.start + timing.length <= to)
                    .then(|| (slot, self.project.clips.id(slot).expect("live clip")))
            })
            .collect();
        let parents: Vec<TimelineId> = inside.iter().map(|&(_, id)| TimelineId::Clip(id)).collect();
        let kids = self.children(&parents)?;
        for &(slot, _) in &inside {
            self.project.clips.remove(slot);
        }
        if ripple {
            self.shift_after(to, from - to, None)?;
        }
        let kids: Vec<(Slot, Frame)> = kids
            .into_iter()
            .map(|(slot, at)| {
                (
                    slot,
                    if at >= to && ripple {
                        at - (to - from)
                    } else {
                        at.min(from)
                    },
                )
            })
            .collect();
        self.reanchor_at(&kids)
    }

    fn cut_words(&mut self, words: &[WordId], hash: Option<u64>) -> Result<Outcome, CommandError> {
        let mut ranges = Vec::with_capacity(words.len());
        if let Some(hash) = hash
            && !self.project.transcripts.iter().any(|t| t.hash == hash)
        {
            return Err(error(
                "TRANSCRIPT_CHANGED",
                "The transcript hash has changed.",
                "Call transcript.get and use the current hash.",
            ));
        }
        for &word in words {
            // A transcript word outside every clip is already cut: skip it, like the prototype.
            if self.word_text(word).is_none() {
                return Err(not_found(&word, "transcript.get lists words."));
            }
            let Some(span) = time::word_span(&self.project, word, hash)? else {
                continue;
            };
            let slot = clip_slot(&self.project, span.clip)?;
            let clip = self.project.clips.timing(slot).expect("live clip");
            let from = (span.start - 2).max(clip.start);
            let to = (span.end + 2).min(clip.start + clip.length);
            ranges.push((from, to));
        }
        ranges.sort_unstable_by_key(|&(from, _)| std::cmp::Reverse(from));
        let mut limit = Frame::MAX;
        for (from, to) in ranges {
            let to = to.min(limit);
            if to - from >= 1 {
                self.cut_range(from, to, true)?;
                limit = from;
            }
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Deleted {} word(s)", words.len())),
            primary: None,
            created: Vec::new(),
        })
    }

    // ── Lift and replace ──

    fn lift(&mut self, items: &[Target], lane: Option<&Target>) -> Result<Outcome, CommandError> {
        let mut created = Vec::new();
        for target in items {
            let TimelineId::Clip(id) = self.timeline(target)? else {
                return Err(wrong_kind("Lift spine clips (clp_…)."));
            };
            let slot = clip_slot(&self.project, id)?;
            let timing = self.project.clips.timing(slot).expect("live clip");
            let data = self.project.clips.data(slot).expect("live clip");
            let Some(media_id) = data.media else {
                return Err(error(
                    "WRONG_KIND",
                    "A gap can't become an overlay.",
                    "Delete the gap instead.",
                ));
            };
            if timing.speed != 1.0 {
                // ponytail: overlays have no speed. Lift retimed clips once overlays can retime.
                return Err(error(
                    "WRONG_KIND",
                    "Overlays play at normal speed.",
                    "Set clip.speed to 1 before lifting.",
                ));
            }
            let framing = data.framing;
            let name = data.appearance.name.clone();
            let kids = self.children(&[TimelineId::Clip(id)])?;
            self.project.clips.remove(slot);
            self.shift_after(timing.start + timing.length, -timing.length, None)?;
            self.reanchor_at(&kids)?;
            let outcome = self.add_item(
                OverlayContent::Media {
                    media: Target::Media(media_id),
                    source: timing.source,
                    placement: Placement::Full,
                    behind_presenter: false,
                    muted: false,
                    crop: framing.crop,
                    fit: framing.fit,
                },
                Some(&FrameArg::Frame(timing.start)),
                None,
                lane,
                Some(timing.length),
            )?;
            if let Some(Target::Timeline(TimelineId::Item(item))) = outcome.created.first() {
                let slot = item_slot(&self.project, *item)?;
                self.project
                    .items
                    .data_mut(slot)
                    .expect("live item")
                    .appearance
                    .name = name;
            }
            created.extend(outcome.created);
        }
        Ok(Outcome {
            summary: Summary::Text(format!("Lifted {} clip(s) to an overlay lane", items.len())),
            primary: created.first().cloned(),
            created,
        })
    }

    fn replace_spine(
        &mut self,
        item: &Target,
        target: Option<&Target>,
    ) -> Result<Outcome, CommandError> {
        let TimelineId::Item(item_id) = self.timeline(item)? else {
            return Err(wrong_kind("Pass a media overlay (ovl_…)."));
        };
        let item_slot_ = item_slot(&self.project, item_id)?;
        let timing = self.project.items.timing(item_slot_).expect("live item");
        let ItemKind::Media(overlay) = self.project.items.data(item_slot_).expect("live").kind
        else {
            return Err(wrong_kind("Pass a media overlay (ovl_…)."));
        };
        let target_id = match target {
            Some(target) => match self.timeline(target)? {
                TimelineId::Clip(id) => id,
                _ => return Err(wrong_kind("target is a spine clip (clp_…).")),
            },
            None => match timing.anchor {
                Anchor::ClipSource { clip, .. } => clip,
                _ => {
                    let at = time::item_span(&self.project, item_slot_)?.start;
                    match anchor_at(&self.project, at)? {
                        Anchor::ClipSource { clip, .. } => clip,
                        _ => {
                            return Err(error(
                                "NOT_FOUND",
                                "No spine clip under the overlay.",
                                "Pass target with a clip ID.",
                            ));
                        }
                    }
                }
            },
        };
        let slot = clip_slot(&self.project, target_id)?;
        let old = self.project.clips.timing(slot).expect("live clip");
        let parents = [TimelineId::Clip(target_id), TimelineId::Item(item_id)];
        let kids: Vec<_> = self
            .children(&parents)?
            .into_iter()
            .filter(|&(kid, _)| kid != item_slot_)
            .collect();
        let descriptor = media(&self.project, overlay.media)?;
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
        self.project.clips.remove(slot);
        self.project.items.remove(item_slot_);
        self.shift_after(old.start + old.length, timing.length - old.length, None)?;
        let id = ClipId::from_serial(self.mint("clp")?);
        self.project
            .clips
            .insert(Clip {
                id,
                timing: ClipTiming {
                    start: old.start,
                    length: timing.length,
                    source: overlay.source,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(overlay.media),
                    framing: overlay.framing,
                    enabled,
                    ..ClipData::default()
                },
            })
            .ok_or_else(edits::arena_full)?;
        self.project.clips.sort_by_start();
        self.reanchor_at(&kids)?;
        Ok(Outcome::create(
            format!("Replaced {target_id} with {item_id}"),
            Target::Timeline(TimelineId::Clip(id)),
        ))
    }

    // ── Captions ──

    fn caption_slot(&self, target: &Target) -> Result<Slot, CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            return Err(wrong_kind("Pass a caption (ovl_…)."));
        };
        let slot = item_slot(&self.project, id)?;
        match self.project.items.data(slot).expect("live item").kind {
            ItemKind::Caption { .. } => Ok(slot),
            _ => Err(wrong_kind("Pass a caption (ovl_…).")),
        }
    }

    fn generate_captions(
        &mut self,
        clips: Option<&[Target]>,
        max_words: Frame,
        fillers: bool,
        lane: Option<&Target>,
        style: Option<String>,
    ) -> Result<Outcome, CommandError> {
        if !(1..=8).contains(&max_words) {
            return Err(CommandError::bad("maxWords", "an integer from 1 to 8"));
        }
        let mut chosen = Vec::new();
        match clips {
            Some(targets) => {
                for target in targets {
                    let (id, _) = self.take_clip(target)?;
                    chosen.push(id);
                }
            }
            None => {
                for &slot in self.project.clips.order() {
                    let media = self.project.clips.data(slot).expect("live clip").media;
                    if media.is_some_and(|m| self.project.transcripts.iter().any(|t| t.media == m))
                    {
                        chosen.push(self.project.clips.id(slot).expect("live clip"));
                    }
                }
            }
        }
        if chosen.is_empty() {
            return Err(error(
                "NOT_FOUND",
                "No clip with a transcript is on the timeline.",
                "Transcribe a take first (transcript.get), or insert one.",
            ));
        }
        let lane = match lane {
            Some(lane) => {
                let Target::Lane(id) = self.resolve(lane)? else {
                    return Err(wrong_kind("Pass a captions lane or omit lane."));
                };
                let found = self
                    .project
                    .lanes
                    .iter()
                    .find(|l| l.id == id)
                    .ok_or_else(|| not_found(&id, "project.info lists lanes."))?;
                if found.kind != LaneKind::Captions {
                    return Err(wrong_kind("Pass a captions lane or omit lane."));
                }
                id
            }
            None => self.default_lane(LaneKind::Captions)?,
        };
        // Visible words (word, clip, source start, timeline start, end), in timeline order.
        let mut visible: Vec<(WordId, ClipId, Frame, Frame, Frame, String)> = Vec::new();
        for transcript in &self.project.transcripts {
            for word in &transcript.words {
                if !fillers && is_filler(&word.text) {
                    continue;
                }
                if let Some(span) = time::word_span(&self.project, word.id, Some(transcript.hash))?
                    && chosen.contains(&span.clip)
                {
                    visible.push((
                        word.id,
                        span.clip,
                        span.source,
                        span.start,
                        span.end,
                        word.text.clone(),
                    ));
                }
            }
        }
        if visible.is_empty() {
            return Err(error(
                "NOT_FOUND",
                "Those clips have no words on the timeline.",
                "Every word is cut or the clips have no speech.",
            ));
        }
        visible.sort_by_key(|word| word.3);
        let ids: Vec<WordId> = visible.iter().map(|word| word.0).collect();
        for slot in self.project.items.order().to_vec() {
            let data = self.project.items.data(slot).expect("live item");
            if data.lane == lane
                && let ItemKind::Caption { words, .. } = &data.kind
                && words.iter().any(|word| ids.contains(word))
            {
                let id = self.project.items.id(slot).expect("live item");
                self.project.items.remove(slot);
                self.report.side_effects.push(SideEffect::Removed {
                    id,
                    cascade_from: None,
                });
            }
        }
        // Break at maxWords, a pause over 12 frames, a comma-length pause once a cue has two
        // words, or past 22 characters (core.js cueGroups; our words carry no punctuation).
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for index in 0..visible.len() {
            if let Some(&last) = current.last() {
                let gap = visible[index].3 - visible[last].4;
                let chars: usize = current
                    .iter()
                    .map(|&i| visible[i].5.len() + 1)
                    .sum::<usize>()
                    + visible[index].5.len();
                if current.len() >= max_words as usize
                    || gap > 12
                    || (gap >= 8 && current.len() > 1)
                    || chars > 22
                {
                    groups.push(std::mem::take(&mut current));
                }
            }
            current.push(index);
        }
        groups.push(current);
        let mut created = Vec::new();
        for group in &groups {
            let first = &visible[group[0]];
            let id = ItemId::from_serial(self.mint("ovl")?);
            self.project
                .items
                .insert(Item {
                    id,
                    timing: ItemTiming {
                        anchor: Anchor::ClipSource {
                            clip: first.1,
                            source: first.2,
                        },
                        length: 3,
                    },
                    data: ItemData {
                        lane,
                        kind: ItemKind::Caption {
                            words: group.iter().map(|&i| visible[i].0).collect(),
                            edits: Vec::new(),
                        },
                        appearance: Appearance::default(),
                    },
                })
                .ok_or_else(edits::arena_full)?;
            self.explicit_lanes.push(id);
            created.push(Target::Timeline(TimelineId::Item(id)));
        }
        let index = self
            .project
            .lanes
            .iter()
            .position(|l| l.id == lane)
            .expect("captions lane");
        let styled = self.project.lanes[index]
            .effects
            .iter()
            .any(|effect| effect.kind == "captions");
        if style.is_some() || !styled {
            let preset = EffectParam {
                name: "preset".into(),
                value: EffectValue::Text(style.unwrap_or_else(|| "bold".into())),
            };
            self.upsert(
                Owner::Lane(index),
                "captions",
                vec![preset],
                None,
                &mut Vec::new(),
            )?;
        }
        Ok(Outcome {
            summary: Summary::Text(format!(
                "Generated {} captions on {lane} ({max_words} words max)",
                groups.len()
            )),
            primary: Some(Target::Lane(lane)),
            created,
        })
    }

    fn set_caption(
        &mut self,
        item: &Target,
        text: Option<String>,
        patches: Vec<CaptionWordPatch>,
    ) -> Result<Outcome, CommandError> {
        let slot = self.caption_slot(item)?;
        let ItemKind::Caption { words, .. } = &self.project.items.data(slot).expect("live").kind
        else {
            unreachable!("checked caption")
        };
        let words = words.clone();
        let visible: Vec<(WordId, String)> = words
            .iter()
            .filter_map(|&id| {
                let span = time::word_span(&self.project, id, None).ok().flatten()?;
                let _ = span;
                Some((id, self.word_text(id)?))
            })
            .collect();
        let ItemKind::Caption { edits, .. } =
            &mut self.project.items.data_mut(slot).expect("live").kind
        else {
            unreachable!("checked caption")
        };
        if let Some(text) = text {
            let tokens: Vec<&str> = text.split_whitespace().collect();
            if tokens.is_empty() {
                return Err(error(
                    "BAD_ARGS",
                    "text can't be empty.",
                    "Hide words with words:{id:{hidden:true}}, or item.delete the cue.",
                ));
            }
            for (index, (id, original)) in visible.iter().enumerate() {
                let token = if index + 1 < visible.len() {
                    tokens.get(index).map(|t| (*t).to_owned())
                } else {
                    (index < tokens.len()).then(|| tokens[index..].join(" "))
                };
                let edit = caption_edit(edits, *id);
                // A hidden word is stored as empty display text.
                edit.text = match token {
                    None => Some(String::new()),
                    Some(token) if token == *original => None,
                    Some(token) => Some(token),
                };
            }
        }
        for patch in patches {
            if !words.contains(&patch.word) {
                return Err(error(
                    "NOT_FOUND",
                    &format!("{} isn't in this caption.", patch.word),
                    "items.inspect shows the cue's words.",
                ));
            }
            let Some(change) = patch.edit else {
                edits.retain(|edit| edit.word != patch.word);
                continue;
            };
            let original = visible
                .iter()
                .find(|(id, _)| *id == patch.word)
                .map(|(_, text)| text.clone());
            let edit = caption_edit(edits, patch.word);
            if let Some(text) = change.text {
                edit.text = text.filter(|t| !t.is_empty() && Some(t) != original.as_ref());
            }
            if let Some(emoji) = change.emoji {
                edit.emoji = emoji
                    .filter(|e| !e.is_empty())
                    .map(|e| e.chars().take(8).collect());
            }
            if let Some(emph) = change.emph {
                edit.emph = emph.unwrap_or(false);
            }
            match change.hidden {
                Some(Some(true)) => edit.text = Some(String::new()),
                Some(_) if edit.text.as_deref() == Some("") => edit.text = None,
                _ => {}
            }
        }
        edits.retain(|edit| edit.text.is_some() || edit.emph || edit.emoji.is_some());
        let id = self.project.items.id(slot).expect("live item");
        Ok(Outcome::edit(
            format!("Caption {id}"),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }

    fn word_text(&self, id: WordId) -> Option<String> {
        self.project
            .transcripts
            .iter()
            .flat_map(|t| &t.words)
            .find(|word| word.id == id)
            .map(|word| word.text.clone())
    }

    fn split_caption(&mut self, item: &Target, word: WordId) -> Result<Outcome, CommandError> {
        let slot = self.caption_slot(item)?;
        let old = self.project.items.timing(slot).expect("live item");
        let data = self.project.items.data_mut(slot).expect("live item");
        let lane = data.lane;
        let appearance = data.appearance.clone();
        let ItemKind::Caption { words, edits } = &mut data.kind else {
            unreachable!("checked caption")
        };
        let index = words
            .iter()
            .position(|&w| w == word)
            .filter(|&i| i >= 1)
            .ok_or_else(|| {
                CommandError::bad("word", "a word in this caption after the first one")
            })?;
        let right_words = words.split_off(index);
        let (right_edits, left_edits): (Vec<_>, Vec<_>) = std::mem::take(edits)
            .into_iter()
            .partition(|edit| right_words.contains(&edit.word));
        *edits = left_edits;
        let id = ItemId::from_serial(self.mint("ovl")?);
        self.project
            .items
            .insert(Item {
                id,
                timing: old,
                data: ItemData {
                    lane,
                    kind: ItemKind::Caption {
                        words: right_words,
                        edits: right_edits,
                    },
                    appearance,
                },
            })
            .ok_or_else(edits::arena_full)?;
        self.explicit_lanes.push(id);
        let left = self.project.items.id(slot).expect("live item");
        Ok(Outcome {
            summary: Summary::Text(format!("Split caption {left} before {word}")),
            primary: Some(Target::Timeline(TimelineId::Item(left))),
            created: vec![Target::Timeline(TimelineId::Item(id))],
        })
    }

    fn merge_captions(&mut self, items: &[Target]) -> Result<Outcome, CommandError> {
        let mut cues = Vec::new();
        for item in items {
            let slot = self.caption_slot(item)?;
            if !cues.iter().any(|&(s, _)| s == slot) {
                cues.push((slot, time::item_span(&self.project, slot)?.start));
            }
        }
        if cues.len() < 2 {
            return Err(CommandError::bad("items", "two or more captions"));
        }
        cues.sort_by_key(|&(_, start)| start);
        let lane = self.project.items.data(cues[0].0).expect("live").lane;
        if cues
            .iter()
            .any(|&(slot, _)| self.project.items.data(slot).expect("live").lane != lane)
        {
            return Err(error(
                "WRONG_KIND",
                "Those captions are on different lanes.",
                "Merge captions on one lane.",
            ));
        }
        let mut words = Vec::new();
        let mut merged = Vec::new();
        for &(slot, _) in &cues[1..] {
            let data = self.project.items.data_mut(slot).expect("live");
            if let ItemKind::Caption { words: w, edits } = &mut data.kind {
                words.append(w);
                merged.append(edits);
            }
            self.project.items.remove(slot);
        }
        let order: Vec<WordId> = self
            .project
            .transcripts
            .iter()
            .flat_map(|t| t.words.iter().map(|w| w.id))
            .collect();
        let keep = cues[0].0;
        let ItemKind::Caption { words: kept, edits } =
            &mut self.project.items.data_mut(keep).expect("live").kind
        else {
            unreachable!("checked caption")
        };
        kept.append(&mut words);
        kept.sort_by_key(|word| order.iter().position(|w| w == word).unwrap_or(usize::MAX));
        kept.dedup();
        edits.append(&mut merged);
        let id = self.project.items.id(keep).expect("live item");
        Ok(Outcome::edit(
            format!("Merged {} captions", cues.len()),
            Target::Timeline(TimelineId::Item(id)),
        ))
    }
}

fn caption_edit(edits: &mut Vec<CaptionEdit>, word: WordId) -> &mut CaptionEdit {
    let index = match edits.iter().position(|edit| edit.word == word) {
        Some(index) => index,
        None => {
            edits.push(CaptionEdit {
                word,
                text: None,
                emph: false,
                emoji: None,
            });
            edits.len() - 1
        }
    };
    &mut edits[index]
}

fn is_filler(text: &str) -> bool {
    matches!(
        text.trim_matches(|c: char| !c.is_alphabetic())
            .to_ascii_lowercase()
            .as_str(),
        "um" | "uh" | "er" | "erm" | "ah" | "hmm"
    )
}

fn number(name: &str, value: f64) -> EffectParam {
    EffectParam {
        name: name.into(),
        value: EffectValue::Number(value),
    }
}

fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

fn check_scale(value: f64, name: &str) -> Result<(), CommandError> {
    if (0.5..=2.0).contains(&value) {
        Ok(())
    } else {
        Err(CommandError::bad(name, "a scale from 0.5 to 2"))
    }
}

fn no_source_frame() -> CommandError {
    error(
        "WRONG_KIND",
        "That item has no fixed source frame to crop.",
        "Crop works on screen takes, video files and media overlays.",
    )
}

fn component_state(states: &mut ComponentStates, component: Component) -> &mut Option<bool> {
    match component {
        Component::Screen => &mut states.screen,
        Component::Camera => &mut states.camera,
        Component::Mic => &mut states.mic,
        Component::System => &mut states.system,
        Component::Pointer => &mut states.pointer,
        Component::Keys => &mut states.keys,
    }
}

fn component_name(component: Component) -> &'static str {
    match component {
        Component::Screen => "Screen video",
        Component::Camera => "Camera video",
        Component::Mic => "Microphone",
        Component::System => "System audio",
        Component::Pointer => "Cursor",
        Component::Keys => "Keystrokes",
    }
}

fn no_component(id: ClipId, component: Component) -> CommandError {
    error(
        "WRONG_KIND",
        &format!("{id} has no {} component.", component_name(component)),
        "items.inspect lists the clip's components.",
    )
}
