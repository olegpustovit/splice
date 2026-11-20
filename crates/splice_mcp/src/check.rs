//! `view.check` (PLAN.md T43.2, D18): finds what an agent would not see in a few frames. The
//! model checks (camera jumps, pictures pushed off the canvas) run on every platform from the
//! evaluator; the pixel checks (black and frozen spans, transitions that disturb the backdrop)
//! render the export composition on macOS and return a contact sheet of the flagged frames.
use super::{Json, object, text, wire::*};
use splice_core::history::{Store, StoreError};
use splice_eval::{Evaluator, Layer};
use splice_model::{Frame, Project, TimelineId};

/// Checks a caller may ask for; all by default.
pub(crate) const CHECKS: [&str; 5] = ["camera", "offCanvas", "black", "frozen", "bleed"];
/// A camera change per frame beyond this reads as a cut, not a move.
const CAMERA_STEP: f64 = 0.06;
/// A picture with more than this share of itself off the canvas is flagged.
const OFF_CANVAS: f64 = 0.25;

pub(super) fn view_check(
    store: &Store,
    args: &Json,
    content: &mut Vec<Json>,
) -> Result<Json, StoreError> {
    let project = store.project();
    let end = super::timeline::duration(project)?;
    let from = args.frame("from")?.unwrap_or(0).clamp(0, end);
    let to = args.frame("to")?.unwrap_or(end).clamp(from, end);
    let wanted: Vec<String> = match args.field("checks") {
        None => CHECKS.iter().map(|c| (*c).to_owned()).collect(),
        Some(value) => {
            let list = value
                .as_array()
                .ok_or_else(|| bad("checks", "an array of check names"))?;
            let mut names = Vec::with_capacity(list.len());
            for name in list {
                match name.as_str().filter(|n| CHECKS.contains(n)) {
                    Some(name) => names.push(name.to_owned()),
                    None => {
                        return Err(bad(
                            "checks",
                            "names from camera, offCanvas, black, frozen, bleed",
                        ));
                    }
                }
            }
            names
        }
    };
    let profile = super::lints::profile_arg(args)?;
    let wants = |name: &str| wanted.iter().any(|w| w == name);
    // Zooms that follow the cursor are checked on the path the renderer draws (T46.2).
    let pointers = super::media::pointers(store.path(), project);
    let evaluator = Evaluator::prepare_with(project, &pointers).map_err(|error| {
        failure(
            "INVARIANT",
            &error.to_string(),
            "Fix the project with edit.apply first.",
        )
    })?;
    let mut issues = model_issues(
        project,
        &evaluator,
        [from, to],
        wants("camera"),
        wants("offCanvas"),
    );
    // A delivery profile judges the whole edit (its pacing has no meaning for a range).
    let pacing = profile.map(|profile| {
        let actions = super::lints::actions(store.path(), project);
        let (findings, measures) = super::lints::measure(project, &profile, &actions);
        for finding in findings {
            let mut found = issue(finding.check, finding.span, &finding.message, &finding.fix);
            if let (Json::Object(fields), Some(ops)) = (&mut found, finding.ops) {
                fields.push(("ops".into(), ops));
            }
            issues.push(found);
        }
        object(measures)
    });
    let pixels = wants("black") || wants("frozen") || wants("bleed");
    #[cfg(target_os = "macos")]
    let (issues, sheet, offline) = {
        let mut issues = issues;
        let (sheet, offline) = if pixels {
            pixel_issues(store, &evaluator, [from, to], &wants, &mut issues, content)?
        } else {
            (None, Vec::new())
        };
        (issues, sheet, offline)
    };
    #[cfg(not(target_os = "macos"))]
    let (sheet, offline): (Option<String>, Vec<(&str, Json)>) = {
        let _ = (store, &content);
        (None, Vec::new())
    };
    let mut fields = vec![
        ("revision", rev(project.rev)),
        ("issues", Json::Array(issues)),
        ("pixels", Json::Bool(pixels && cfg!(target_os = "macos"))),
    ];
    if let Some(pacing) = pacing {
        fields.push(("pacing", pacing));
    }
    if let Some(sheet) = sheet {
        fields.push(("sheet", text(&sheet)));
    }
    fields.extend(offline);
    Ok(object(fields))
}

fn issue(check: &str, [from, to]: [Frame; 2], message: &str, fix: &str) -> Json {
    object(vec![
        ("check", text(check)),
        ("from", Json::Signed(i64::from(from))),
        ("to", Json::Signed(i64::from(to))),
        ("message", text(message)),
        ("fix", text(fix)),
    ])
}

/// Camera jumps and off-canvas pictures from the evaluated frames (every frame of the range).
pub(crate) fn model_issues(
    project: &Project,
    evaluator: &Evaluator,
    [from, to]: [Frame; 2],
    camera: bool,
    off_canvas: bool,
) -> Vec<Json> {
    let mut plan = evaluator.buffer();
    let [cw, ch] = project.size.map(f64::from);
    let mut issues = Vec::new();
    let mut previous: Option<(Frame, splice_eval::View, Option<splice_model::ClipId>)> = None;
    let mut off: Option<(Frame, Frame, TimelineId)> = None;
    for at in from..to {
        evaluator.frame(at, &mut plan);
        let picture = plan.layers.iter().find_map(|layer| match layer {
            Layer::Picture(p) if matches!(p.item, TimelineId::Clip(_)) => Some(*p),
            _ => None,
        });
        let Some(picture) = picture else {
            previous = None;
            continue;
        };
        if camera {
            let view = picture.view;
            // A cut legitimately changes the framing; only a jump within one clip counts.
            if let Some((frame, last, clip)) = previous
                && frame + 1 == at
                && clip == plan.clip
            {
                let [ax, ay, aw, _] = last.viewport();
                let [bx, by, bw, _] = view.viewport();
                let moved = (bx - ax).abs().max((by - ay).abs()).max((bw - aw).abs());
                if moved > CAMERA_STEP {
                    issues.push(issue(
                        "camera",
                        [at - 1, at + 1],
                        &format!("The camera jumps {:.0} % of the frame in one frame.", moved * 100.0),
                        "Use a spring other than instant on the zoom, or move the zoom boundary off this frame.",
                    ));
                }
            }
            previous = Some((at, view, plan.clip));
        }
        if off_canvas {
            let placed = picture.look.placed(picture.destination);
            let [x, y, w, h] = [placed.x, placed.y, placed.w, placed.h];
            let inside =
                ((x + w).min(cw) - x.max(0.0)).max(0.0) * ((y + h).min(ch) - y.max(0.0)).max(0.0);
            let outside = 1.0 - inside / (w * h).max(1e-9);
            match (&mut off, outside > OFF_CANVAS) {
                (Some(span), true) if span.1 == at => span.1 = at + 1,
                (None, true) => off = Some((at, at + 1, picture.item)),
                (Some(span), _) => {
                    issues.push(off_issue(*span));
                    off = (outside > OFF_CANVAS).then_some((at, at + 1, picture.item));
                }
                (None, false) => {}
            }
        }
    }
    if let Some(span) = off {
        issues.push(off_issue(span));
    }
    issues
}

fn off_issue((from, to, item): (Frame, Frame, TimelineId)) -> Json {
    issue(
        "offCanvas",
        [from, to],
        &format!("More than a quarter of {item} is off the canvas."),
        "Check its transform (param.set transform.x/y/scale) or lower the zoom.",
    )
}

/// Runs of consecutive sampled frames where `hit` holds, as `[first, last + stride)` spans of
/// at least `min` frames. `samples` are frame numbers `stride` apart.
#[cfg(any(target_os = "macos", test))]
fn spans(samples: &[(Frame, bool)], stride: Frame, min: Frame) -> Vec<[Frame; 2]> {
    let mut out = Vec::new();
    let mut open: Option<Frame> = None;
    for (index, &(at, hit)) in samples.iter().enumerate() {
        match (open, hit) {
            (None, true) => open = Some(at),
            (Some(start), false) => {
                if at - start >= min {
                    out.push([start, at]);
                }
                open = None;
            }
            _ => {}
        }
        if index + 1 == samples.len()
            && let Some(start) = open
            && at + stride - start >= min
        {
            out.push([start, at + stride]);
        }
    }
    out
}

/// Black and frozen spans and backdrop bleed from rendered frames at 5 per second, with the
/// contact sheet and the offline media drawn as placeholders.
#[cfg(target_os = "macos")]
fn pixel_issues(
    store: &Store,
    evaluator: &Evaluator,
    [from, to]: [Frame; 2],
    wants: &dyn Fn(&str) -> bool,
    issues: &mut Vec<Json>,
    content: &mut Vec<Json>,
) -> Result<(Option<String>, Vec<(&'static str, Json)>), StoreError> {
    use splice_av::Composition;
    let project = store.project();
    let render = |error: &dyn std::fmt::Display| {
        failure(
            "FRAME_FAILED",
            &error.to_string(),
            "Retry with a smaller range.",
        )
    };
    let composition =
        Composition::prepare_for_agent(project, store.path()).map_err(|e| render(&e))?;
    let size = composition.size();
    let fps = project.fps.max(1) as Frame;
    // ponytail: at most 400 sampled frames (80 s at 5/s); a longer range samples more sparsely.
    // Upgrade: a GPU-side reduction pass when checks need every frame.
    let stride = (fps / 5).max(1).max((to - from) / 400 + 1);
    let mut pixels = vec![0u8; size[0] as usize * size[1] as usize * 4];
    let mut previous = pixels.clone();
    let (mut black, mut frozen) = (Vec::new(), Vec::new());
    for at in (from..to).step_by(stride as usize) {
        composition
            .read_frame(at, &mut pixels)
            .map_err(|e| render(&e))?;
        black.push((at, mean_luma(&pixels) < 0.03));
        frozen.push((at, at > from && mean_difference(&previous, &pixels) < 0.002));
        previous.copy_from_slice(&pixels);
    }
    let mut flagged = Vec::new();
    for (check, samples, min, message, fix) in [
        (
            "black",
            &black,
            fps / 2,
            "The picture is black.",
            "Cut the span (cut.range) or check the clip's media and fades.",
        ),
        (
            "frozen",
            &frozen,
            3 * fps,
            "Nothing on screen changes.",
            "Cut or speed up the span (cut.pauses with basis activity, intent.pace), or add a zoom.",
        ),
    ] {
        if !wants(check) {
            continue;
        }
        for span in spans(samples, stride, min) {
            issues.push(issue(check, span, message, fix));
            if flagged.len() < 12 {
                composition
                    .read_frame(span[0], &mut pixels)
                    .map_err(|e| render(&e))?;
                flagged.push(pixels.clone());
            }
        }
    }
    if wants("bleed") {
        bleed(
            project,
            evaluator,
            &composition,
            [from, to],
            &mut pixels,
            issues,
        )?;
    }
    let offline = super::frames::offline_fields(&composition);
    if flagged.is_empty() {
        return Ok((None, offline));
    }
    let (sheet, sheet_size) = super::sheet::tile(&flagged, size, 360);
    let png = splice_av::encode_frame_png(&sheet, sheet_size, 0).map_err(|e| render(&e))?;
    let file = store
        .path()
        .join("cache")
        .join(format!("check-r{}.png", project.rev));
    std::fs::create_dir_all(store.path().join("cache"))
        .and_then(|()| std::fs::write(&file, &png))
        .map_err(|e| render(&e))?;
    content.push(object(vec![
        ("type", text("image")),
        ("mimeType", text("image/png")),
        ("data", Json::String(super::frames::base64(&png))),
    ]));
    Ok((Some(format!("file://{}", file.display())), offline))
}

/// Inside each transition, the backdrop (a corner patch) must stay as it was just before.
#[cfg(target_os = "macos")]
fn bleed(
    project: &Project,
    evaluator: &Evaluator,
    composition: &splice_av::Composition,
    [from, to]: [Frame; 2],
    pixels: &mut [u8],
    issues: &mut Vec<Json>,
) -> Result<(), StoreError> {
    let size = composition.size();
    for &slot in project.clips.order() {
        let (Some(id), Some(timing)) = (project.clips.id(slot), project.clips.timing(slot)) else {
            continue;
        };
        let Some(transition) = evaluator.transition(id) else {
            continue;
        };
        let length = transition.length;
        let [start, end] = [
            timing.start - length / 2,
            timing.start + length - length / 2,
        ];
        if start < from + 1 || end > to {
            continue;
        }
        let read = |at: Frame, pixels: &mut [u8]| {
            composition
                .read_frame(at, pixels)
                .map_err(|error| failure("FRAME_FAILED", &error.to_string(), "Retry."))
        };
        // Only a windowed clip has a backdrop corner to protect.
        let mut plan = evaluator.buffer();
        evaluator.frame(start - 1, &mut plan);
        if !matches!(plan.layers.first(), Some(Layer::Background { .. })) {
            continue;
        }
        read(start - 1, pixels)?;
        let before = corner(pixels, size);
        let mid = (start + end) / 2;
        read(mid, pixels)?;
        let during = corner(pixels, size);
        let difference = (0..3)
            .map(|c| (before[c] - during[c]).abs())
            .fold(0.0, f64::max);
        if difference > 6.0 / 255.0 {
            issues.push(issue(
                "bleed",
                [start, end],
                &format!("The transition into {id} changes the backdrop."),
                "Use a content-only transition (dissolve, push, slide, whip, swipe) between windowed clips.",
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn corner(pixels: &[u8], [w, _]: [u32; 2]) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for y in 1..5usize {
        for x in 1..5usize {
            let i = (y * w as usize + x) * 4;
            for c in 0..3 {
                sum[c] += f64::from(pixels[i + c]) / 255.0 / 16.0;
            }
        }
    }
    sum
}

#[cfg(target_os = "macos")]
fn mean_luma(pixels: &[u8]) -> f64 {
    let mut sum = 0u64;
    for p in pixels.chunks_exact(4).step_by(7) {
        sum += u64::from(p[0]) * 54 + u64::from(p[1]) * 183 + u64::from(p[2]) * 19;
    }
    sum as f64 / (pixels.len() / 4 / 7).max(1) as f64 / 256.0 / 255.0
}

#[cfg(target_os = "macos")]
fn mean_difference(a: &[u8], b: &[u8]) -> f64 {
    let mut sum = 0u64;
    let mut count = 0u64;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)).step_by(5) {
        for c in 0..3 {
            sum += u64::from(x[c].abs_diff(y[c]));
        }
        count += 3;
    }
    sum as f64 / count.max(1) as f64 / 255.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use splice_core::{edit, package::journal::Source};

    #[test]
    fn spans_keep_runs_at_least_min_long_including_one_at_the_end() {
        let samples: Vec<(Frame, bool)> = [false, true, true, true, false, true, false, true, true]
            .iter()
            .enumerate()
            .map(|(i, &hit)| (i as Frame * 6, hit))
            .collect();
        // Runs: 6..24 (18 frames), 30..36 (6), 42..54 (12, open at the end).
        assert_eq!(spans(&samples, 6, 12), vec![[6, 24], [42, 54]]);
        assert_eq!(spans(&samples, 6, 13), vec![[6, 24]]);
        assert!(spans(&[], 6, 1).is_empty());
    }

    /// T43.2: an instant zoom jumps the camera; a transform that pushes the take off the
    /// canvas is flagged with its span.
    #[test]
    fn model_checks_find_camera_jumps_and_off_canvas_pictures() {
        let mut project = crate::tests::sample_project();
        assert!(
            model_issues(
                &project,
                &Evaluator::prepare(&project).unwrap(),
                [120, 540],
                true,
                true
            )
            .is_empty()
        );
        let ops = splice_json::parse(
            r#"[{"cmd":"zoom.set","zoom":"ovl_detail","amount":2.5,"spring":"instant"},
                {"cmd":"param.set","target":"clp_take","path":"transform.x","value":-1.2}]"#,
        )
        .unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        let issues = model_issues(
            &project,
            &Evaluator::prepare(&project).unwrap(),
            [120, 540],
            true,
            true,
        );
        let checks: Vec<&str> = issues
            .iter()
            .filter_map(|issue| issue.field("check").and_then(Json::as_str))
            .collect();
        assert!(checks.contains(&"camera"), "{issues:?}");
        let off = issues
            .iter()
            .find(|issue| issue.field("check").and_then(Json::as_str) == Some("offCanvas"))
            .expect("off canvas");
        assert_eq!(off.field("from").and_then(Json::as_i64), Some(120));
    }

    /// T46.2: view.check reads the take's pointer, so a zoom that follows a fast cursor move
    /// with an instant spring is a camera jump; without the pointer the same zoom holds still.
    #[test]
    fn camera_check_follows_the_recorded_cursor() {
        let mut project = crate::tests::sample_project();
        let ops = splice_json::parse(
            r#"[{"cmd":"zoom.set","zoom":"ovl_detail","amount":3,"focus":"cursor","spring":"instant"},
                {"cmd":"item.trim","item":"ovl_detail","edge":"out","delta":150}]"#,
        )
        .unwrap();
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
        let dir = std::env::temp_dir().join(format!("splice-follow-check-{}", std::process::id()));
        let take = dir.join("takes/med_screen");
        std::fs::create_dir_all(&take).unwrap();
        // Rests left until 6 s, then a hand move across the screen in 0.1 s.
        let mut events = String::new();
        for i in 0..=1200 {
            let t = f64::from(i) / 120.0;
            let x = 0.2 + 0.7 * ((t - 6.0) / 0.1).clamp(0.0, 1.0);
            if !events.is_empty() {
                events.push(',');
            }
            events.push_str(&format!(r#"{{"t":{t},"k":"move","x":{x},"y":0.5}}"#));
        }
        std::fs::write(
            take.join("events.json"),
            format!(r#"{{"source":{{"rect":{{"width":1440,"height":900}}}},"events":[{events}]}}"#),
        )
        .unwrap();
        let pointers = crate::media::pointers(&dir, &project);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(pointers.len(), 1);
        let jumps = |evaluator: &Evaluator| {
            model_issues(&project, evaluator, [290, 340], true, false).len()
        };
        assert!(jumps(&Evaluator::prepare_with(&project, &pointers).unwrap()) > 0);
        assert_eq!(jumps(&Evaluator::prepare(&project).unwrap()), 0);
    }
}
