//! Scene-graph lints returned with every `edit.apply` and dry run (PLAN.md D18, T43.1): cheap
//! checks of the resulting project that catch what agents miss (text outside the platform's safe
//! zone or over the window, zooms that overflow the canvas, static stretches)
//! with a fix and, where one exists, a suggested op batch. Warnings never block the edit.
use super::{Json, object, text};
use splice_eval::{Evaluator, FramePlan, Layer};
use splice_model::{Frame, ItemKind, Project, TimelineId};

/// The safe zone of a 9:16 short as fractions of the canvas (top, bottom, left, right): the most
/// conservative union of published 1080×1920 figures (research T43.2), versioned so a later
/// profile never silently moves old checks.
pub(crate) const SAFE_ZONE: (&str, [f64; 4]) = ("tiktok-2026", [0.084, 0.1875, 0.084, 0.112]);
/// A static stretch longer than this on a vertical short is flagged.
const LONGEST_STATIC: f64 = 8.0;

/// Lints and pacing stats for `project`; both empty when it cannot be evaluated.
pub(crate) fn check(project: &Project) -> (Vec<Json>, Json) {
    let Ok(evaluator) = Evaluator::prepare(project) else {
        return (Vec::new(), Json::Null);
    };
    let end = project
        .clips
        .order()
        .iter()
        .filter_map(|&slot| project.clips.timing(slot))
        .map(|timing| timing.start + timing.length)
        .max()
        .unwrap_or(0);
    let mut lints = Vec::new();
    let mut plan = evaluator.buffer();
    let [cw, ch] = project.size.map(f64::from);
    let portrait = ch > cw;
    for &slot in project.items.order() {
        let (Some(id), Some(data)) = (project.items.id(slot), project.items.data(slot)) else {
            continue;
        };
        // Resolved from anchors (camera items have no evaluated layer of their own).
        let Ok(span) = splice_core::edit::span(project, TimelineId::Item(id)) else {
            continue;
        };
        let (from, to) = (span.start, span.end);
        match &data.kind {
            ItemKind::Title { .. } => {
                let mid = (from + to) / 2;
                evaluator.frame(mid.min(end.max(1) - 1), &mut plan);
                let Some(bounds) = plan.layers.iter().find_map(|layer| match layer {
                    Layer::Title { item, bounds, .. } if *item == id => Some(*bounds),
                    _ => None,
                }) else {
                    continue;
                };
                if portrait {
                    let [top, bottom, left, right] = SAFE_ZONE.1;
                    let outside = bounds.y < ch * top - 0.5
                        || bounds.y + bounds.h > ch * (1.0 - bottom) + 0.5
                        || bounds.x < cw * left - 0.5
                        || bounds.x + bounds.w > cw * (1.0 - right) + 0.5;
                    if outside {
                        lints.push(lint(
                            "OUTSIDE_SAFE_ZONE",
                            &format!(
                                "{id}'s text box leaves the {} safe zone (platform UI covers it).",
                                SAFE_ZONE.0
                            ),
                            &[id.to_string()],
                            "Move or shrink the title inside the safe zone (title.set box, or a template that uses safe zones).",
                            "",
                        ));
                    }
                }
                if let Some(card) = window_card(&plan) {
                    let overlap = overlap(bounds, card);
                    if overlap > 0.3 * bounds.w * bounds.h {
                        lints.push(lint(
                            "TEXT_OVER_WINDOW",
                            &format!("{id} covers the recorded window, hiding what the viewer should read."),
                            &[id.to_string()],
                            "Place the title in the backdrop's margin, or shrink the window (window padding).",
                            "",
                        ));
                    }
                }
            }
            ItemKind::Zoom(zoom) => {
                let safe = super::timeline::fit_safe_scale(&evaluator, from);
                if zoom.amount > safe + 1e-6 {
                    lints.push(lint(
                        "ZOOM_OVERFLOW",
                        &format!(
                            "{id} zooms to {:.2}×, past {safe:.2}× where the card leaves the canvas.",
                            zoom.amount
                        ),
                        &[id.to_string()],
                        "Lower the amount to fitSafeScale, or use mode content (zoom inside the frame).",
                        &format!(r#"[{{"cmd":"zoom.set","zoom":"{id}","amount":{safe}}}]"#),
                    ));
                }
            }
            _ => {}
        }
    }
    let pacing = pacing(project, end);
    if portrait
        && let Some(longest) = pacing.field("longestStatic").and_then(Json::as_f64)
        && longest > LONGEST_STATIC
    {
        lints.push(lint(
            "LONG_STATIC",
            &format!("Nothing changes on screen for {longest:.1} s; short-form viewers leave during static stretches."),
            &[],
            "Cut the stretch (cut.pauses, intent.pace), or add a zoom or title inside it.",
            "",
        ));
    }
    (lints, pacing)
}

fn lint(code: &str, message: &str, items: &[String], fix: &str, ops: &str) -> Json {
    let mut fields = vec![
        ("code", text(code)),
        ("severity", text("warning")),
        ("message", text(message)),
        (
            "items",
            Json::Array(items.iter().map(|id| text(id)).collect()),
        ),
        ("fix", text(fix)),
    ];
    if let Ok(ops) = splice_json::parse(ops) {
        fields.push(("ops", ops));
    }
    object(fields)
}

/// The spine picture's card (a windowed clip's destination), if a window is drawn.
fn window_card(plan: &FramePlan) -> Option<splice_model::Rect> {
    plan.layers.iter().find_map(|layer| match layer {
        Layer::Picture(p) if matches!(p.item, TimelineId::Clip(_)) && p.window.is_some() => {
            Some(p.destination)
        }
        _ => None,
    })
}

fn overlap(a: splice_model::Rect, b: splice_model::Rect) -> f64 {
    let w = (a.x + a.w).min(b.x + b.w) - a.x.max(b.x);
    let h = (a.y + a.h).min(b.y + b.h) - a.y.max(b.y);
    w.max(0.0) * h.max(0.0)
}

/// Cut rate and the longest stretch without a visual change (a cut, a zoom boundary, or an
/// overlay or title starting or ending), in seconds.
fn pacing(project: &Project, end: Frame) -> Json {
    let fps = f64::from(project.fps.max(1));
    let mut changes: Vec<Frame> = vec![0, end];
    changes.extend(
        project
            .clips
            .order()
            .iter()
            .filter_map(|&slot| project.clips.timing(slot))
            .map(|timing| timing.start),
    );
    for &slot in project.items.order() {
        if let Some(id) = project.items.id(slot)
            && let Ok(span) = splice_core::edit::span(project, TimelineId::Item(id))
        {
            changes.extend([span.start, span.end]);
        }
    }
    changes.retain(|&frame| (0..=end).contains(&frame));
    changes.sort_unstable();
    changes.dedup();
    let longest = changes.windows(2).map(|w| w[1] - w[0]).max().unwrap_or(0);
    let cuts = project.clips.order().len().saturating_sub(1);
    let minutes = f64::from(end) / fps / 60.0;
    object(vec![
        (
            "cutsPerMinute",
            Json::Float(if minutes > 0.0 {
                (cuts as f64 / minutes * 10.0).round() / 10.0
            } else {
                0.0
            }),
        ),
        (
            "longestStatic",
            Json::Float((f64::from(longest) / fps * 10.0).round() / 10.0),
        ),
        ("safeZone", text(SAFE_ZONE.0)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use splice_core::{edit, package::journal::Source};

    fn codes(project: &Project) -> Vec<(String, Json)> {
        check(project)
            .0
            .into_iter()
            .map(|lint| {
                let code = lint
                    .field("code")
                    .and_then(Json::as_str)
                    .unwrap()
                    .to_owned();
                (code, lint)
            })
            .collect()
    }

    /// T43.1: titles outside the safe zone and zooms past the canvas come back as lints with a
    /// fix and a ready batch. (Items past the end cannot happen: every edit ends them with
    /// the spine, T37.2.)
    #[test]
    fn lints_flag_safe_zone_and_overflowing_zooms() {
        let mut project = crate::tests::sample_project();
        // The sample's hook title sits 134 px from the top of a 1920 px short.
        let found = codes(&project);
        assert!(found.iter().any(|(code, lint)| code == "OUTSIDE_SAFE_ZONE"
            && lint.field("items").and_then(Json::as_array).unwrap()[0].as_str()
                == Some("ovl_hook")));
        let ops = splice_json::parse(
            r#"[{"cmd":"zoom.set","zoom":"ovl_detail","mode":"frame","amount":2}]"#,
        )
        .unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        let found = codes(&project);
        let zoom = found
            .iter()
            .find(|(code, _)| code == "ZOOM_OVERFLOW")
            .expect("overflow");
        let fix = &zoom.1.field("ops").and_then(Json::as_array).unwrap()[0];
        assert_eq!(fix.field("amount").and_then(Json::as_f64), Some(1.11));
        let (_, pacing) = check(&project);
        assert!(
            pacing
                .field("cutsPerMinute")
                .and_then(Json::as_f64)
                .unwrap()
                > 0.0
        );
    }
}
