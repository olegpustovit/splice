//! Graphics (PLAN.md T45.4): counters, progress bars and rings, particle emitters and look
//! packs. Each writes an ordinary item and ordinary effects (`counter`, `progress`,
//! `particles`, `color`, `window`, `captions`); the evaluator does the per-frame work.
use super::settle::effects;
use super::tier_b::applies;
use super::time::item_slot;
use super::*;
use crate::commands::{AnchorArg, FrameArg, OverlayContent};

impl Editor {
    pub(super) fn run_graphics(&mut self, op: Op) -> Result<Outcome, CommandError> {
        match op {
            Op::CounterAdd {
                at,
                length,
                values,
                params,
                ease,
                lane,
                look,
            } => {
                let text = counter_text(
                    values[1],
                    &text_param(&params, "format").unwrap_or_else(|| "{n}".into()),
                    number_param(&params, "decimals").unwrap_or(0.0) as u32,
                    bool_param(&params, "group").unwrap_or(true),
                );
                let outcome = self.add_item(
                    OverlayContent::Title(Some(text)),
                    Some(&at),
                    None,
                    lane.as_ref(),
                    Some(length),
                )?;
                let target = outcome.primary.clone().expect("created title");
                let last = self.item_length(&target)? - 1;
                let mut params = params;
                params.push(EffectParam {
                    name: "value".into(),
                    value: EffectValue::Keys(vec![
                        Key {
                            at: 0,
                            value: values[0],
                            ease,
                        },
                        Key {
                            at: last,
                            value: values[1],
                            ease: Ease::Linear,
                        },
                    ]),
                });
                let owner = self.owner(&target)?;
                self.upsert(owner, "counter", params, None, &mut Vec::new())?;
                self.title_look(outcome, look.as_deref())
            }
            Op::ProgressAdd {
                shape,
                at,
                length,
                rect,
                lane,
                mut style,
                mut params,
            } => {
                let at = match &at {
                    Some(at) => self.frame(at)?,
                    None => 0,
                };
                let length = match length {
                    Some(length) => length,
                    None => self.spine_end()? - at,
                };
                let [w, h] = self.project.size.map(f64::from);
                let rect = rect.unwrap_or(match shape {
                    // A centered circle a sixth of the shorter side across.
                    ShapeKind::Ring => {
                        let side = w.min(h) / 6.0;
                        Rect {
                            x: 0.5 - side / w / 2.0,
                            y: 0.5 - side / h / 2.0,
                            w: side / w,
                            h: side / h,
                        }
                    }
                    _ => Rect {
                        x: 0.04,
                        y: 0.95,
                        w: 0.92,
                        h: 0.012,
                    },
                });
                // Pinned to the timeline: it measures playback, not a clip.
                let outcome = self.add_item(
                    OverlayContent::Shape { shape, rect },
                    None,
                    Some(&AnchorArg::Pinned(Some(FrameArg::Frame(at)))),
                    lane.as_ref(),
                    Some(length),
                )?;
                let target = outcome.primary.clone().expect("created shape");
                let last = self.item_length(&target)? - 1;
                let default = match shape {
                    ShapeKind::Ring => ("strokeWidth", 0.012),
                    _ => ("corner", 0.5),
                };
                if !style.iter().any(|param| param.name == default.0) {
                    style.push(EffectParam {
                        name: default.0.into(),
                        value: EffectValue::Number(default.1),
                    });
                }
                params.push(EffectParam {
                    name: "value".into(),
                    value: EffectValue::Keys(vec![
                        Key {
                            at: 0,
                            value: 0.0,
                            ease: Ease::Linear,
                        },
                        Key {
                            at: last,
                            value: 1.0,
                            ease: Ease::Linear,
                        },
                    ]),
                });
                let owner = self.owner(&target)?;
                self.upsert(owner, "shape", style, None, &mut Vec::new())?;
                self.upsert(owner, "progress", params, None, &mut Vec::new())?;
                Ok(outcome)
            }
            Op::ParticlesAdd {
                at,
                length,
                rect,
                lane,
                params,
            } => {
                let rect = rect.unwrap_or(Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 1.0,
                    h: 1.0,
                });
                let outcome = self.add_item(
                    OverlayContent::Shape {
                        shape: ShapeKind::Rect,
                        rect,
                    },
                    Some(&at),
                    None,
                    lane.as_ref(),
                    length,
                )?;
                let target = outcome.primary.clone().expect("created shape");
                let owner = self.owner(&target)?;
                self.upsert(owner, "particles", params, None, &mut Vec::new())?;
                Ok(outcome)
            }
            Op::LookApply {
                target,
                name,
                pack,
                effects,
            } => self.apply_saved_look(&target, &name, pack, effects),
            Op::LookSave { target, file } => {
                let owner = self.owner(&target)?;
                let kinds = saved_kinds(effects(&self.project, owner));
                let label = self.owner_label(owner);
                if kinds.is_empty() {
                    return Err(CommandError::rule(
                        "NOTHING_TO_SAVE",
                        "target",
                        None,
                        "a target with its own window, camera, cursor, color or captions effect",
                        &format!(
                            "{label} has no look effects of its own. Save the owner that holds \
                             them (items.inspect shows each effect's scope), often \"project\"."
                        ),
                    ));
                }
                Ok(Outcome::edit(
                    format!("Saved the look of {label} ({}) to {file}", kinds.join(", ")),
                    target,
                ))
            }
            _ => unreachable!("graphics dispatch"),
        }
    }

    fn item_length(&self, target: &Target) -> Result<Frame, CommandError> {
        let TimelineId::Item(id) = self.timeline(target)? else {
            unreachable!("created an item");
        };
        let slot = item_slot(&self.project, id)?;
        Ok(self.project.items.timing(slot).expect("live item").length)
    }

    /// Writes a look's effects into the target. A part the target cannot hold is skipped and
    /// named; a pack only restyles a window where the target already has one (a new window
    /// effect would put every picture in a window), while a saved look brings its window along.
    fn apply_saved_look(
        &mut self,
        target: &Target,
        pack: &str,
        is_pack: bool,
        parts: Vec<(String, Vec<EffectParam>)>,
    ) -> Result<Outcome, CommandError> {
        let owner = self.owner(target)?;
        let mut written = Vec::new();
        let mut skipped = Vec::new();
        for (kind, params) in parts {
            let (class, _) = applies(&kind)?;
            let windowless = is_pack
                && kind == "window"
                && !effects(&self.project, owner)
                    .iter()
                    .any(|e| e.kind == "window");
            if !self.fits(class, owner) || windowless {
                skipped.push(kind);
                continue;
            }
            self.upsert(owner, &kind, params, None, &mut Vec::new())?;
            written.push(kind);
        }
        let label = self.owner_label(owner);
        let mut summary = format!("Applied the {pack} look to {label}: {}", written.join(", "));
        if written.is_empty() {
            summary = format!("The {pack} look has nothing that applies to {label}");
        }
        if !skipped.is_empty() {
            summary.push_str(&format!(
                " (skipped {}: not on this target)",
                skipped.join(", ")
            ));
        }
        Ok(Outcome::edit(summary, target.clone()))
    }
}

/// The look effect types an owner holds itself and has on.
fn saved_kinds(list: &[Effect]) -> Vec<&'static str> {
    crate::commands::LOOK_KINDS
        .iter()
        .copied()
        .filter(|kind| {
            list.iter()
                .any(|effect| effect.kind == *kind && effect.enabled != Some(false))
        })
        .collect()
}

/// The file `look.save {target}` writes (T46.3): `{"splice":"look","version":1,"effects":
/// {type: {param: value}}}` with the target's own window, camera, cursor, color and captions
/// values (keys as `{keys:[…]}`), the form `look.apply {file}` reads. A window `image` is the
/// media's file, made absolute against `package`, so the look applies in any project.
pub fn saved_look(
    project: &Project,
    target: &Target,
    package: &std::path::Path,
) -> Result<Json, CommandError> {
    let list: &[Effect] = match target {
        Target::Project => &project.effects,
        Target::Lane(id) if *id == LaneId::SPINE => &project.spine.effects,
        Target::Lane(id) => {
            &project
                .lanes
                .iter()
                .find(|lane| lane.id == *id)
                .ok_or_else(|| not_found(id, "project.info lists lanes."))?
                .effects
        }
        Target::Timeline(TimelineId::Clip(id)) => {
            let slot = super::time::clip_slot(project, *id)?;
            &project
                .clips
                .data(slot)
                .expect("live clip")
                .appearance
                .effects
        }
        Target::Timeline(TimelineId::Item(id)) => {
            let slot = item_slot(project, *id)?;
            &project
                .items
                .data(slot)
                .expect("live item")
                .appearance
                .effects
        }
        _ => {
            return Err(wrong_kind(
                "Targets: a clip (clp_), an overlay (ovl_), a lane (lan_…, lan_spine) or \"project\".",
            ));
        }
    };
    let mut kinds = Vec::new();
    for kind in saved_kinds(list) {
        let mut params: Vec<(String, Json)> = Vec::new();
        for effect in list
            .iter()
            .filter(|e| e.kind == kind && e.enabled != Some(false))
        {
            for param in &effect.params {
                let value = match &param.value {
                    EffectValue::Number(n) => Json::Float(*n),
                    EffectValue::Bool(b) => Json::Bool(*b),
                    EffectValue::Text(text) if kind == "window" && param.name == "image" => {
                        let path = text
                            .parse::<MediaId>()
                            .ok()
                            .and_then(|id| project.media.iter().find(|m| m.id == id))
                            .map_or(text.as_str(), |media| media.path.as_str());
                        Json::String(package.join(path).display().to_string())
                    }
                    EffectValue::Text(text) => Json::String(text.clone()),
                    EffectValue::Keys(keys) => crate::package::encode_keys(keys),
                };
                match params.iter_mut().find(|(name, _)| *name == param.name) {
                    Some(slot) => slot.1 = value,
                    None => params.push((param.name.clone(), value)),
                }
            }
        }
        params.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        kinds.push((kind.to_owned(), Json::Object(params)));
    }
    Ok(Json::Object(vec![
        ("splice".into(), Json::String("look".into())),
        ("version".into(), Json::Unsigned(1)),
        ("effects".into(), Json::Object(kinds)),
    ]))
}

fn text_param(params: &[EffectParam], name: &str) -> Option<String> {
    params.iter().find_map(|param| match &param.value {
        EffectValue::Text(text) if param.name == name => Some(text.clone()),
        _ => None,
    })
}

fn number_param(params: &[EffectParam], name: &str) -> Option<f64> {
    params.iter().find_map(|param| match param.value {
        EffectValue::Number(n) if param.name == name => Some(n),
        _ => None,
    })
}

fn bool_param(params: &[EffectParam], name: &str) -> Option<bool> {
    params.iter().find_map(|param| match param.value {
        EffectValue::Bool(b) if param.name == name => Some(b),
        _ => None,
    })
}
