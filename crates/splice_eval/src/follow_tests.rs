//! T46.2: zooms that follow the cursor, the cursor loop and motion blur on the camera.
use super::cursor::{Recorded, Sample};
use super::tests::sample;
use super::*;
use splice_core::{edit, package::journal::Source};
use splice_model::Project;

fn applied(ops: &str) -> Project {
    let mut project = sample();
    let ops = splice_json::parse(ops).unwrap();
    edit::apply(&mut project, ops.as_array().unwrap(), Source::Agent, false).unwrap();
    project
}

/// A pointer that rests at `a` until 4 s, drifts a little, moves to `b` by hand over 0.5 s at
/// 6 s and rests there (120 Hz samples, as the recorder writes them).
fn pointer(a: [f64; 2], b: [f64; 2]) -> Vec<Recorded> {
    let mut samples = Vec::new();
    for i in 0..=(12 * 120) {
        let t = f64::from(i) / 120.0;
        let p = if t < 4.0 {
            a
        } else if t < 5.0 {
            // Inside the dead zone: 2 % of the content.
            [a[0] + 0.02 * (t - 4.0), a[1]]
        } else if t < 6.0 {
            [a[0] + 0.02, a[1]]
        } else if t < 6.5 {
            let k = (t - 6.0) / 0.5;
            [
                a[0] + 0.02 + (b[0] - a[0] - 0.02) * k,
                a[1] + (b[1] - a[1]) * k,
            ]
        } else {
            b
        };
        samples.push(Sample {
            t,
            x: p[0],
            y: p[1],
            image: 0,
            click: false,
        });
    }
    vec![Recorded {
        media: "med_screen".parse().unwrap(),
        size: [1440.0, 900.0],
        samples,
    }]
}

/// The take picture at `at`: its view and where the cursor (source point `p`) sits in it as a
/// content point and relative to the viewport center, in viewports.
fn framing(evaluator: &Evaluator, at: Frame, p: [f64; 2]) -> (View, [f64; 2]) {
    let mut plan = evaluator.buffer();
    evaluator.frame(at, &mut plan);
    let picture = plan
        .layers
        .iter()
        .find_map(|l| match l {
            Layer::Picture(p) if p.item == TimelineId::Clip("clp_take".parse().unwrap()) => {
                Some(*p)
            }
            _ => None,
        })
        .unwrap();
    let [x, y, w, h] = picture.content([640.0, 400.0]);
    let point = [(p[0] - x) / w, (p[1] - y) / h];
    let [vx, vy, vw, vh] = picture.view.viewport();
    (
        picture.view,
        [
            (point[0] - (vx + vw / 2.0)) / vw,
            (point[1] - (vy + vh / 2.0)) / vh,
        ],
    )
}

/// `focus:"cursor"` centers on the cursor as the zoom starts, holds still while the cursor
/// moves inside the central 60 %, and pans just enough to keep it there when it leaves.
#[test]
fn a_following_zoom_keeps_the_cursor_in_its_dead_zone() {
    // The take plays source frame s at timeline 120 + s; the zoom covers source 60–360.
    let project = applied(
        r#"[{"cmd":"zoom.set","zoom":"ovl_detail","amount":2.5,"focus":"cursor"},
            {"cmd":"item.trim","item":"ovl_detail","edge":"out","delta":150}]"#,
    );
    let (a, b) = ([0.5, 0.5], [0.78, 0.82]);
    let evaluator = Evaluator::prepare_with(&project, &pointer(a, b)).unwrap();
    // Settled on the cursor 1 s into the zoom (source 3 s).
    let (view, offset) = framing(&evaluator, 120 + 90, a);
    assert!(view.amount > 2.4, "{view:?}");
    assert!(
        offset[0].abs() < 0.02 && offset[1].abs() < 0.02,
        "{offset:?}"
    );
    // The drift inside the dead zone (source 4–6 s) leaves the camera where it was.
    let still = framing(&evaluator, 120 + 150, a).0;
    let drifted = framing(&evaluator, 120 + 175, [a[0] + 0.02, a[1]]).0;
    assert!(
        (still.center[0] - drifted.center[0]).abs() < 1e-6,
        "{still:?} {drifted:?}"
    );
    // After the move the cursor sits on the dead zone's edge (0.3 of the view from center),
    // not in the middle: the camera moved just enough. The drawn cursor rests a shake radius
    // (0.0035 of the source, 0.02 of this view) short of the raw sample the test measures.
    let (moved, offset) = framing(&evaluator, 120 + 260, b);
    assert!(moved.center != still.center);
    for axis in offset {
        assert!((0.28..=0.33).contains(&axis.abs()), "{offset:?}");
    }
    // Deterministic: a second prepare gives the same camera.
    let again = Evaluator::prepare_with(&project, &pointer(a, b)).unwrap();
    for at in [150, 210, 300, 380] {
        assert_eq!(evaluator.view(at), again.view(at));
    }
}

/// Without the take's pointer (an offline event log) a following zoom still zooms, holding the
/// framing it has; the camera never jumps.
#[test]
fn a_following_zoom_without_pointer_holds_its_framing() {
    let project =
        applied(r#"[{"cmd":"zoom.set","zoom":"ovl_detail","amount":2,"focus":"cursor"}]"#);
    let evaluator = Evaluator::prepare(&project).unwrap();
    assert!(evaluator.view(250).amount > 1.9);
    assert_eq!(evaluator.view(250).center, [0.5, 0.5]);
}

/// The cursor `loop`: untouched before the clip's last `frames`, then eased (monotonically) to
/// where the clip started, landing exactly on it on the last frame. The effect resolves it.
#[test]
fn a_looping_cursor_eases_back_to_its_first_position() {
    use super::cursor::{Point, looped};
    let at = |x: f64| Point {
        x,
        y: 0.2,
        image: 0,
        opacity: 1.0,
        scale: 1.0,
    };
    let (first, here) = (at(0.1), at(0.9));
    assert_eq!(looped(here, first, 30.0, 30.0), here);
    assert_eq!(looped(here, first, 29.0, 30.0).x, 0.9);
    assert_eq!(looped(here, first, 0.0, 30.0), first);
    assert_eq!(looped(here, first, 0.0, 0.0), here);
    let xs: Vec<f64> = (0..30)
        .rev()
        .map(|left| looped(here, first, f64::from(left), 30.0).x)
        .collect();
    assert!(xs.windows(2).all(|w| w[1] <= w[0]), "{xs:?}");
    let project = applied(
        r#"[{"cmd":"effect.add","target":"clp_take","type":"cursor","params":{"loop":45,"motionBlur":0.5,"alwaysArrow":true,"clickSound":true}}]"#,
    );
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    evaluator.frame(300, &mut plan);
    let cursor = plan
        .layers
        .iter()
        .find_map(|l| match l {
            Layer::Picture(p) => p.cursor,
            _ => None,
        })
        .unwrap();
    assert_eq!((cursor.loop_frames, cursor.motion_blur), (45.0, 0.5));
    assert!(cursor.motion.always_arrow);
    assert!(evaluator.click_sound("clp_take".parse().unwrap()));
    assert!(!evaluator.click_sound("clp_open".parse().unwrap()));
}

/// Per-frame evaluation with a following, blurred zoom allocates nothing: the path is solved at
/// prepare.
#[test]
fn warm_frames_with_a_following_zoom_allocate_nothing() {
    let project = applied(
        r#"[{"cmd":"zoom.set","zoom":"ovl_detail","amount":2,"focus":"cursor","motionBlur":0.5}]"#,
    );
    let evaluator = Evaluator::prepare_with(&project, &pointer([0.2, 0.3], [0.8, 0.7])).unwrap();
    let mut plan = evaluator.buffer();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for at in [100, 181, 240, 300, 329, 400] {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(evaluator.view(at - 1));
                std::hint::black_box(&plan);
            }
        },
    );
    assert_eq!(evaluator.view(250).blur, 0.5);
    // The blur stays with the camera as it springs back out, like the spring does.
    assert_eq!(evaluator.view(335).blur, 0.5);
    assert_eq!(evaluator.view(100).blur, 0.0);
}
