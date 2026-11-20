//! 0.3 render plan: keyed looks, transitions, speed ramps, freezes, compounds and solo, plus the
//! warm-frame budget with 2,000 keys (PLAN.md T16.4).
use super::tests::sample;
use super::*;
use splice_core::{edit, package::journal::Source};
use splice_model::Project;

fn applied(ops: &str) -> Project {
    applied_to(sample(), ops)
}

fn applied_to(mut project: Project, ops: &str) -> Project {
    let ops = splice_json::parse(ops).unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    project
}

fn picture(plan: &FramePlan, item: TimelineId) -> Picture {
    plan.layers
        .iter()
        .find_map(|layer| match layer {
            Layer::Picture(p) if p.item == item => Some(*p),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no picture for {item}"))
}

fn clip(id: &str) -> TimelineId {
    TimelineId::Clip(id.parse().unwrap())
}

#[test]
fn keyed_transform_color_and_masks_sample_per_frame() {
    let project = applied(
        r##"[{"cmd":"param.keys","target":"clp_take","path":"transform.scale","keys":[{"at":0,"value":1,"ease":"inOut"},{"at":60,"value":2}]},
            {"cmd":"param.set","target":"clp_take","path":"transform.rotation","value":-5},
            {"cmd":"param.set","target":"project","path":"color.saturation","value":0.5},
            {"cmd":"mask.add","item":"clp_take","mode":"pixelate","x":0.1,"y":0.1,"w":0.2,"h":0.1}]"##,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(150, &mut plan);
    let look = picture(&plan, clip("clp_take")).look;
    assert!((look.scale - 1.0 - 1.0 * keys_ease(30.0 / 60.0)).abs() < 1e-9);
    assert_eq!(look.rotation, -5.0);
    assert_eq!(look.color.unwrap().saturation, 0.5);
    let mask = look.masks[0].unwrap();
    assert_eq!(
        (mask.mode, mask.rect),
        (MaskMode::Pixelate, [0.1, 0.1, 0.2, 0.1])
    );
    evaluator.frame(300, &mut plan);
    assert_eq!(picture(&plan, clip("clp_take")).look.scale, 2.0);
    // The opener inherits the project grade but not the take's own transform.
    evaluator.frame(10, &mut plan);
    let opener = picture(&plan, clip("clp_open")).look;
    assert_eq!((opener.scale, opener.color.unwrap().saturation), (1.0, 0.5));
}

fn keys_ease(t: f64) -> f64 {
    splice_model::Ease::InOut.apply(t, 1.0)
}

#[test]
fn h6_dissolve_composites_the_incoming_side_as_one_group() {
    // Source handles on both sides of the cut: the opener runs 30 frames past its clip and the
    // take starts 30 frames into its media (D14: a transition consumes them).
    let mut project = sample();
    let opener = "med_opener".parse().unwrap();
    project
        .media
        .iter_mut()
        .find(|m| m.id == opener)
        .unwrap()
        .probe
        .frames = 150;
    let take = project.clips.find("clp_take".parse().unwrap()).unwrap();
    project.clips.timing_mut(take).unwrap().source = 30;
    let project = applied_to(
        project,
        r#"[{"cmd":"transition.add","item":"clp_take","kind":"dissolve","length":20}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let cuts = evaluator.breakpoints();
    assert!(cuts.contains(&110) && cuts.contains(&130));
    let mut plan = evaluator.buffer();
    evaluator.frame(119, &mut plan);
    let (outgoing, incoming) = (
        picture(&plan, clip("clp_open")),
        picture(&plan, clip("clp_take")),
    );
    // The incoming side is one group composited at the transition's progress (D14); its
    // layers keep their own opacity, so a window and its backdrop never compound.
    assert_eq!((outgoing.look.opacity, incoming.look.opacity), (1.0, 1.0));
    let group = plan.group.expect("incoming group");
    assert!((group.opacity - 0.475).abs() < 1e-9);
    assert!(
        plan.layers[group.layers[0]..group.layers[1]]
            .iter()
            .any(|l| matches!(l, Layer::Picture(p) if p.item == clip("clp_take")))
    );
    assert!(
        !plan.layers[group.layers[0]..group.layers[1]]
            .iter()
            .any(|l| matches!(l, Layer::Picture(p) if p.item == clip("clp_open")))
    );
    // The incoming take starts early on its own clock, inside its head handle.
    assert_eq!(incoming.source_frame, 29.0);
    // Inside the take, its audio fades in on the equal-power curve.
    evaluator.frame(125, &mut plan);
    let gain = |plan: &FramePlan| -> f64 {
        plan.audio
            .iter()
            .filter(|a| a.item == clip("clp_take"))
            .map(|a| a.gain)
            .sum()
    };
    let partial = gain(&plan);
    evaluator.frame(200, &mut plan);
    let full = gain(&plan);
    let t: f64 = 15.5 / 20.0;
    assert!((partial / full - (t * std::f64::consts::FRAC_PI_2).sin()).abs() < 1e-9);
    evaluator.frame(105, &mut plan);
    assert!(
        plan.layers
            .iter()
            .all(|l| !matches!(l, Layer::Picture(p) if p.item == clip("clp_take")))
    );
}

#[test]
fn speed_ramps_integrate_and_freezes_hold() {
    let project = applied(
        r#"[{"cmd":"param.keys","target":"clp_take","path":"speed.rate","keys":[{"at":0,"value":1,"ease":"linear"},{"at":60,"value":2,"ease":"linear"},{"at":120,"value":1}]}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(120 + 60, &mut plan);
    // ∫ 1→2 over 60 frames = 90 source frames (the take runs at the timeline rate).
    assert!((picture(&plan, clip("clp_take")).source_frame - 90.0).abs() < 1e-6);
    evaluator.frame(120 + 70, &mut plan);
    let expected = 90.0 + 20.0 - 100.0 / 120.0;
    assert!((picture(&plan, clip("clp_take")).source_frame - expected).abs() < 1e-6);
    // A ramp that runs past the media's end is refused at apply time.
    let mut project = sample();
    let ops = splice_json::parse(
        r#"[{"cmd":"param.keys","target":"clp_take","path":"speed.rate","keys":[{"at":0,"value":2}]}]"#,
    )
    .unwrap();
    let error =
        edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap_err();
    assert_eq!(error.error.code, "INVARIANT");

    let project =
        applied(r#"[{"cmd":"clip.freeze","item":"clp_take","at":200,"length":45,"ref":"hold"}]"#);
    let evaluator = Evaluator::prepare(&project).unwrap();
    let held = plan_at(&evaluator, 200);
    for at in [210, 244] {
        let frame = plan_at(&evaluator, at);
        assert_eq!(frame.0, held.0);
        assert!(frame.1 == 0.0, "freeze is silent");
    }
}

/// (source frame of the spine picture, total audio gain) at a frame.
fn plan_at(evaluator: &Evaluator, at: Frame) -> (f64, f64) {
    let mut plan = evaluator.buffer();
    evaluator.frame(at, &mut plan);
    let id = TimelineId::Clip(plan.clip.unwrap());
    (
        picture(&plan, id).source_frame,
        plan.audio
            .iter()
            .filter(|a| a.item == id)
            .map(|a| a.gain)
            .sum(),
    )
}

#[test]
fn compound_clips_render_like_the_clips_they_hold() {
    let base = sample();
    let compound = applied(r#"[{"cmd":"clip.compound","items":["clp_take","clp_end"]}]"#);
    let (a, b) = (
        Evaluator::prepare(&base).unwrap(),
        Evaluator::prepare(&compound).unwrap(),
    );
    let (mut pa, mut pb) = (a.buffer(), b.buffer());
    for at in [0, 119, 120, 300, 539, 540, 659] {
        a.frame(at, &mut pa);
        b.frame(at, &mut pb);
        assert_eq!(pa.layers, pb.layers, "frame {at}");
        assert_eq!(pa.audio, pb.audio, "frame {at}");
    }
}

#[test]
fn unlinked_voice_plays_take_audio_with_keyed_gain_and_solo() {
    let project = applied(
        r#"[{"cmd":"audio.unlink","item":"clp_take","ref":"voice"},
            {"cmd":"param.keys","target":"$voice","path":"volume.gain","keys":[{"at":0,"value":0},{"at":30,"value":1}]}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(135, &mut plan);
    let take = clip("clp_take");
    assert!(
        plan.audio
            .iter()
            .filter(|a| a.item == take)
            .all(|a| a.gain == 0.0)
    );
    let voice: Vec<_> = plan.audio.iter().filter(|a| a.item != take).collect();
    assert!(!voice.is_empty() && voice.iter().all(|a| (a.gain - 0.5 * 1.0).abs() < 0.21));
    // Soloing the (muted) take clip silences the unsoloed voice.
    let soloed = applied(
        r#"[{"cmd":"audio.unlink","item":"clp_take"},{"cmd":"effect.add","target":"clp_take","type":"volume","params":{"solo":true}}]"#,
    );
    let evaluator = Evaluator::prepare(&soloed).unwrap();
    evaluator.frame(300, &mut plan);
    assert!(!plan.audio.is_empty() && plan.audio.iter().all(|a| a.gain == 0.0));
}

#[test]
fn warm_frames_with_2000_keys_allocate_nothing_and_stay_fast() {
    let mut ops = String::from("[");
    for n in 0..200 {
        let at = (n * 3) % 600;
        let keys: Vec<String> = (0..10)
            .map(|k| {
                format!(
                    r#"{{"at":{},"value":{},"ease":"inOut"}}"#,
                    k * 9,
                    1.0 + f64::from(k) * 0.05
                )
            })
            .collect();
        if n > 0 {
            ops.push(',');
        }
        ops.push_str(&format!(
            r#"{{"cmd":"title.add","at":{at},"length":90,"text":"T{n}","ref":"t{n}"}},{{"cmd":"param.keys","target":"$t{n}","path":"transform.scale","keys":[{}]}}"#,
            keys.join(",")
        ));
    }
    ops.push(']');
    let project = applied(&ops);
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    let frames: Vec<Frame> = (0..660).step_by(7).collect();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for &at in &frames {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(&plan);
            }
        },
    );
    // Best of 3 per frame keeps the lock stable on a loaded machine.
    let mut times: Vec<u128> = frames
        .iter()
        .map(|&at| {
            (0..3)
                .map(|_| {
                    let start = std::time::Instant::now();
                    evaluator.frame(at, &mut plan);
                    std::hint::black_box(&plan);
                    start.elapsed().as_nanos()
                })
                .min()
                .unwrap()
        })
        .collect();
    times.sort_unstable();
    let p99 = times[times.len() * 99 / 100];
    // T24.3 lock: measured ~1.9 µs on the Linux box (2026-11-19); 100 µs is ~50× headroom.
    assert!(p99 < 100_000, "p99 {p99} ns");
}

/// The depth cap counts nesting, not compound clips: six sibling compounds render.
#[test]
fn many_sibling_compounds_render_like_their_clips() {
    let splits = r#"{"cmd":"item.split","item":"clp_take","at":200,"ref":"a"},
        {"cmd":"item.split","item":"$a.right","at":280,"ref":"b"},
        {"cmd":"item.split","item":"$b.right","at":360,"ref":"c"}"#;
    let compounds = [
        "clp_open", "clp_take", "$a.right", "$b.right", "$c.right", "clp_end",
    ]
    .map(|item| format!(r#"{{"cmd":"clip.compound","items":["{item}"]}}"#))
    .join(",");
    let split = applied(&format!("[{splits}]"));
    let compound = applied(&format!("[{splits},{compounds}]"));
    assert_eq!(compound.compounds.len(), 6);
    let (a, b) = (
        Evaluator::prepare(&split).unwrap(),
        Evaluator::prepare(&compound).unwrap(),
    );
    let (mut pa, mut pb) = (a.buffer(), b.buffer());
    for at in [0, 150, 250, 330, 400, 600] {
        a.frame(at, &mut pa);
        b.frame(at, &mut pb);
        assert_eq!(pa.layers, pb.layers, "frame {at}");
    }
}

/// T36.4 (M20): every parameter the schema marks keyable changes the evaluated frame when it
/// is keyed, and nothing else accepts keys. A keyed parameter that renders its constant would
/// fail here.
#[test]
fn m20_every_keyable_parameter_renders_its_keys() {
    use splice_core::commands::{effect_schemas, keyable};
    use splice_json::Value as Json;
    let render = |ops: &str| {
        let evaluator = Evaluator::prepare(&applied(ops)).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(180, &mut plan);
        // A counter's number is the text it shows.
        let titles: Vec<_> = plan
            .layers
            .iter()
            .filter_map(|layer| match layer {
                Layer::Title { item, .. } => evaluator.title_at(*item, 180),
                _ => None,
            })
            .collect();
        format!("{:?} {:?} {titles:?}", plan.layers, plan.audio)
    };
    let mut checked = 0;
    for (kind, schema) in effect_schemas().as_object().unwrap() {
        let params = schema.field("params").and_then(Json::as_object);
        for (name, spec) in params.into_iter().flatten() {
            if !keyable(spec) {
                continue;
            }
            let bound = |key: &str| spec.field(key).and_then(Json::as_f64).unwrap();
            let (min, max) = (bound("min"), bound("max"));
            let (a, b) = match (kind.as_str(), name.as_str()) {
                // The take has 120 spare media frames: ramp gently.
                ("speed", "rate") => (1.0, 1.1),
                _ => (min + 0.25 * (max - min), min + 0.75 * (max - min)),
            };
            // A crop pans inside the slack its size leaves; counters and progress live on
            // their own title and shape (T45.4).
            let (context, target) = match (kind.as_str(), name.as_str()) {
                ("crop", "x" | "y") => (
                    r#"{"cmd":"param.set","target":"clp_take","path":"crop.w","value":0.5},
                       {"cmd":"param.set","target":"clp_take","path":"crop.h","value":0.5},"#,
                    "clp_take",
                ),
                ("counter", _) => (
                    r#"{"cmd":"counter.add","at":150,"length":60,"to":10,"ref":"own"},"#,
                    "$own",
                ),
                ("progress", _) => (
                    r#"{"cmd":"progress.add","at":150,"length":60,"ref":"own"},"#,
                    "$own",
                ),
                _ => ("", "clp_take"),
            };
            let held = render(&format!(
                r#"[{context}{{"cmd":"param.set","target":"{target}","path":"{kind}.{name}","value":{a}}}]"#
            ));
            let keyed = render(&format!(
                r#"[{context}{{"cmd":"param.keys","target":"{target}","path":"{kind}.{name}","keys":[{{"at":0,"value":{a}}},{{"at":120,"value":{b}}}]}}]"#
            ));
            assert_ne!(
                held, keyed,
                "{kind}.{name} accepts keys but renders a constant"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 35, "keyable parameters");
}

/// T40.4: whip, zoomPunch and swipe move content only: the outgoing side's backdrop is never
/// shifted and the incoming side composites as one group.
#[test]
fn content_only_transitions_never_move_the_backdrop() {
    for kind in ["whip", "zoomPunch", "swipe"] {
        let project = applied(&format!(
            r#"[{{"cmd":"transition.add","item":"clp_end","kind":"{kind}","length":20}}]"#
        ));
        let evaluator = Evaluator::prepare(&project).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(540, &mut plan);
        let shift = plan.shift.unwrap_or_else(|| panic!("{kind}: no shift"));
        let group = plan.group.unwrap_or_else(|| panic!("{kind}: no group"));
        assert!(matches!(plan.layers[0], Layer::Background { .. }), "{kind}");
        assert!(shift.layers[0] >= 1, "{kind}: backdrop moved");
        assert!(group.layers[0] >= shift.layers[1], "{kind}");
        assert!(
            plan.layers[shift.layers[0]..shift.layers[1]]
                .iter()
                .any(|l| matches!(l, Layer::Picture(p) if p.item == clip("clp_take"))),
            "{kind}: the take moves"
        );
    }
}
