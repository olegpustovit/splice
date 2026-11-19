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
