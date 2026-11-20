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
            Op::LookApply { target, pack } => self.apply_look_pack(&target, &pack),
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

    /// Writes a pack's effects into the target. A part the target cannot hold is skipped and
    /// named; a window is only restyled where the target already has one (a new window effect
    /// would put every picture in a window).
    fn apply_look_pack(&mut self, target: &Target, pack: &str) -> Result<Outcome, CommandError> {
        let owner = self.owner(target)?;
        let parts = crate::commands::look_packs()
            .field(pack)
            .and_then(|pack| pack.field("effects"))
            .and_then(Json::as_object)
            .expect("decoded pack");
        let mut written = Vec::new();
        let mut skipped = Vec::new();
        for (kind, values) in parts {
            let (class, _) = applies(kind)?;
            let windowless = kind == "window"
                && !effects(&self.project, owner)
                    .iter()
                    .any(|e| e.kind == "window");
            if !self.fits(class, owner) || windowless {
                skipped.push(kind.as_str());
                continue;
            }
            let params = values
                .as_object()
                .into_iter()
                .flatten()
                .map(|(name, value)| EffectParam {
                    name: name.clone(),
                    value: match value {
                        Json::String(text) => EffectValue::Text(text.clone()),
                        Json::Bool(flag) => EffectValue::Bool(*flag),
                        number => EffectValue::Number(number.as_f64().expect("checked pack")),
                    },
                })
                .collect();
            self.upsert(owner, kind, params, None, &mut Vec::new())?;
            written.push(kind.as_str());
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
