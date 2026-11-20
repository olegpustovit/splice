//! T45.4 render plan: counters resolve their text per frame, progress shapes sweep with
//! playback, particle emitters are seeded and constant-size, all without allocating per frame.
use super::tests::sample;
use super::*;
use splice_core::{edit, package::journal::Source};
use splice_model::{ItemKind, Project};

fn applied(ops: &str) -> Project {
    let mut project = sample();
    let ops = splice_json::parse(ops).unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    project
}

/// The item carrying an effect of `kind`.
fn holding(project: &Project, kind: &str) -> ItemId {
    project
        .items
        .order()
        .iter()
        .find(|&&slot| {
            let data = project.items.data(slot).unwrap();
            data.appearance.effects.iter().any(|e| e.kind == kind)
        })
        .map(|&slot| project.items.id(slot).unwrap())
        .unwrap()
}

fn shapes(plan: &FramePlan, id: ItemId) -> Vec<Layer> {
    plan.layers
        .iter()
        .filter(|l| matches!(l, Layer::Shape { item, .. } if *item == id))
        .copied()
        .collect()
}

#[test]
fn counter_text_follows_its_eased_keys_in_runs() {
    let project = applied(
        r#"[{"cmd":"counter.add","at":60,"length":45,"from":0,"to":1250,"format":"{n} users"}]"#,
    );
    let id = holding(&project, "counter");
    let slot = project.items.find(id).unwrap();
    // The stored title shows the final value, so a snapshot reads like the last frame.
    assert!(matches!(
        &project.items.data(slot).unwrap().kind,
        ItemKind::Title { text } if text == "1,250 users"
    ));
    let evaluator = Evaluator::prepare(&project).unwrap();
    assert_eq!(evaluator.title_at(id, 60), Some("0 users"));
    assert_eq!(evaluator.title_at(id, 104), Some("1,250 users"));
    let runs = evaluator.title_runs(id);
    assert_eq!(runs.first().unwrap().range[0], 60);
    assert_eq!(runs.last().unwrap().range[1], 105);
    assert!(
        runs.windows(2)
            .all(|pair| pair[0].range[1] == pair[1].range[0])
    );
    assert!(runs.windows(2).all(|pair| pair[0].text != pair[1].text));
    assert!(runs.len() <= 45);
    let number = |at| {
        evaluator
            .title_at(id, at)
            .unwrap()
            .trim_end_matches(" users")
            .replace(',', "")
            .parse::<f64>()
            .unwrap()
    };
    assert!((60..104).all(|at| number(at) <= number(at + 1)));
    // Ease out: past half way at the middle.
    assert!(number(82) > 625.0, "{}", number(82));
    // A plain title has no runs and shows its text.
    assert!(evaluator.title_runs("ovl_hook".parse().unwrap()).is_empty());
}

#[test]
fn progress_bar_and_ring_sweep_with_playback() {
    let project = applied(
        r##"[{"cmd":"progress.add","shape":"bar","fill":"#ff3b30","track":"#202020"},
            {"cmd":"progress.add","shape":"ring","at":100,"length":101}]"##,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let end = evaluator.breakpoints().last().copied().unwrap();
    let mut plan = evaluator.buffer();
    let ids: Vec<ItemId> = project
        .items
        .order()
        .iter()
        .filter(|&&s| matches!(project.items.data(s).unwrap().kind, ItemKind::Shape { .. }))
        .map(|&s| project.items.id(s).unwrap())
        .collect();
    let sweep = |plan: &FramePlan, id: ItemId| -> (Layer, f64) {
        let layers = shapes(plan, id);
        assert_eq!(layers.len(), 2, "track and fill");
        let Layer::Shape { sweep, .. } = layers[1] else {
            unreachable!()
        };
        (layers[0], sweep)
    };
    let (bar, ring) = (ids[ids.len() - 2], ids[ids.len() - 1]);
    evaluator.frame(0, &mut plan);
    let (track, at_start) = sweep(&plan, bar);
    assert_eq!(at_start, 0.0);
    assert!(matches!(
        track,
        Layer::Shape { fill: 0x202020, opacity, sweep, .. } if opacity == 0.25 && sweep == 1.0
    ));
    evaluator.frame(end - 1, &mut plan);
    assert_eq!(sweep(&plan, bar).1, 1.0);
    evaluator.frame(150, &mut plan);
    let (track, half) = sweep(&plan, ring);
    assert!((half - 0.5).abs() < 1e-9, "{half}");
    // A ring is circular on the portrait canvas and has a thickness.
    let Layer::Shape {
        shape,
        bounds,
        stroke_width,
        ..
    } = track
    else {
        unreachable!()
    };
    assert_eq!(shape, ShapeKind::Ring);
    assert!((bounds.w - bounds.h).abs() < 1.0 && stroke_width > 0.0);
}

#[test]
fn particles_are_seeded_constant_in_count_and_allocation_free() {
    let ops = |seed: u32, preset: &str| {
        format!(
            r#"[{{"cmd":"particles.add","at":90,"length":60,"preset":"{preset}","count":80,"seed":{seed}}}]"#
        )
    };
    for preset in ["confetti", "sparkle", "snow", "hearts"] {
        let project = applied(&ops(7, preset));
        let id = holding(&project, "particles");
        let evaluator = Evaluator::prepare(&project).unwrap();
        let again = Evaluator::prepare(&applied(&ops(7, preset))).unwrap();
        let other = Evaluator::prepare(&applied(&ops(8, preset))).unwrap();
        let (mut a, mut b, mut c) = (evaluator.buffer(), again.buffer(), other.buffer());
        let mut alive = 0;
        for at in 90..150 {
            evaluator.frame(at, &mut a);
            again.frame(at, &mut b);
            other.frame(at, &mut c);
            let layers = shapes(&a, id);
            // The render stack never changes inside the item.
            assert_eq!(layers.len(), 80, "{preset} at {at}");
            assert_eq!(
                layers,
                shapes(&b, id),
                "{preset}: same seed, same particles"
            );
            assert_ne!(layers, shapes(&c, id), "{preset}: a new seed moves them");
            alive += layers
                .iter()
                .filter(|l| matches!(l, Layer::Shape { opacity, .. } if *opacity > 0.0))
                .count();
        }
        assert!(alive > 80 * 60 / 4, "{preset}: {alive} live sprites");
        let kind = |l: &Layer| match l {
            Layer::Shape { shape, .. } => *shape,
            _ => unreachable!(),
        };
        evaluator.frame(120, &mut a);
        let expected = match preset {
            "confetti" => ShapeKind::Rect,
            "hearts" => ShapeKind::Heart,
            _ => ShapeKind::Ellipse,
        };
        assert!(shapes(&a, id).iter().all(|l| kind(l) == expected));
    }
    // Counters, progress and particles evaluate per frame without allocating.
    let project = applied(
        r#"[{"cmd":"particles.add","at":90,"length":60,"preset":"confetti","count":120},
            {"cmd":"progress.add","shape":"ring"},
            {"cmd":"counter.add","at":60,"length":45,"to":99}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for at in 0..660 {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(&plan);
            }
        },
    );
}
