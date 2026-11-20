//! Scene-graph lints returned with every `edit.apply` and dry run (PLAN.md D18, T43.1): cheap
//! checks of the resulting project that catch what agents miss (text outside the platform's safe
//! zone or over the window, zooms that overflow the canvas, static stretches)
//! with a fix and, where one exists, a suggested op batch. Warnings never block the edit.
//! A delivery `profile` (reel, demo, tutorial) adds Recorder's pacing rules: hook, still
//! stretches, average shot length, tiny shots and duration.
use super::{
    Json, object, text,
    wire::{Args, bad},
};
use splice_core::history::StoreError;
use splice_eval::{Evaluator, FramePlan, Layer};
use splice_model::{ClipId, Frame, ItemKind, Media, MediaId, MediaKind, Project, TimelineId};
use std::path::Path;

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
    // A1: chips that show typing put typed text (passwords, private notes) in the export;
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
        let shown = splice_eval::chips::shown(&scopes);
        if shown & splice_eval::chips::TYPING != 0 {
            // Keep every other category: shortcuts alone when typing was all there was.
            let rest = match shown & !splice_eval::chips::TYPING {
                0 => splice_eval::chips::SHORTCUTS,
                rest => rest,
            };
            let keep = splice_eval::chips::SHOW[usize::from(rest)];
            lints.push(lint(
                "KEYS_SHOW_TYPING",
                &format!(
                    "{id}'s keystroke chips show typing, so typed text appears in the export (agent renders show it as dots)."
                ),
                &[id.to_string()],
                &format!(
                    "Drop typing (keys.set {{show:\"{keep}\"}}), hide the private keys (keys.hide), or turn the chips off (keys.set {{show:\"none\"}})."
                ),
                Some(Json::Array(vec![object(vec![
                    ("cmd", text("keys.set")),
                    ("clip", text(&id.to_string())),
                    ("show", text(keep)),
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

/// A delivery profile (Recorder's `render.lint` profiles, its docs/MCP.md table): how a reel, a
/// product demo or a tutorial is paced. Seconds throughout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Profile {
    pub name: &'static str,
    /// Something happens by this time.
    pub hook: f64,
    /// The longest stretch with nothing happening.
    pub still: f64,
    /// Average shot length range.
    pub asl: [f64; 2],
    /// Length range.
    pub duration: [f64; 2],
    /// A cut-to-cut clip shorter than this is a flash.
    pub tiny: f64,
}

/// Recorder's thresholds (RecorderCore `LintProfile`).
pub(crate) const PROFILES: [Profile; 3] = [
    Profile {
        name: "reel",
        hook: 1.0,
        still: 1.2,
        asl: [0.8, 2.0],
        duration: [15.0, 45.0],
        tiny: 0.25,
    },
    Profile {
        name: "demo",
        hook: 2.0,
        still: 2.5,
        asl: [1.5, 5.0],
        duration: [20.0, 120.0],
        tiny: 0.3,
    },
    Profile {
        name: "tutorial",
        hook: 3.0,
        still: 4.0,
        asl: [2.0, 8.0],
        duration: [30.0, 300.0],
        tiny: 0.4,
    },
];

/// The `profile` argument of `edit.apply` and `view.check`, if any.
pub(crate) fn profile_arg(args: &Json) -> Result<Option<Profile>, StoreError> {
    let Some(name) = args.optional_string("profile")? else {
        return Ok(None);
    };
    PROFILES
        .iter()
        .find(|profile| profile.name == name)
        .copied()
        .map(Some)
        .ok_or_else(|| bad("profile", "reel, demo or tutorial"))
}

/// One profile finding: its lint code, its `view.check` name, its timeline span and the fix.
pub(crate) struct Finding {
    pub code: &'static str,
    pub check: &'static str,
    pub span: [Frame; 2],
    pub message: String,
    pub fix: String,
    pub items: Vec<String>,
    pub ops: Option<Json>,
}

impl Finding {
    fn lint(&self) -> Json {
        lint(
            self.code,
            &self.message,
            &self.items,
            &self.fix,
            self.ops.clone(),
        )
    }
}

/// Timeline ranges where a spine take records input a viewer sees happen (clicks, typing,
/// scrolling; a pointer move alone is not action), from the takes' event logs. A take whose
/// log is missing or unreadable adds nothing.
pub(crate) fn actions(dir: &Path, project: &Project) -> Vec<[Frame; 2]> {
    let mut out = Vec::new();
    for &slot in project.clips.order() {
        let (Some(id), Some(timing), Some(media)) = (
            project.clips.id(slot),
            project.clips.timing(slot),
            project
                .clips
                .data(slot)
                .and_then(|data| data.media)
                .and_then(|media| project.media.iter().find(|m| m.id == media))
                .filter(|media| media.kind == MediaKind::Take),
        ) else {
            continue;
        };
        let fps = media.probe.fps.max(1.0);
        let whole = [0.0, f64::from(media.probe.frames) / fps];
        let Ok(found) = super::media::activity(dir, project, media, whole) else {
            continue;
        };
        let end = timing.start + timing.length;
        let at = |seconds: f64| {
            splice_model::timing::anchor_start(
                project,
                splice_model::Anchor::ClipSource {
                    clip: id,
                    source: (seconds * fps).round() as Frame,
                },
            )
            .ok()
        };
        for &[from, to] in found
            .clicks
            .iter()
            .chain(&found.typing)
            .chain(&found.scroll)
        {
            let (Some(a), Some(b)) = (at(from), at(to)) else {
                continue;
            };
            let [a, b] = [a.max(timing.start), b.min(end)];
            if a < b {
                out.push([a, b]);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Measures `project` against `profile`: the findings and the measures `pacing` reports.
/// `actions` are timeline ranges with recorded input ([`actions`]).
///
/// Something happens at a cut, at a title, overlay or zoom edge, and during recorded clicks,
/// typing and scrolling. Shots are the stretches between cuts and zoom edges (a camera move
/// reframes like a cut). Tiny shots are cut-to-cut clips only: zooms ease, cuts flash.
pub(crate) fn measure(
    project: &Project,
    profile: &Profile,
    actions: &[[Frame; 2]],
) -> (Vec<Finding>, Vec<(&'static str, Json)>) {
    let fps = f64::from(project.fps.max(1));
    let seconds = |frames: Frame| f64::from(frames) / fps;
    let round = |value: f64| (value * 100.0).round() / 100.0;
    let clips: Vec<(String, Frame, Frame)> = project
        .clips
        .order()
        .iter()
        .filter_map(|&slot| {
            let timing = project.clips.timing(slot)?;
            Some((
                project.clips.id(slot)?.to_string(),
                timing.start,
                timing.start + timing.length,
            ))
        })
        .collect();
    let end = clips.iter().map(|c| c.2).max().unwrap_or(0);
    let mut findings = Vec::new();
    if end <= 0 {
        return (findings, Vec::new());
    }
    // Instants where the picture changes, and the shot edges among them.
    let mut instants: Vec<Frame> = clips.iter().map(|c| c.1).filter(|&f| f > 0).collect();
    let mut shots = instants.clone();
    let mut first_item: Option<Frame> = None;
    for &slot in project.items.order() {
        let (Some(id), Some(data)) = (project.items.id(slot), project.items.data(slot)) else {
            continue;
        };
        let Ok(span) = splice_core::edit::span(project, TimelineId::Item(id)) else {
            continue;
        };
        if span.start >= end || span.end <= 0 {
            continue;
        }
        let [from, to] = [span.start.max(0), span.end.min(end)];
        first_item = Some(first_item.map_or(from, |f| f.min(from)));
        instants.extend([from, to]);
        if matches!(data.kind, ItemKind::Zoom(_)) {
            shots.extend([from, to]);
        }
    }
    let first_action = actions.iter().map(|a| a[0]).filter(|&f| f < end).min();
    let hook = [
        instants.iter().copied().filter(|&f| f > 0).min(),
        first_item,
        first_action,
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(end);
    if seconds(hook) > profile.hook + 1e-9 {
        // Keep 0.3 s before the first thing that happens.
        let lead = hook - (0.3 * fps).round() as Frame;
        findings.push(Finding {
            code: "HOOK_LATE",
            check: "hook",
            span: [0, hook],
            message: format!(
                "Nothing happens until {:.1} s; a {} hooks by {:.1} s.",
                seconds(hook),
                profile.name,
                profile.hook
            ),
            fix: "Cut the lead-in (ops), or open on a title or a zoom.".into(),
            items: Vec::new(),
            ops: (seconds(lead) > 0.1).then(|| {
                Json::Array(vec![object(vec![
                    ("cmd", text("cut.range")),
                    ("from", Json::Signed(0)),
                    ("to", Json::Signed(i64::from(lead))),
                ])])
            }),
        });
    }
    // Still stretches: the gaps between happenings.
    let mut busy: Vec<[Frame; 2]> = instants.iter().map(|&f| [f, f]).collect();
    busy.extend(actions.iter().map(|&[a, b]| [a.max(0), b.min(end)]));
    busy.push([end, end]);
    busy.sort_unstable();
    let mut stills: Vec<[Frame; 2]> = Vec::new();
    let mut reach = 0;
    for &[a, b] in &busy {
        if a > reach {
            stills.push([reach, a]);
        }
        reach = reach.max(b);
    }
    let limit = (profile.still * fps).round() as Frame;
    let longest = stills.iter().map(|s| s[1] - s[0]).max().unwrap_or(0);
    let long: Vec<[Frame; 2]> = stills
        .iter()
        .copied()
        .filter(|s| s[1] - s[0] > limit)
        .collect();
    if let (Some(&[first, _]), Some(&[_, last])) = (long.first(), long.last()) {
        let worst = long
            .iter()
            .copied()
            .max_by_key(|s| s[1] - s[0])
            .unwrap_or([first, last]);
        // Latest first, so each cut leaves the earlier frames where they are.
        let ops = long
            .iter()
            .rev()
            .map(|&[a, b]| {
                object(vec![
                    ("cmd", text("cut.range")),
                    ("from", Json::Signed(i64::from(a + limit))),
                    ("to", Json::Signed(i64::from(b))),
                ])
            })
            .collect();
        findings.push(Finding {
            code: "LONG_STILL",
            check: "still",
            span: [first, last],
            message: format!(
                "{} stretch(es) with nothing happening for over {:.1} s, the longest {:.1} s from {:.1} s.",
                long.len(),
                profile.still,
                seconds(worst[1] - worst[0]),
                seconds(worst[0])
            ),
            fix: "Cut each down to the limit (ops), speed it up, or add a zoom or title inside it."
                .into(),
            items: Vec::new(),
            ops: Some(Json::Array(ops)),
        });
    }
    shots.extend([0, end]);
    shots.retain(|&f| (0..=end).contains(&f));
    shots.sort_unstable();
    shots.dedup();
    let count = shots.len().saturating_sub(1).max(1);
    let asl = seconds(end) / count as f64;
    let [low, high] = profile.asl;
    if asl < low - 1e-9 || asl > high + 1e-9 {
        findings.push(Finding {
            code: "SHOT_LENGTH",
            check: "shotLength",
            span: [0, end],
            message: format!(
                "Shots average {asl:.1} s ({count} shots); a {} wants {low:.1}–{high:.1} s.",
                profile.name
            ),
            fix: if asl > high {
                "Add cuts or zooms: cut.pauses, intent.pace, intent.autoZoom.".into()
            } else {
                "Fewer, longer shots: remove the shortest clips or zooms.".into()
            },
            items: Vec::new(),
            ops: None,
        });
    }
    let tiny = (profile.tiny * fps).round() as Frame;
    let flashes: Vec<&(String, Frame, Frame)> = clips
        .iter()
        .filter(|(_, from, to)| to - from < tiny)
        .collect();
    if let (Some(first), Some(last)) = (flashes.first(), flashes.last()) {
        findings.push(Finding {
            code: "TINY_SHOT",
            check: "tinyShot",
            span: [first.1, last.2],
            message: format!(
                "{} clip(s) shorter than {:.2} s flash by.",
                flashes.len(),
                profile.tiny
            ),
            fix: "Delete them (item.delete) or extend them past the limit.".into(),
            items: flashes.iter().map(|c| c.0.clone()).collect(),
            ops: None,
        });
    }
    let length = seconds(end);
    let [shortest, longest_allowed] = profile.duration;
    if length < shortest - 1e-9 || length > longest_allowed + 1e-9 {
        let over = length > longest_allowed;
        findings.push(Finding {
            code: "DURATION",
            check: "duration",
            span: [0, end],
            message: format!(
                "The edit runs {length:.1} s; a {} runs {shortest:.0}–{longest_allowed:.0} s.",
                profile.name
            ),
            fix: if over {
                format!("Tighten it with intent.fitDuration {{seconds:{longest_allowed}}} (ops).")
            } else {
                "Add material, or pick the profile this piece is.".into()
            },
            items: Vec::new(),
            ops: over.then(|| {
                Json::Array(vec![object(vec![
                    ("cmd", text("intent.fitDuration")),
                    ("seconds", Json::Float(longest_allowed)),
                ])])
            }),
        });
    }
    let measures = vec![
        ("profile", text(profile.name)),
        ("seconds", Json::Float(round(length))),
        ("hookSec", Json::Float(round(seconds(hook)))),
        ("aslSec", Json::Float(round(asl))),
        ("shots", Json::Signed(count as i64)),
        ("longestStill", Json::Float(round(seconds(longest)))),
    ];
    (findings, measures)
}

/// [`check`] plus a profile's lints and measures (`edit.apply {profile}`); `LONG_STILL`
/// replaces the generic `LONG_STATIC` then.
pub(crate) fn check_profiled(
    project: &Project,
    dir: &Path,
    profile: Option<Profile>,
) -> (Vec<Json>, Json) {
    let (mut lints, mut pacing) = check(project);
    let Some(profile) = profile else {
        return (lints, pacing);
    };
    let (findings, measures) = measure(project, &profile, &actions(dir, project));
    lints.retain(|lint| lint.field("code").and_then(Json::as_str) != Some("LONG_STATIC"));
    lints.extend(findings.iter().map(Finding::lint));
    if let Json::Object(fields) = &mut pacing {
        fields.extend(measures.into_iter().map(|(k, v)| (k.to_owned(), v)));
    }
    (lints, pacing)
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
        // Categories: typing anywhere in the set is flagged, and the fix keeps the rest.
        for (show, keep) in [
            ("all", Some("shortcuts+navigation+modifiers")),
            ("typing+navigation", Some("navigation")),
            ("typing", Some("shortcuts")),
            ("shortcuts+navigation+modifiers", None),
        ] {
            let ops = splice_json::parse(&format!(
                r#"[{{"cmd":"keys.set","clip":"clp_take","show":"{show}"}}]"#
            ))
            .unwrap();
            edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
            let found = codes(&project);
            let fix = found
                .iter()
                .find(|(code, _)| code == "KEYS_SHOW_TYPING")
                .map(|(_, lint)| lint.field("ops").unwrap().as_array().unwrap()[0].clone());
            assert_eq!(
                fix.as_ref()
                    .and_then(|op| op.field("show"))
                    .and_then(Json::as_str),
                keep,
                "{show}"
            );
            if let Some(op) = fix {
                edit::apply(&mut project, &[op], Source::Agent, false).unwrap();
                assert!(!codes(&project).iter().any(|(c, _)| c == "KEYS_SHOW_TYPING"));
            }
        }
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

    fn bare_sample() -> Project {
        let mut bare = crate::tests::sample_project();
        let slots: Vec<_> = bare.items.order().to_vec();
        for slot in slots {
            bare.items.remove(slot);
        }
        bare
    }

    /// Recorder parity (render.lint profiles): a reel with nothing happening for 4 s and 7 s
    /// shots is late, still and slow, with a cut batch for each; recorded clicks and typing
    /// count as something happening, and a tutorial is judged by its own thresholds.
    #[test]
    fn profiles_measure_hook_still_stretches_shot_length_tiny_shots_and_duration() {
        let mut project = bare_sample();
        // Cuts at 120 and 540 of a 660-frame (22 s) edit at 30 fps, nothing else.
        let reel = PROFILES[0];
        let (findings, measures) = measure(&project, &reel, &[]);
        let hook = findings
            .iter()
            .find(|f| f.code == "HOOK_LATE")
            .expect("late hook");
        assert_eq!(hook.span, [0, 120]);
        let ops = hook.ops.as_ref().unwrap().as_array().unwrap();
        assert_eq!(ops[0].field("to").and_then(Json::as_i64), Some(111));
        let still = findings
            .iter()
            .find(|f| f.code == "LONG_STILL")
            .expect("still stretches");
        // Three stretches, cut latest first down to 1.2 s (36 frames) each.
        let cuts = still.ops.as_ref().unwrap().as_array().unwrap();
        let from = |n: usize| cuts[n].field("from").and_then(Json::as_i64).unwrap();
        assert_eq!((cuts.len(), from(0), from(2)), (3, 576, 36));
        assert!(findings.iter().find(|f| f.code == "SHOT_LENGTH").is_some());
        assert!(
            findings.iter().find(|f| f.code == "DURATION").is_none()
                && findings.iter().find(|f| f.code == "TINY_SHOT").is_none()
        );
        let get = |key: &str| {
            measures
                .iter()
                .find(|(name, _)| *name == key)
                .and_then(|(_, value)| value.as_f64())
                .unwrap()
        };
        assert_eq!(
            (
                get("hookSec"),
                get("aslSec"),
                get("longestStill"),
                get("seconds")
            ),
            (4.0, 7.33, 14.0, 22.0)
        );
        // A 5-frame clip flashes by; recorded input fills the still stretches.
        let ops = splice_json::parse(
            r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"a"},
                {"cmd":"item.split","item":"$a.right","at":305}]"#,
        )
        .unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        let actions = [[30, 60], [150, 400], [420, 500]];
        let (findings, _) = measure(&project, &PROFILES[2], &actions);
        let codes: Vec<&str> = findings.iter().map(|f| f.code).collect();
        // Every gap is at most 4 s; the shots average within 2–8 s; 22 s is short.
        assert_eq!(codes, ["TINY_SHOT", "DURATION"], "{codes:?}");
        let tiny = findings.iter().find(|f| f.code == "TINY_SHOT").unwrap();
        assert_eq!((tiny.span, tiny.items.len()), ([300, 305], 1));
        assert!(
            findings
                .iter()
                .find(|f| f.code == "DURATION")
                .unwrap()
                .ops
                .is_none()
        );
        // Input all along: hooked and never still. Past the longest length: fit it.
        let short = Profile {
            duration: [5.0, 10.0],
            ..reel
        };
        let (findings, _) = measure(&project, &short, &[[0, 660]]);
        assert!(
            findings.iter().find(|f| f.code == "HOOK_LATE").is_none()
                && findings.iter().find(|f| f.code == "LONG_STILL").is_none()
        );
        let fit = findings
            .iter()
            .find(|f| f.code == "DURATION")
            .unwrap()
            .ops
            .as_ref()
            .unwrap();
        assert_eq!(
            fit.as_array().unwrap()[0]
                .field("seconds")
                .and_then(Json::as_f64),
            Some(10.0)
        );
    }

    /// The recorded input `profile` reads: clicks and typing of a spine take on the timeline,
    /// pointer moves alone not; edit.apply's lints carry the profile's measures, and LONG_STILL
    /// replaces LONG_STATIC.
    #[test]
    fn profile_actions_come_from_the_take_event_log() {
        let project = bare_sample();
        let dir = crate::tests::TestDirectory::new("profile-actions");
        let take = dir.0.join("takes/med_screen");
        std::fs::create_dir_all(&take).unwrap();
        std::fs::write(
            take.join("events.json"),
            r#"{"source":{"rect":{"width":640,"height":400}},"events":[
                {"t":0.5,"k":"move","x":0.1,"y":0.1},
                {"t":1.0,"k":"down","x":0.2,"y":0.2,"b":0},{"t":1.05,"k":"up","x":0.2,"y":0.2,"b":0},
                {"t":2.0,"k":"key","keyCode":4,"mods":0}]}"#,
        )
        .unwrap();
        let actions = actions(&dir.0, &project);
        // The take starts at frame 120: the click at 1 s is frame 150, the key at 2 s frame 180.
        assert_eq!(actions.len(), 2, "{actions:?}");
        assert_eq!((actions[0][0], actions[1][0]), (150, 180));
        let (lints, pacing) = check_profiled(&project, &dir.0, Some(PROFILES[1]));
        let codes: Vec<&str> = lints
            .iter()
            .filter_map(|lint| lint.field("code").and_then(Json::as_str))
            .collect();
        assert!(
            codes.contains(&"LONG_STILL") && !codes.contains(&"LONG_STATIC"),
            "{codes:?}"
        );
        assert_eq!(pacing.field("profile").and_then(Json::as_str), Some("demo"));
        assert_eq!(pacing.field("hookSec").and_then(Json::as_f64), Some(4.0));
        // Without a profile nothing changes.
        let (lints, pacing) = check_profiled(&project, &dir.0, None);
        assert!(pacing.field("profile").is_none());
        assert!(
            lints
                .iter()
                .any(|l| l.field("code").and_then(Json::as_str) == Some("LONG_STATIC"))
        );
    }
}
