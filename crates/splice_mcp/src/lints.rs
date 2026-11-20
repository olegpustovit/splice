//! Scene-graph lints returned with every `edit.apply` and dry run (PLAN.md D18, T43.1): cheap
//! checks of the resulting project that catch what agents miss (text outside the platform's safe
//! zone or over the window, zooms that overflow the canvas, static stretches)
//! with a fix and, where one exists, a suggested op batch. Warnings never block the edit.
use super::{Json, object, text};
use splice_eval::{Evaluator, FramePlan, Layer};
use splice_model::{ClipId, Frame, ItemKind, Media, MediaId, Project, TimelineId};

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
                    Layer::Title {
                        item, bounds, look, ..
                    } if *item == id => Some(look.placed(*bounds)),
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
                            None,
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
                            None,
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
                        Some(Json::Array(vec![object(vec![
                            ("cmd", text("zoom.set")),
                            ("zoom", text(&id.to_string())),
                            ("amount", Json::Float(safe)),
                        ])])),
                    ));
                }
                // A following zoom needs a take with a recorded pointer under it (T46.2);
                // anywhere else the camera holds whatever framing it had.
                if zoom.follow && !follows_a_pointer(project, [from, to]) {
                    lints.push(lint(
                        "ZOOM_FOLLOWS_NOTHING",
                        &format!(
                            "{id} follows the cursor, but no screen take with recorded pointer events plays under it; the camera holds its framing."
                        ),
                        &[id.to_string()],
                        "Move the zoom over a screen take, or give it a point: zoom.set {focus:{x, y}}.",
                        None,
                    ));
                }
            }
            _ => {}
        }
    }
    // A1: chips that show every key put typed text (passwords, private notes) in the export;
    // renders an agent sees redact them, so only this says so.
    for &slot in project.clips.order() {
        let (Some(id), Some(data)) = (project.clips.id(slot), project.clips.data(slot)) else {
            continue;
        };
        if data.enabled.keys != Some(true) {
            continue;
        }
        let scopes = [
            project.effects.as_slice(),
            &project.spine.effects,
            &data.appearance.effects,
        ];
        let shortcuts =
            splice_eval::resolve_effect(&scopes, "keystrokes").is_some_and(|(enabled, params)| {
                !enabled
                    || params.iter().any(|param| {
                        param.name == "show"
                            && param.value == splice_model::EffectValue::Text("shortcuts".into())
                    })
            });
        if !shortcuts {
            lints.push(lint(
                "KEYS_SHOW_TYPING",
                &format!(
                    "{id}'s keystroke chips show every key, so typed text appears in the export (agent renders show it as dots)."
                ),
                &[id.to_string()],
                "Show only shortcuts (keys.set {show:\"shortcuts\"}), hide the private keys (keys.hide), or turn the chips off (keys.set {show:\"none\"}).",
                Some(Json::Array(vec![object(vec![
                    ("cmd", text("keys.set")),
                    ("clip", text(&id.to_string())),
                    ("show", text("shortcuts")),
                ])])),
            ));
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
            None,
        ));
    }
    (lints, pacing)
}

/// The canvas shape nearest a `width` × `height` picture, as `project.set` names it
/// (`Aspect::nearest`, the same rule as `project.set {aspect:"auto"}`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn nearest_aspect(width: u32, height: u32) -> &'static str {
    splice_model::Aspect::nearest(width, height).label()
}

/// `TAKE_ASPECT` (A6): a recorded picture whose shape is not the canvas's does not fill it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn take_fit(project: &Project, media: &Media, clip: Option<ClipId>) -> Option<Json> {
    let [width, height] = [media.probe.width, media.probe.height];
    let [cw, ch] = project.size;
    let ratio = |w: u32, h: u32| f64::from(w.max(1)) / f64::from(h.max(1));
    if (ratio(width, height) / ratio(cw, ch)).ln().abs() <= 0.02_f64.ln_1p() {
        return None;
    }
    let nearest = nearest_aspect(width, height);
    let canvas = nearest_aspect(cw, ch);
    let items: Vec<String> = clip.iter().map(ToString::to_string).collect();
    let mut fix = format!(
        "Match the canvas to the take with project.set {{aspect:\"{nearest}\"}}, or fill the canvas with clip.crop {{aspect:\"canvas\"}} (cuts the take's edges)."
    );
    let ops = if nearest != canvas {
        Some(Json::Array(vec![object(vec![
            ("cmd", text("project.set")),
            ("aspect", text(nearest)),
        ])]))
    } else if let Some(clip) = clip {
        fix = format!(
            "The nearest canvas shape is already {canvas}: fill it with clip.crop {{aspect:\"canvas\"}} (cuts the take's edges), or keep the margins."
        );
        Some(Json::Array(vec![object(vec![
            ("cmd", text("clip.crop")),
            ("items", Json::Array(vec![text(&clip.to_string())])),
            ("aspect", text("canvas")),
        ])]))
    } else {
        None
    };
    Some(lint(
        "TAKE_ASPECT",
        &format!(
            "The take is {width}×{height} (nearest {nearest}) but the canvas is {cw}×{ch} ({canvas}): it will not fill the canvas."
        ),
        &items,
        &fix,
        ops,
    ))
}

/// `MIC_SILENT`: a take whose microphone recorded nothing audible.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn mic_silent(peak_db: f64, media: MediaId) -> Json {
    lint(
        "MIC_SILENT",
        &format!("The microphone track of {media} peaks at {peak_db:.0} dBFS: it is silent."),
        &[],
        "Check the microphone (record.start {dryRun:true} lists devices; pass mic:\"<id>\") and record again, or remove the mic component.",
        None,
    )
}

/// Whether a spine take with a recorded event log plays somewhere in `[from, to)`.
fn follows_a_pointer(project: &Project, [from, to]: [Frame; 2]) -> bool {
    project.clips.order().iter().any(|&slot| {
        let (Some(timing), Some(data)) = (project.clips.timing(slot), project.clips.data(slot))
        else {
            return false;
        };
        timing.start < to
            && timing.start + timing.length > from
            && data
                .media
                .and_then(|id| project.media.iter().find(|m| m.id == id))
                .and_then(|m| m.take.as_ref())
                .is_some_and(|take| take.events.is_some())
    })
}

fn lint(code: &str, message: &str, items: &[String], fix: &str, ops: Option<Json>) -> Json {
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
    if let Some(ops) = ops {
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

    /// Review #17: a title over the recorded window and a long stretch with nothing changing
    /// are flagged; the sample as shipped has neither.
    #[test]
    fn lints_flag_text_over_the_window_and_long_static_stretches() {
        let has = |project: &Project, code: &str| {
            check(project)
                .0
                .iter()
                .any(|lint| lint.field("code").and_then(Json::as_str) == Some(code))
        };
        let mut project = crate::tests::sample_project();
        assert!(!has(&project, "TEXT_OVER_WINDOW") && !has(&project, "LONG_STATIC"));
        let ops = splice_json::parse(
            r#"[{"cmd":"title.add","text":"Look here","at":200,"length":60,"ref":"t"},
                {"cmd":"param.set","target":"$t","path":"transform.y","value":0.4}]"#,
        )
        .unwrap();
        splice_core::edit::apply(
            &mut project,
            ops.as_array().unwrap(),
            splice_core::package::journal::Source::Agent,
            false,
        )
        .unwrap();
        let mut bare = crate::tests::sample_project();
        let slots: Vec<_> = bare.items.order().to_vec();
        for slot in slots {
            bare.items.remove(slot);
        }
        // Only the cuts at 120 and 540 remain: 14 s of take with nothing changing.
        assert!(has(&bare, "LONG_STATIC"));
        assert!(has(&project, "TEXT_OVER_WINDOW"));
    }

    /// A1: chips that show every key are flagged with a batch that keeps only shortcuts;
    /// shortcuts only or chips off are not.
    #[test]
    fn lints_flag_keystroke_chips_that_show_typed_text() {
        let mut project = crate::tests::sample_project();
        // The 0.3 sample keeps its chips on (nothing migrates silently).
        let found = codes(&project);
        let (_, lint) = found
            .iter()
            .find(|(code, _)| code == "KEYS_SHOW_TYPING")
            .expect("typed text flagged");
        let fix = lint.field("ops").unwrap().clone();
        edit::apply(&mut project, fix.as_array().unwrap(), Source::Agent, false).unwrap();
        assert!(!codes(&project).iter().any(|(c, _)| c == "KEYS_SHOW_TYPING"));
        let ops =
            splice_json::parse(r#"[{"cmd":"keys.set","clip":"clp_take","show":"all"}]"#).unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        assert!(codes(&project).iter().any(|(c, _)| c == "KEYS_SHOW_TYPING"));
        let ops =
            splice_json::parse(r#"[{"cmd":"keys.set","clip":"clp_take","show":"none"}]"#).unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        assert!(!codes(&project).iter().any(|(c, _)| c == "KEYS_SHOW_TYPING"));
    }

    /// T46.2: a zoom that follows the cursor over a take without recorded pointer events (or
    /// over no take) is flagged; over a take with an event log it is not.
    #[test]
    fn lints_flag_a_following_zoom_with_no_pointer_under_it() {
        let follows = |project: &Project| {
            check(project).0.iter().any(|lint| {
                lint.field("code").and_then(Json::as_str) == Some("ZOOM_FOLLOWS_NOTHING")
            })
        };
        let mut project = crate::tests::sample_project();
        let ops =
            splice_json::parse(r#"[{"cmd":"zoom.set","zoom":"ovl_detail","focus":"cursor"}]"#)
                .unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        // Over the sample's take, which declares an event log: fine.
        assert!(!follows(&project));
        // Over the opener (not a take): flagged.
        let mut opener = project.clone();
        let ops =
            splice_json::parse(r#"[{"cmd":"zoom.add","at":10,"length":60,"focus":"cursor"}]"#)
                .unwrap();
        edit::apply(&mut opener, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        assert!(follows(&opener));
        // A take without an event log: flagged.
        let take = project
            .media
            .iter_mut()
            .find(|m| m.id == "med_screen".parse().unwrap())
            .unwrap();
        take.take.as_mut().unwrap().events = None;
        assert!(follows(&project));
    }
}
