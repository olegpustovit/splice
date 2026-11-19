//! Compound clips (PLAN.md T17.4) render by flattening: each compound clip on the spine is
//! replaced by its nested clips, trimmed to the compound clip's window and shifted onto the
//! outer timeline, and its pinned items join the outer items. Renderers then see ordinary clips.
use super::EvalError;
use splice_model::*;

/// Nesting depth past which a compound refuses to render.
const MAX_DEPTH: usize = 4;

/// The project with every compound clip expanded, or `None` when it has none.
pub(crate) fn flatten(project: &Project) -> Result<Option<Project>, EvalError> {
    if project.compounds.is_empty() {
        return Ok(None);
    }
    let mut flat = project.clone();
    // Each round expands every compound clip on the spine: one nesting level per round.
    for _ in 0..=MAX_DEPTH {
        let round: Vec<ClipId> = flat
            .clips
            .order()
            .iter()
            .filter(|&&slot| {
                flat.clips
                    .data(slot)
                    .and_then(|data| data.media)
                    .is_some_and(|id| flat.compounds.iter().any(|c| c.media == id))
            })
            .map(|&slot| flat.clips.id(slot).expect("live clip"))
            .collect();
        if round.is_empty() {
            return Ok(Some(flat));
        }
        for id in round {
            let slot = flat.clips.find(id).ok_or(EvalError::Range)?;
            expand(&mut flat, slot)?;
        }
    }
    Err(EvalError::Range)
}

fn expand(flat: &mut Project, slot: Slot) -> Result<(), EvalError> {
    let outer = flat.clips.timing(slot).ok_or(EvalError::Range)?;
    let data = flat.clips.data(slot).ok_or(EvalError::Range)?;
    let media = data.media.ok_or(EvalError::Range)?;
    // ponytail: a compound clip's own effects apply to each nested clip on that clip's clock,
    // and retimed compounds play at 1×. Render through an offscreen pass if keyed compound
    // effects or compound speed need exactness.
    let outer_effects = data.appearance.effects.clone();
    let compound = flat
        .compounds
        .iter()
        .find(|c| c.media == media)
        .ok_or(EvalError::Media(media))?
        .clone();
    let id = flat.clips.id(slot).ok_or(EvalError::Range)?;
    flat.clips.remove(slot);
    // Outer items anchored to the compound clip keep their frame once it is gone.
    for index in 0..flat.items.order().len() {
        let item = flat.items.order()[index];
        let timing = flat.items.timing_mut(item).ok_or(EvalError::Range)?;
        match timing.anchor {
            Anchor::ClipSource { clip, source } if clip == id => {
                timing.anchor = Anchor::Pinned(outer.start + source - outer.source);
            }
            Anchor::Item {
                item: TimelineId::Clip(clip),
                edge,
                offset,
            } if clip == id => {
                let base = match edge {
                    Edge::Start => outer.start,
                    Edge::End => outer.start + outer.length,
                };
                timing.anchor = Anchor::Pinned(base + offset);
            }
            _ => {}
        }
    }
    let window = (outer.source, outer.source + outer.length);
    let offset = outer.start - outer.source;
    for mut clip in compound.clips {
        let (from, to) = (clip.timing.start, clip.timing.start + clip.timing.length);
        let (kept_from, kept_to) = (from.max(window.0), to.min(window.1));
        if kept_to <= kept_from {
            continue;
        }
        let rate = match clip.data.media {
            Some(id) => {
                let media = flat
                    .media
                    .iter()
                    .find(|m| m.id == id)
                    .ok_or(EvalError::Media(id))?;
                media.probe.fps / f64::from(flat.fps) * clip.timing.speed
            }
            None => clip.timing.speed,
        };
        clip.timing.source += (f64::from(kept_from - from) * rate).round() as Frame;
        clip.timing.start = kept_from + offset;
        clip.timing.length = kept_to - kept_from;
        clip.data.appearance.fade_in = clip.data.appearance.fade_in.min(clip.timing.length);
        clip.data.appearance.fade_out = clip
            .data
            .appearance
            .fade_out
            .min(clip.timing.length - clip.data.appearance.fade_in);
        let mut effects = outer_effects.clone();
        effects.append(&mut clip.data.appearance.effects);
        clip.data.appearance.effects = effects;
        flat.clips.insert(clip).ok_or(EvalError::Range)?;
    }
    flat.clips.sort_by_start();
    for lane in compound.lanes {
        if !flat.lanes.iter().any(|known| known.id == lane.id) {
            flat.lanes.push(lane);
        }
    }
    for mut item in compound.items {
        let Anchor::Pinned(at) = item.timing.anchor else {
            continue;
        };
        let (from, to) = (at.max(window.0), (at + item.timing.length).min(window.1));
        if to <= from {
            continue;
        }
        item.timing.anchor = Anchor::Pinned(from + offset);
        item.timing.length = to - from;
        flat.items.insert(item).ok_or(EvalError::Range)?;
    }
    Ok(())
}
