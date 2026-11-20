//! T46.3 compositing: window backdrops (image, blur, angle, wallpapers), the camera bubble
//! (mirror, aspect, free position, size, shrink when zoomed, eased layouts) and timed masks.
use super::tests::sample;
use super::*;
use splice_core::{edit, package::journal::Source};
use splice_model::{MediaId, Project};

fn applied(ops: &str) -> Project {
    let mut project = sample();
    let ops = splice_json::parse(ops).unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    project
}

fn frame(project: &Project, at: Frame) -> FramePlan {
    let evaluator = Evaluator::prepare(project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(at, &mut plan);
    plan
}

/// Bounds, radius, shadow, opacity and mirror of the camera bubble.
fn bubble(plan: &FramePlan) -> (Rect, f64, f64, f64, bool) {
    plan.layers
        .iter()
        .find_map(|layer| match *layer {
            Layer::Camera {
                bounds,
                radius,
                shadow,
                opacity,
                mirror,
                ..
            } => Some((bounds, radius, shadow, opacity, mirror)),
            _ => None,
        })
        .expect("a camera layer")
}

fn backdrop(plan: &FramePlan) -> Background {
    plan.layers
        .iter()
        .find_map(|layer| match *layer {
            Layer::Background { fill, .. } => Some(fill),
            _ => None,
        })
        .expect("a backdrop")
}

fn camera_set(params: &str) -> String {
    format!(r#"[{{"cmd":"effect.set","target":"clp_take","type":"camera","params":{params}}}]"#)
}

#[test]
fn window_backdrops_take_an_image_an_angle_or_a_wallpaper() {
    let set = |params: &str| {
        applied(&format!(
            r#"[{{"cmd":"effect.set","target":"clp_take","type":"window","params":{params}}}]"#
        ))
    };
    let logo: MediaId = "med_logo".parse().unwrap();
    let image = set(r#"{"background":"image","image":"med_logo","blur":0.4}"#);
    assert_eq!(backdrop(&frame(&image, 180)), Background::Image(logo, 0.4));
    let angled = set(r##"{"color":"#ff0000","color2":"#0000ff","angle":90}"##);
    assert_eq!(
        backdrop(&frame(&angled, 180)),
        Background::Angled(0xff0000, 0x0000ff, 90.0)
    );
    let ocean = set(r#"{"background":"wallpaper","wallpaper":"ocean"}"#);
    let &(_, a, b, angle) = WALLPAPERS.iter().find(|w| w.0 == "ocean").unwrap();
    assert_eq!(
        backdrop(&frame(&ocean, 180)),
        Background::Angled(a, b, angle)
    );
    // Without an angle the gradient stays corner to corner (0.3 files render as before).
    let plain = set(r##"{"color":"#ff0000","color2":"#0000ff"}"##);
    assert_eq!(
        backdrop(&frame(&plain, 180)),
        Background::Gradient(0xff0000, 0x0000ff)
    );
}

#[test]
fn an_image_backdrop_must_be_a_still_of_the_project() {
    // A video is refused when written…
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"effect.set","target":"clp_take","type":"window","params":{"background":"image","image":"med_broll"}}]"#,
    )
    .unwrap();
    let refused =
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(refused.error.code, "WRONG_KIND");
    // …and a file that names one anyway does not prepare.
    let mut project = applied(
        r#"[{"cmd":"effect.set","target":"clp_take","type":"window","params":{"background":"image","image":"med_logo"}}]"#,
    );
    project
        .media
        .iter_mut()
        .find(|media| media.id == "med_logo".parse().unwrap())
        .unwrap()
        .kind = splice_model::MediaKind::Video;
    assert!(matches!(
        Evaluator::prepare(&project),
        Err(EvalError::Media(_))
    ));
}

#[test]
fn the_bubble_mirrors_takes_any_aspect_position_and_size() {
    let wide = applied(&camera_set(
        r#"{"mirror":true,"aspect":"16:9","x":0.5,"y":0.25,"size":1}"#,
    ));
    let (bounds, _, _, opacity, mirror) = bubble(&frame(&wide, 180));
    // Canvas 1080 × 1920: the bubble is the short side wide, 16:9, centered at (0.5, 0.25).
    assert!(mirror && opacity == 1.0);
    assert!((bounds.w - 1080.0).abs() < 1e-6, "{bounds:?}");
    assert!((bounds.h - 607.5).abs() < 1e-6, "{bounds:?}");
    assert!(bounds.x.abs() < 1e-6 && (bounds.y - (480.0 - 303.75)).abs() < 1e-6);
    // A circle is fully round whatever the roundness; a position near an edge stays on canvas.
    let circle = applied(&camera_set(
        r#"{"aspect":"circle","size":0.5,"roundness":0,"x":1,"y":1}"#,
    ));
    let (bounds, radius, ..) = bubble(&frame(&circle, 180));
    assert_eq!((bounds.w, bounds.h, radius), (540.0, 540.0, 270.0));
    assert_eq!((bounds.x + bounds.w, bounds.y + bounds.h), (1080.0, 1920.0));
    // Tall aspects keep the size on their long side; corners still work without x/y.
    let tall = applied(&camera_set(r#"{"aspect":"9:16","corner":"tl","size":0.4}"#));
    let (bounds, ..) = bubble(&frame(&tall, 180));
    assert!((bounds.h - 432.0).abs() < 1e-6 && (bounds.w - 243.0).abs() < 1e-6);
    assert_eq!((bounds.x, bounds.y), (54.0, 54.0));
}

#[test]
fn layouts_ease_between_bubble_full_and_hidden_without_a_cut() {
    // clp_take starts at 120; keys are clip-relative and name the layouts.
    let project = applied(&camera_set(
        r#"{"layout":{"keys":[{"at":0,"value":"bubble"},{"at":60,"value":"full","ease":"inOut"},{"at":150,"value":"hidden"}]}}"#,
    ));
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    let mut widths = Vec::new();
    for at in 175..=190 {
        evaluator.frame(at, &mut plan);
        widths.push(bubble(&plan).0.w);
    }
    let small = widths[0];
    // Bubble until the key at 180, then 0.3 s (9 frames) to the full canvas: no jump.
    assert_eq!(widths[5], small, "{widths:?}");
    assert!(widths[6] > small && widths[6] < 1080.0, "{widths:?}");
    assert!(widths.windows(2).all(|w| w[1] >= w[0]), "{widths:?}");
    assert!(
        widths
            .windows(2)
            .all(|w| w[1] - w[0] < 0.4 * (1080.0 - small)),
        "{widths:?}"
    );
    evaluator.frame(189, &mut plan);
    let (full, radius, shadow, ..) = bubble(&plan);
    assert_eq!((full.x, full.y, full.w, full.h), (0.0, 0.0, 1080.0, 1920.0));
    assert_eq!((radius, shadow), (0.0, 0.0));
    // Hidden fades out over the same ease from the key at 270.
    evaluator.frame(274, &mut plan);
    let fading = bubble(&plan).3;
    assert!(fading > 0.0 && fading < 1.0, "{fading}");
    evaluator.frame(279, &mut plan);
    assert_eq!(bubble(&plan).3, 0.0);
    // The layer stays in the stack (no change in the render stack mid-clip).
    assert!(
        plan.layers
            .iter()
            .any(|l| matches!(l, Layer::Camera { .. }))
    );
    // `hold` cuts.
    let cut = applied(&camera_set(
        r#"{"layout":{"keys":[{"at":0,"value":0},{"at":60,"value":1,"ease":"hold"}]}}"#,
    ));
    assert_eq!(bubble(&frame(&cut, 180)).0.w, 1080.0);
    // A constant layout needs no keys.
    let hidden = applied(&camera_set(r#"{"layout":"hidden"}"#));
    assert_eq!(bubble(&frame(&hidden, 180)).3, 0.0);
}

#[test]
fn unknown_layout_names_are_refused_with_the_options() {
    let mut project = sample();
    let ops = splice_json::parse(&camera_set(
        r#"{"layout":{"keys":[{"at":0,"value":"fullscreen"}]}}"#,
    ))
    .unwrap();
    let refused =
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(refused.error.code, "BAD_ARGS");
    assert!(refused.error.message.contains("layout"), "{refused:?}");
}

#[test]
fn the_bubble_shrinks_while_the_camera_zooms() {
    let zoom = r#"{"cmd":"zoom.add","at":300,"length":60,"amount":2,"spring":"instant"}"#;
    let plain = applied(&format!("[{zoom}]"));
    let shrinking = applied(&format!(
        r#"[{zoom},{{"cmd":"effect.set","target":"clp_take","type":"camera","params":{{"shrinkWhenZoomed":true}}}}]"#
    ));
    let size = |project: &Project, at| bubble(&frame(project, at)).0.w;
    assert!((size(&shrinking, 320) - 0.6 * size(&plain, 320)).abs() < 1e-6);
    // It follows the solved camera amount (the sample's own zooms too) frame by frame.
    for at in (130..540).step_by(10) {
        let plan = frame(&shrinking, at);
        let amount = plan
            .layers
            .iter()
            .find_map(|layer| match layer {
                Layer::Picture(p) if p.item == TimelineId::Clip("clp_take".parse().unwrap()) => {
                    Some(p.view.amount)
                }
                _ => None,
            })
            .unwrap();
        let expected = size(&plain, at) * (1.0 - 0.4 * (amount - 1.0).clamp(0.0, 1.0));
        assert!((size(&shrinking, at) - expected).abs() < 1e-6, "frame {at}");
    }
    // It stays pinned to its corner.
    let (a, b) = (
        bubble(&frame(&plain, 320)).0,
        bubble(&frame(&shrinking, 320)).0,
    );
    assert!((a.x + a.w - (b.x + b.w)).abs() < 1e-6 && (a.y + a.h - (b.y + b.h)).abs() < 1e-6);
}

#[test]
fn a_timed_mask_shows_only_from_to() {
    let project = applied(
        r#"[{"cmd":"mask.add","item":"clp_take","mode":"blur","x":0.1,"y":0.1,"w":0.3,"h":0.2,"from":150,"to":200,"ref":"m"}]"#,
    );
    let masked = |at| {
        let plan = frame(&project, at);
        plan.layers.iter().any(|layer| {
            matches!(layer, Layer::Picture(p)
                if p.item == TimelineId::Clip("clp_take".parse().unwrap())
                    && p.look.masks.iter().any(Option::is_some))
        })
    };
    assert_eq!([149, 150, 199, 200].map(masked), [false, true, true, false]);
    // Stored on the clip's own clock: it follows the clip when the clip moves.
    let fx = project
        .clips
        .order()
        .iter()
        .flat_map(|&slot| &project.clips.data(slot).unwrap().appearance.effects)
        .find(|effect| effect.kind == "mask")
        .unwrap();
    let get = |name: &str| {
        fx.params
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.value.clone())
    };
    assert_eq!(get("start"), Some(splice_model::EffectValue::Number(30.0)));
    assert_eq!(get("end"), Some(splice_model::EffectValue::Number(80.0)));
}

#[test]
fn camera_and_backdrop_frames_allocate_nothing() {
    let project = applied(
        r#"[{"cmd":"effect.set","target":"clp_take","type":"camera","params":{"shrinkWhenZoomed":true,
                "x":{"keys":[{"at":0,"value":0.2},{"at":200,"value":0.8}]},"y":0.7,
                "layout":{"keys":[{"at":0,"value":"bubble"},{"at":60,"value":"full"},{"at":120,"value":"hidden"},{"at":200,"value":"bubble"}]}}},
            {"cmd":"effect.set","target":"clp_take","type":"window","params":{"background":"wallpaper","angle":30}}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    let frames: Vec<Frame> = (100..560).step_by(3).collect();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for &at in &frames {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(&plan);
            }
        },
    );
}
