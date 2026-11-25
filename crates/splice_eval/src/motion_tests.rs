//! T42 render plan: a title's text box, one paint order across kinds, and lane, spine and
//! project transforms as parents.
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

fn item(id: &str) -> ItemId {
    id.parse().unwrap()
}

/// Index of the layer an item draws, if it draws one.
fn depth(plan: &FramePlan, id: ItemId) -> Option<usize> {
    plan.layers.iter().position(|layer| match layer {
        Layer::Picture(p) => p.item == TimelineId::Item(id),
        Layer::Title { item, .. } | Layer::Shape { item, .. } | Layer::Caption { item, .. } => {
            *item == id
        }
        _ => false,
    })
}

fn title(plan: &FramePlan, id: ItemId) -> (Rect, f64, bool, Look) {
    plan.layers
        .iter()
        .find_map(|layer| match *layer {
            Layer::Title {
                item,
                bounds,
                font,
                wrap,
                look,
                ..
            } if item == id => Some((bounds, font, wrap, look)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn unstyled_title_keeps_the_plate_bounds_and_uses_single_line_program_type() {
    let project = sample();
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(192, &mut plan);
    let id = item("ovl_label");
    assert!(evaluator.is_unstyled_title(id));
    assert!(!evaluator.is_unstyled_title(item("ovl_cap2")));
    let (bounds, font, wrap, _) = title(&plan, id);
    let [w, h] = project.size.map(f64::from);
    let height = w.min(h) * 0.1;
    for (actual, expected) in [
        (bounds.x, w * 0.06),
        (bounds.y, h * 0.07),
        (bounds.w, w * 0.88),
        (bounds.h, height),
        (font, height * 0.56),
    ] {
        assert!((actual - expected).abs() < 1e-9);
    }
    assert!(!wrap);
    let [start, _] = evaluator.extent(TimelineId::Item(id)).unwrap();
    evaluator.frame(start, &mut plan);
    assert!(plan.layers.iter().any(|layer| matches!(layer,
        Layer::Title { item, opacity, .. } if *item == id && *opacity == 1.0 / 6.0)));
}

/// T42.2/T42.3: a box pins the title's rectangle; the font starts at its style size (or the
/// box height for `fill`) and the rasterizer wraps or not as the box says.
#[test]
fn title_box_sets_bounds_font_and_wrap() {
    let project = applied(
        r##"[{"cmd":"title.set","item":"ovl_hook","box":{"x":0.1,"y":0.2,"w":0.8,"h":0.25,"wrap":false},
              "style":{"size":0.08}},
             {"cmd":"title.set","item":"ovl_label","box":{"x":0.1,"y":0.6,"w":0.5,"h":0.1,"fit":"fill"}}]"##,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(30, &mut plan);
    assert!(!evaluator.is_unstyled_title(item("ovl_hook")));
    assert!(!evaluator.is_unstyled_title(item("ovl_label")));
    let (bounds, font, wrap, _) = title(&plan, item("ovl_hook"));
    let [w, h] = project.size.map(f64::from);
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(close(bounds.x, 0.1 * w) && close(bounds.y, 0.2 * h));
    assert!(close(bounds.w, 0.8 * w) && close(bounds.h, 0.25 * h));
    assert!(close(font, 0.08 * w.min(h)));
    assert!(!wrap);
    let label = (0..400)
        .find_map(|at| {
            evaluator.frame(at, &mut plan);
            depth(&plan, item("ovl_label")).map(|_| title(&plan, item("ovl_label")))
        })
        .unwrap();
    assert!(close(label.1, 0.1 * h) && label.2);
}

/// T42.4: z puts any kind in front of any other (higher = front); equal z keeps the default
/// order, so a sticker shape can sit above a title and a picture above both.
#[test]
fn one_paint_order_across_kinds() {
    let base = applied(
        r##"[{"cmd":"generator.add","shape":"ellipse","at":10,"length":60,"fill":"#ff0000","ref":"sticker"}]"##,
    );
    let order = |project: &Project| {
        let evaluator = Evaluator::prepare(project).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(30, &mut plan);
        let sticker = plan
            .layers
            .iter()
            .position(|l| matches!(l, Layer::Shape { .. }))
            .unwrap();
        (
            depth(&plan, item("ovl_logo")).unwrap(),
            sticker,
            depth(&plan, item("ovl_hook")).unwrap(),
        )
    };
    // Default: pictures, then shapes, then titles.
    let (logo, sticker, hook) = order(&base);
    assert!(logo < sticker && sticker < hook);
    let shape = base
        .items
        .order()
        .iter()
        .map(|&slot| (base.items.id(slot).unwrap(), base.items.data(slot).unwrap()))
        .find(|(_, data)| matches!(data.kind, ItemKind::Shape { .. }))
        .unwrap()
        .0;
    let mut project = base.clone();
    let ops = splice_json::parse(&format!(
        r#"[{{"cmd":"item.order","item":"{shape}","to":{{"above":"ovl_hook"}}}},
            {{"cmd":"item.order","item":"ovl_logo","to":"front"}}]"#
    ))
    .unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let (logo, sticker, hook) = order(&project);
    assert!(hook < sticker && sticker < logo, "{hook} {sticker} {logo}");
    // Back puts the title behind the picture too.
    let ops =
        splice_json::parse(r#"[{"cmd":"item.order","item":"ovl_hook","to":"back"}]"#).unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    let (_, _, hook) = order(&project);
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(30, &mut plan);
    assert!(hook < depth(&plan, item("ovl_logo")).unwrap());
}

/// T42.4: a lane's transform moves, scales and fades everything on it about the canvas; the
/// project's applies on top, to spine pictures too, and never to the backdrop.
#[test]
fn lane_and_project_transforms_are_parents() {
    let plain = applied("[]");
    let project = applied(
        r#"[{"cmd":"param.set","target":"lan_gfx","path":"transform.x","value":0.1},
            {"cmd":"param.set","target":"lan_gfx","path":"transform.scale","value":2},
            {"cmd":"param.keys","target":"project","path":"transform.opacity","keys":[
                {"at":0,"value":1,"ease":"linear"},{"at":60,"value":0.5}]}]"#,
    );
    let [w, h] = project.size.map(f64::from);
    let frame = |project: &Project, at| {
        let evaluator = Evaluator::prepare(project).unwrap();
        let mut plan = evaluator.buffer();
        evaluator.frame(at, &mut plan);
        plan
    };
    let (before, after) = (frame(&plain, 60), frame(&project, 60));
    let (bounds, _, _, look) = title(&after, item("ovl_hook"));
    assert_eq!(title(&before, item("ovl_hook")).3, Look::default());
    // Scale 2 about the canvas center, then 0.1 canvas widths right: the title's center moves
    // to 2 × (center − canvas center) + canvas center + 0.1 w.
    let center = [bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0];
    let moved = [
        2.0 * (center[0] - w / 2.0) + w / 2.0 + 0.1 * w,
        2.0 * (center[1] - h / 2.0) + h / 2.0,
    ];
    assert!((center[0] + look.offset[0] - moved[0]).abs() < 1e-6);
    assert!((center[1] + look.offset[1] - moved[1]).abs() < 1e-6);
    assert_eq!((look.scale, look.opacity), (2.0, 0.5));
    // The project fades the take's picture (absolute keys) and leaves the backdrop alone.
    let picture = after
        .layers
        .iter()
        .find_map(|l| match l {
            Layer::Picture(p) if matches!(p.item, TimelineId::Clip(_)) => Some(p.look),
            _ => None,
        })
        .unwrap();
    assert_eq!((picture.opacity, picture.scale), (0.5, 1.0));
    let backdrop = |plan: &FramePlan| {
        plan.layers.iter().find_map(|l| match l {
            Layer::Background { opacity, .. } => Some(*opacity),
            _ => None,
        })
    };
    assert_eq!(backdrop(&before), backdrop(&after));
    // Parents are evaluated per frame without allocating.
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for at in (0..660).step_by(11) {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(&plan);
            }
        },
    );
}

/// V13 chooses the topmost lane and scales the spine picture on its own clock.
#[test]
fn v13_scale_motion_has_exact_endpoints_and_no_warm_allocations() {
    let mut project = sample();
    let camera = Evaluator::prepare(&project).unwrap();
    let apply = |project: &mut Project, ops: &str| {
        let ops = splice_json::parse(ops).unwrap();
        edit::apply(project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    };
    apply(
        &mut project,
        r#"[{"cmd":"motion.add","at":180,"length":150,"from":1,"to":1.16}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let scale = |at| evaluator.motion_scale(at);
    assert_eq!(scale(179), 1.0);
    assert_eq!(scale(180), 1.0);
    assert!((scale(192) - 1.0128859060402684).abs() < 1e-12);
    assert_eq!(scale(329), 1.16);
    assert_eq!(scale(330), 1.0);
    assert!(camera.view(192).amount > 1.0);
    for at in 175..340 {
        assert_eq!(evaluator.view(at), camera.view(at));
    }

    let mut plan = evaluator.buffer();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for at in 178..=331 {
                evaluator.frame(at, &mut plan);
                std::hint::black_box((scale(at), &plan));
            }
        },
    );

    apply(
        &mut project,
        r#"[{"cmd":"motion.add","at":210,"length":30,"from":0.8,"to":0.9}]"#,
    );
    let overlap = Evaluator::prepare(&project).unwrap();
    assert_eq!(overlap.motion_scale(209), scale(209));
    assert_eq!(overlap.motion_scale(210), 0.8);
    assert_eq!(overlap.motion_scale(239), 0.9);
    assert_eq!(overlap.motion_scale(240), scale(240));

    // A file may supply overlapping items at the same rank. Creation order breaks the tie,
    // even when the second item starts before the first.
    let slot = *project.items.order().last().unwrap();
    let mut data = project.items.data(slot).unwrap().clone();
    data.kind = ItemKind::Motion(splice_model::ScaleMotion { from: 1.7, to: 1.7 });
    project
        .items
        .insert(splice_model::Item {
            id: ItemId::from_serial(project.serial),
            timing: splice_model::ItemTiming {
                anchor: splice_model::Anchor::Pinned(204),
                length: 50,
            },
            data,
        })
        .unwrap();
    let tie = Evaluator::prepare(&project).unwrap();
    assert_eq!(tie.motion_scale(209), 1.7);
    assert_eq!(tie.motion_scale(220), overlap.motion_scale(220));
    assert_eq!(tie.motion_scale(240), 1.7);
}

#[test]
fn v13_scale_motion_is_identity_in_explicit_and_implicit_gaps() {
    let project = applied(
        r#"[{"cmd":"overlay.add","kind":"motion","anchor":{"pinned":true,"frame":20},
             "length":560,"from":0.6,"to":0.6},
            {"cmd":"item.delete","items":"clp_take","ripple":false,"overlays":"delete"}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    assert_eq!(evaluator.motion_scale(90), 0.6);
    assert_eq!(evaluator.motion_scale(200), 1.0);
    assert_eq!(evaluator.motion_scale(550), 0.6);

    let mut project = project;
    let gap = project.clips.find("clp_open".parse().unwrap()).unwrap();
    project.clips.data_mut(gap).unwrap().media = None;
    let evaluator = Evaluator::prepare(&project).unwrap();
    assert_eq!(evaluator.motion_scale(90), 1.0);
    assert_eq!(evaluator.motion_scale(550), 0.6);
}
