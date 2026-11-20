//! AC-CROP-2: a crop or fit change keeps what sits on the picture on the same source content.
//! Zoom `focus` is crop space and mask `x, y, w, h` picture space (docs/MCP.md, A5), so a new
//! crop re-expresses both instead of leaving the numbers to slide over other content.
use super::settle::{Owner, effects, effects_vec_mut};
use super::time::{clip_slot, item_slot, media};
use super::*;

/// The whole source.
const FULL: Rect = Rect {
    x: 0.0,
    y: 0.0,
    w: 1.0,
    h: 1.0,
};

/// Mask parameters in picture space, each with its axis (0 x, 1 y) and whether it is a size.
const MASK_RECT: [(&str, usize, bool); 4] = [
    ("x", 0, false),
    ("y", 1, false),
    ("w", 0, true),
    ("h", 1, true),
];

impl Editor {
    /// Re-expresses the masks of `id` (framed by `before` until now), and the zooms that start
    /// over it when it is a spine clip, in its current framing; each change is a `recropped`
    /// side effect, flagged `outside` when part of its content is now cropped away.
    pub(super) fn recrop(&mut self, id: TimelineId, before: Framing) -> Result<(), CommandError> {
        let canvas = self.project.size.map(f64::from);
        let (owner, after, media_id, bounds, padding) = match id {
            TimelineId::Clip(clip) => {
                let slot = clip_slot(&self.project, clip)?;
                let data = self.project.clips.data(slot).expect("live clip");
                let Some(media_id) = data.media else {
                    return Ok(());
                };
                let scopes: [&[Effect]; 3] = [
                    &self.project.effects,
                    &self.project.spine.effects,
                    &data.appearance.effects,
                ];
                let bounds = Placement::Full.bounds(canvas);
                (
                    Owner::Clip(slot),
                    data.framing,
                    media_id,
                    bounds,
                    padding(&scopes),
                )
            }
            TimelineId::Item(item) => {
                let slot = item_slot(&self.project, item)?;
                let data = self.project.items.data(slot).expect("live item");
                let ItemKind::Media(overlay) = &data.kind else {
                    return Ok(());
                };
                let lane = self
                    .project
                    .lanes
                    .iter()
                    .find(|lane| lane.id == data.lane)
                    .map_or(&[][..], |lane| &lane.effects);
                let scopes: [&[Effect]; 2] = [lane, &data.appearance.effects];
                let bounds = overlay.placement.bounds(canvas);
                (
                    Owner::Item(slot),
                    overlay.framing,
                    overlay.media,
                    bounds,
                    padding(&scopes),
                )
            }
            TimelineId::Marker(_) => return Ok(()),
        };
        // An animated crop (crop.keys) is the picture itself: the static crop draws nothing.
        if enabled(&[effects(&self.project, owner)], "crop") {
            return Ok(());
        }
        let probe = &media(&self.project, media_id)?.probe;
        let size = [probe.width, probe.height];
        if size.contains(&0) {
            return Ok(());
        }
        let margin = bounds.w.min(bounds.h) * padding;
        let inner = [bounds.w - 2.0 * margin, bounds.h - 2.0 * margin];
        let (old, new) = (before.visible(size, inner), after.visible(size, inner));
        if old != new {
            self.recrop_masks(id, owner, old, new)?;
        }
        let (old, new) = (before.crop.unwrap_or(FULL), after.crop.unwrap_or(FULL));
        if let (TimelineId::Clip(clip), true) = (id, old != new) {
            self.recrop_zooms(clip, old, new)?;
        }
        Ok(())
    }

    /// Masks are picture space: 0–1 of the visible source rectangle `old`, now `new`.
    fn recrop_masks(
        &mut self,
        id: TimelineId,
        owner: Owner,
        old: Rect,
        new: Rect,
    ) -> Result<(), CommandError> {
        let schema = crate::commands::effect_schemas()
            .field("mask")
            .and_then(|mask| mask.field("params"));
        let spec = |name: &str, key: &str| {
            schema
                .and_then(|params| params.field(name))
                .and_then(|param| param.field(key))
                .and_then(Json::as_f64)
                .unwrap_or(0.0)
        };
        let map = |value: f64, axis: usize, size: bool| {
            let (o, n) = if axis == 0 {
                ([old.x, old.w], [new.x, new.w])
            } else {
                ([old.y, old.h], [new.y, new.h])
            };
            if size {
                value * o[1] / n[1]
            } else {
                (o[0] + value * o[1] - n[0]) / n[1]
            }
        };
        let count = effects(&self.project, owner).len();
        for index in 0..count {
            let effect = &effects(&self.project, owner)[index];
            if effect.kind != "mask" {
                continue;
            }
            let mask = effect.id;
            // Refuse before writing anything: the new numbers must still be a valid mask.
            // [lowest edge, highest edge] per axis, over every key.
            let mut span = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
            let mut extent = [0.0_f64; 2];
            for (name, axis, size) in MASK_RECT {
                let value = effect.params.iter().find(|p| p.name == name);
                let mut check = |v: f64| {
                    let mapped = map(v, axis, size);
                    if !(mapped.is_finite()
                        && mapped >= spec(name, "min")
                        && mapped <= spec(name, "max"))
                    {
                        return Err(crop_moves_mask(mask, name, mapped));
                    }
                    if size {
                        extent[axis] = extent[axis].max(mapped);
                    } else {
                        span[axis] = [span[axis][0].min(mapped), span[axis][1].max(mapped)];
                    }
                    Ok(())
                };
                match value.map(|p| &p.value) {
                    Some(EffectValue::Number(n)) => check(*n)?,
                    Some(EffectValue::Keys(keys)) => {
                        for key in keys {
                            check(key.value)?;
                        }
                    }
                    _ => check(spec(name, "def"))?,
                }
            }
            let outside = (0..2).any(|axis| {
                span[axis][0] < -EPSILON || span[axis][1] + extent[axis] > 1.0 + EPSILON
            });
            let params = &mut effects_vec_mut(&mut self.project, owner)[index].params;
            for (name, axis, size) in MASK_RECT {
                match params.iter_mut().find(|p| p.name == name) {
                    Some(param) => match &mut param.value {
                        EffectValue::Number(n) => *n = round6(map(*n, axis, size)),
                        EffectValue::Keys(keys) => {
                            for key in keys {
                                key.value = round6(map(key.value, axis, size));
                            }
                        }
                        _ => {}
                    },
                    None => params.push(EffectParam {
                        name: name.into(),
                        value: EffectValue::Number(round6(map(spec(name, "def"), axis, size))),
                    }),
                }
            }
            self.report.side_effects.push(SideEffect::Recropped {
                id: Recropped::Mask(mask),
                owner: id,
                outside,
                cause: Cause::Crop,
            });
        }
        Ok(())
    }

    /// Zoom focus is crop space (0–1 of the spine clip's crop, `old`, now `new`) for the zooms
    /// that start over `clip` (the evaluator reads the clip under a zoom's first frame).
    fn recrop_zooms(&mut self, clip: ClipId, old: Rect, new: Rect) -> Result<(), CommandError> {
        let slot = clip_slot(&self.project, clip)?;
        let timing = self.project.clips.timing(slot).expect("live clip");
        let span = timing.start..timing.start.saturating_add(timing.length);
        for index in 0..self.project.items.order().len() {
            let slot = self.project.items.order()[index];
            let data = self.project.items.data(slot).expect("live item");
            let ItemKind::Zoom(Zoom {
                focus: Some(focus), ..
            }) = data.kind
            else {
                continue;
            };
            let anchor = self.project.items.timing(slot).expect("live item").anchor;
            if !splice_model::timing::anchor_start(&self.project, anchor)
                .is_ok_and(|at| span.contains(&at))
            {
                continue;
            }
            let mapped = [
                (old.x + focus[0] * old.w - new.x) / new.w,
                (old.y + focus[1] * old.h - new.y) / new.h,
            ];
            // A point cropped away cannot be framed: the zoom holds the nearest edge.
            let outside = mapped
                .iter()
                .any(|v| !(-EPSILON..=1.0 + EPSILON).contains(v));
            let data = self.project.items.data_mut(slot).expect("live item");
            if let ItemKind::Zoom(zoom) = &mut data.kind {
                zoom.focus = Some(mapped.map(|v| round6(v.clamp(0.0, 1.0))));
            }
            let Some(zoom) = self.project.items.id(slot) else {
                continue;
            };
            self.report.side_effects.push(SideEffect::Recropped {
                id: Recropped::Zoom(zoom),
                owner: TimelineId::Clip(clip),
                outside,
                cause: Cause::Crop,
            });
        }
        Ok(())
    }
}

/// Rounding slack when deciding that content left the picture.
const EPSILON: f64 = 1e-6;

fn round6(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// Whether the last `kind` effect across `scopes` is on (the evaluator's inheritance rule).
fn enabled(scopes: &[&[Effect]], kind: &str) -> bool {
    let mut on = None;
    for effect in scopes.iter().flat_map(|scope| scope.iter()) {
        if effect.kind == kind {
            on = effect.enabled.or(on).or(Some(true));
        }
    }
    on == Some(true)
}

/// The window padding the evaluator gives a picture under `scopes` (0 without a window).
fn padding(scopes: &[&[Effect]]) -> f64 {
    if !enabled(scopes, "window") {
        return 0.0;
    }
    let set = scopes
        .iter()
        .flat_map(|scope| scope.iter())
        .filter(|effect| effect.kind == "window")
        .flat_map(|effect| effect.params.iter())
        .filter(|param| param.name == "padding")
        .filter_map(|param| match param.value {
            EffectValue::Number(n) if n.is_finite() => Some(n),
            _ => None,
        })
        .next_back();
    let default = || {
        crate::commands::effect_schemas()
            .field("window")
            .and_then(|window| window.field("params"))
            .and_then(|params| params.field("padding"))
            .and_then(|padding| padding.field("def"))
            .and_then(Json::as_f64)
            .unwrap_or(0.0)
    };
    set.unwrap_or_else(default).clamp(0.0, 0.3)
}

fn crop_moves_mask(mask: EffectId, name: &str, value: f64) -> CommandError {
    error(
        "OUT_OF_RANGE",
        &format!(
            "This crop would move mask {mask} to {name} {value:.3}, outside what a mask can \
             hold, to keep it on the same source content."
        ),
        "Crop less tightly, or delete or shrink the mask first (mask.set or effect.remove).",
    )
}
