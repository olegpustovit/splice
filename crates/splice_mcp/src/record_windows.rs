//! Multi-window takes (T46.6): each recorded window is its own movie in one take. The first is
//! the spine clip, every other one a media overlay anchored to it; all of them keep the
//! positions they had on screen (or stand in a row when they overlapped), scaled into the
//! canvas inside the window look's padding.
use crate::{Json, object, text};
use splice_model::{Aspect, Frame, Project};

/// One recorded window: its file in the take directory, its window ID, its bounds on screen
/// (global points, when known) and its picture.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layer {
    /// `screen.mov` for the first window, `window-<n>.mov` for window n (0-based).
    pub file: String,
    /// The `win_` ID it was recorded from, when known.
    pub window: Option<u32>,
    /// x, y, width, height in global points at the start of the take; `None` after a crash
    /// recovery without events (the layers then stand in a row).
    pub rect: Option<[f64; 4]>,
    /// Encoded pixel size.
    pub size: [u32; 2],
    /// Length in project frames.
    pub frames: Frame,
}

/// Where the layers land: `arrangement` is `captured` (their positions on screen) or `row`
/// (side by side in source order, because they overlapped), `boxes` are canvas rectangles
/// (x, y, width, height in pixels) and `transforms` the `transform` effect (x, y, scale) that
/// puts each layer's fitted picture there.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Plan {
    pub arrangement: &'static str,
    pub boxes: Vec<[f64; 4]>,
    pub transforms: Vec<[f64; 3]>,
    /// The window look's padding the layers fit inside.
    pub padding: f64,
    /// Whether the spine already has a window look (otherwise the take gets one).
    pub has_window: bool,
    /// Whether the look is on (a spine look turned off stays off, on every layer).
    pub windowed: bool,
}

/// Each layer's box in arrangement space (points) and the space's extent. Windows that overlap
/// by more than 1 % of the smaller one, or without known bounds, stand in a row of equal
/// heights; others keep their captured positions. Heights follow each picture's shape.
fn arrange(layers: &[Layer]) -> (&'static str, Vec<[f64; 4]>, [f64; 2]) {
    let shaped: Option<Vec<[f64; 4]>> = layers
        .iter()
        .map(|layer| {
            let [x, y, w, _] = layer.rect?;
            let [pw, ph] = layer.size.map(f64::from);
            let rect = [x, y, w, w * ph / pw.max(1.0)];
            (rect.iter().all(|v| v.is_finite()) && w > 0.0 && rect[3] > 0.0).then_some(rect)
        })
        .collect();
    let overlap = |a: &[f64; 4], b: &[f64; 4]| {
        let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
        let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
        w > 0.0 && h > 0.0 && w * h > 0.01 * (a[2] * a[3]).min(b[2] * b[3])
    };
    if let Some(rects) = shaped.filter(|rects| {
        rects
            .iter()
            .enumerate()
            .all(|(i, a)| rects[i + 1..].iter().all(|b| !overlap(a, b)))
    }) {
        let left = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
        let top = rects.iter().map(|r| r[1]).fold(f64::INFINITY, f64::min);
        let right = rects
            .iter()
            .map(|r| r[0] + r[2])
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = rects
            .iter()
            .map(|r| r[1] + r[3])
            .fold(f64::NEG_INFINITY, f64::max);
        let boxes = rects
            .iter()
            .map(|r| [r[0] - left, r[1] - top, r[2], r[3]])
            .collect();
        return ("captured", boxes, [right - left, bottom - top]);
    }
    // A row: every picture as tall as the tallest, a twentieth of that between them.
    let height = layers
        .iter()
        .map(|layer| f64::from(layer.size[1]))
        .fold(1.0, f64::max);
    let gap = height * 0.05;
    let mut x = 0.0;
    let boxes = layers
        .iter()
        .map(|layer| {
            let [w, h] = layer.size.map(|side| f64::from(side.max(1)));
            let width = w * height / h;
            let rect = [x, 0.0, width, height];
            x += width + gap;
            rect
        })
        .collect();
    ("row", boxes, [x - gap, height])
}

/// The canvas shape nearest the arranged windows (`record.start create:true`, aspect:auto).
pub(super) fn picture(layers: &[Layer]) -> [u32; 2] {
    let (_, _, [w, h]) = arrange(layers);
    let scale = 1000.0 / w.max(h).max(f64::MIN_POSITIVE);
    [
        (w * scale).round().max(1.0) as u32,
        (h * scale).round().max(1.0) as u32,
    ]
}

/// The canvas `project` has after `project.set {aspect}` (the same rule: the short side stays).
pub(super) fn canvas(project: &Project, aspect: Option<&str>) -> [f64; 2] {
    let [w, h] = project.size.map(f64::from);
    let Some(ratio) = aspect.and_then(|label| {
        Aspect::ALL
            .iter()
            .find(|(_, name)| *name == label)
            .map(|(aspect, _)| aspect.ratio())
    }) else {
        return [w, h];
    };
    let short = w.min(h);
    if ratio >= 1.0 {
        [(short * ratio).round(), short]
    } else {
        [short, (short / ratio).round()]
    }
}

/// Places `layers` on a `canvas` (pixels) whose spine resolves the window look `window`
/// (enabled, merged params) or none.
pub(super) fn plan(
    layers: &[Layer],
    [width, height]: [f64; 2],
    window: Option<&(bool, Vec<splice_model::EffectParam>)>,
) -> Plan {
    let has_window = window.is_some();
    let windowed = window.is_none_or(|(enabled, _)| *enabled);
    // The look a take gets without one pads by the effect's default.
    let padding = match window {
        Some((false, _)) => 0.0,
        _ => window
            .and_then(|(_, params)| params.iter().find(|param| param.name == "padding"))
            .and_then(|param| match param.value {
                splice_model::EffectValue::Number(padding) if padding.is_finite() => Some(padding),
                _ => None,
            })
            .unwrap_or(0.07)
            .clamp(0.0, 0.3),
    };
    let margin = width.min(height) * padding;
    let inner = [margin, margin, width - 2.0 * margin, height - 2.0 * margin];
    let (arrangement, boxes, [ew, eh]) = arrange(layers);
    let scale = (inner[2] / ew).min(inner[3] / eh);
    let origin = [
        inner[0] + (inner[2] - ew * scale) / 2.0,
        inner[1] + (inner[3] - eh * scale) / 2.0,
    ];
    let boxes: Vec<[f64; 4]> = boxes
        .iter()
        .map(|b| {
            [
                origin[0] + b[0] * scale,
                origin[1] + b[1] * scale,
                b[2] * scale,
                b[3] * scale,
            ]
        })
        .collect();
    // Each layer's picture fits inside the padded canvas (as the window look lays it out);
    // its transform scales that about its center and moves it onto its box.
    let transforms = layers
        .iter()
        .zip(&boxes)
        .map(|(layer, b)| {
            let [w, h] = layer.size.map(|side| f64::from(side.max(1)));
            let fit = (inner[2] / w).min(inner[3] / h);
            let fitted = [
                inner[0] + (inner[2] - w * fit) / 2.0,
                inner[1] + (inner[3] - h * fit) / 2.0,
                w * fit,
                h * fit,
            ];
            let round = |v: f64| (v * 1e6).round() / 1e6;
            [
                round((b[0] + b[2] / 2.0 - fitted[0] - fitted[2] / 2.0) / width),
                round((b[1] + b[3] / 2.0 - fitted[1] - fitted[3] / 2.0) / height),
                round(b[2] / fitted[2]).clamp(0.05, 10.0),
            ]
        })
        .collect();
    Plan {
        arrangement,
        boxes,
        transforms,
        padding,
        has_window,
        windowed,
    }
}

/// The batch reference of window `n`'s media entry (`clip.insert {insert:false}`).
pub(super) fn media_ref(n: usize) -> String {
    format!("window{n}")
}

/// Ops after the take's `clip.insert {ref:"take"}` and the window media entries: the window
/// look when the spine has none, each layer's transform, and every other window as a media
/// overlay anchored to the take's first frame with the look (no backdrop of its own: D14).
pub(super) fn ops(layers: &[Layer], plan: &Plan) -> Vec<Json> {
    let transform = |target: &str, [x, y, scale]: [f64; 3]| {
        object(vec![
            ("cmd", text("effect.add")),
            ("target", text(target)),
            ("type", text("transform")),
            (
                "params",
                object(vec![
                    ("x", Json::Float(x)),
                    ("y", Json::Float(y)),
                    ("scale", Json::Float(scale)),
                ]),
            ),
        ])
    };
    let mut ops = Vec::new();
    if !plan.has_window {
        ops.push(object(vec![
            ("cmd", text("effect.add")),
            ("target", text("$take")),
            ("type", text("window")),
        ]));
    }
    ops.push(transform("$take", plan.transforms[0]));
    let length = layers[0].frames;
    for (n, (layer, &moved)) in layers.iter().zip(&plan.transforms).enumerate().skip(1) {
        let item = format!("layer{n}");
        ops.push(object(vec![
            ("cmd", text("overlay.add")),
            ("kind", text("media")),
            ("media", text(&format!("${}", media_ref(n)))),
            (
                "anchor",
                object(vec![
                    ("clip", text("$take")),
                    ("sourceFrame", Json::Signed(0)),
                ]),
            ),
            ("length", Json::Signed(i64::from(layer.frames.min(length)))),
            ("placement", text("full")),
            ("fit", text("fit")),
            ("ref", text(&item)),
        ]));
        if plan.windowed {
            ops.push(object(vec![
                ("cmd", text("effect.add")),
                ("target", text(&format!("${item}"))),
                ("type", text("window")),
                (
                    "params",
                    object(vec![
                        ("background", text("none")),
                        ("padding", Json::Float(plan.padding)),
                    ]),
                ),
            ]));
        }
        ops.push(transform(&format!("${item}"), moved));
    }
    ops
}

/// `take.layers` in the record.stop summary: every window, the item showing it and where it
/// sits on the canvas (x, y, width, height as fractions of the canvas).
pub(super) fn summary(
    layers: &[Layer],
    plan: &Plan,
    canvas: [f64; 2],
    items: &[(splice_model::MediaId, Option<String>)],
) -> Json {
    let list = layers
        .iter()
        .zip(&plan.boxes)
        .zip(items)
        .map(|((layer, b), (media, item))| {
            let round = |v: f64| (v * 1e4).round() / 1e4;
            let mut fields = vec![("file", text(&layer.file))];
            if let Some(window) = layer.window {
                fields.push(("window", text(&format!("win_{window}"))));
            }
            fields.extend([
                ("mediaId", text(&media.to_string())),
                ("item", item.as_deref().map_or(Json::Null, text)),
                (
                    "box",
                    object(vec![
                        ("x", Json::Float(round(b[0] / canvas[0]))),
                        ("y", Json::Float(round(b[1] / canvas[1]))),
                        ("w", Json::Float(round(b[2] / canvas[0]))),
                        ("h", Json::Float(round(b[3] / canvas[1]))),
                    ]),
                ),
            ]);
            object(fields)
        })
        .collect();
    Json::Array(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        record_publish::{publish, stage_path, summary},
        tests::TestDirectory,
        wire::Args,
    };
    use splice_core::history::{Scope, Source, Store, WriteOptions};
    use splice_eval::{Evaluator, Layer as Drawn};
    use splice_model::{JobId, Media, MediaId, MediaKind, Probe, TakeComponents};
    use std::fs;

    fn media(kind: MediaKind, [width, height]: [u32; 2], frames: Frame) -> Media {
        Media {
            id: MediaId::from_serial(0),
            kind,
            path: String::new(),
            bookmark: None,
            size: 8,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames,
                width,
                height,
                audio: Vec::new(),
                codec: Some("hevc".into()),
                loudness: None,
            },
            take: (kind == MediaKind::Take).then(|| TakeComponents {
                screen: Some("screen.mov".into()),
                ..Default::default()
            }),
        }
    }

    fn layer(n: usize, rect: [f64; 4], frames: Frame) -> Layer {
        Layer {
            file: if n == 0 {
                "screen.mov".into()
            } else {
                format!("window-{n}.mov")
            },
            window: Some(40 + n as u32),
            // Retina windows: two pixels per point.
            size: [rect[2] as u32 * 2, rect[3] as u32 * 2],
            rect: Some(rect),
            frames,
        }
    }

    /// Stages `layers` as one take, publishes it on the spine with every other window as an
    /// overlay, and returns where each window is drawn at frame 10 (canvas pixels), the plan
    /// and the record.stop take summary.
    fn record(
        store: &mut Store,
        job: u64,
        layers: &[Layer],
    ) -> (Vec<[f64; 4]>, Plan, Json, WriteOptions) {
        let id = JobId::from_serial(job);
        let staged = stage_path(store.path(), id);
        fs::create_dir_all(&staged).unwrap();
        for layer in layers {
            fs::write(staged.join(&layer.file), b"stand-in").unwrap();
        }
        let session = store.start_session("windows", 1).unwrap().id;
        let project = store.project();
        let window =
            splice_eval::resolve_effect(&[&project.effects, &project.spine.effects], "window");
        let canvas = canvas(project, None);
        let plan = plan(layers, canvas, window.as_ref());
        let write = WriteOptions {
            source: Source::Agent,
            session: Some(session),
            if_revision: Some(project.rev),
            ..Default::default()
        };
        let windows = layers[1..]
            .iter()
            .map(|layer| {
                let mut window = media(MediaKind::Video, layer.size, layer.frames);
                window.path.clone_from(&layer.file);
                window
            })
            .collect();
        let published = publish(
            store,
            id,
            true,
            media(MediaKind::Take, layers[0].size, layers[0].frames),
            windows,
            &write,
            (Vec::new(), vec![("ref", text("take"))], ops(layers, &plan)),
        )
        .unwrap();
        let (take, lints, _) = summary(
            store,
            &published,
            [90, 0],
            None,
            Some((layers, &plan, canvas)),
        );
        assert!(
            lints
                .iter()
                .all(|l| l.field("code").and_then(Json::as_str) != Some("TAKE_ASPECT")),
            "{lints:?}"
        );
        let evaluator = Evaluator::prepare(store.project()).unwrap();
        let mut frame = evaluator.buffer();
        evaluator.frame(10, &mut frame);
        let drawn = frame
            .layers
            .iter()
            .filter_map(|drawn| match drawn {
                Drawn::Picture(picture) => {
                    let r = picture.look.placed(picture.destination);
                    Some([r.x, r.y, r.w, r.h])
                }
                _ => None,
            })
            .collect();
        (drawn, plan, take, write)
    }

    fn assert_near(drawn: &[[f64; 4]], boxes: &[[f64; 4]]) {
        assert_eq!(drawn.len(), boxes.len(), "{drawn:?}");
        for (a, b) in drawn.iter().zip(boxes) {
            assert!(
                a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.05),
                "drawn {a:?}, planned {b:?}"
            );
        }
    }

    #[test]
    fn multi_window_take_places_windows_as_captured_scaled_into_the_canvas() {
        let temp = TestDirectory::new("record-windows");
        let mut store = Store::create(&temp.0.join("p.splice"), &Project::default()).unwrap();
        let canvas = store.project().size.map(f64::from);
        // Editor on the left, preview on the right, a terminal under both: no overlaps.
        let layers = [
            layer(0, [100.0, 50.0, 800.0, 600.0], 90),
            layer(1, [920.0, 50.0, 600.0, 600.0], 90),
            layer(2, [100.0, 670.0, 1420.0, 300.0], 88),
        ];
        let (drawn, plan, take, mut write) = record(&mut store, 1, &layers);
        assert_eq!(plan.arrangement, "captured");
        assert!(!plan.has_window);
        assert_near(&drawn, &plan.boxes);
        // Relative positions and one scale for every window, inside the default 7 % padding.
        let scale = plan.boxes[0][2] / 800.0;
        assert!((plan.boxes[1][0] - plan.boxes[0][0] - 820.0 * scale).abs() < 1e-6);
        assert!((plan.boxes[2][1] - plan.boxes[0][1] - 620.0 * scale).abs() < 1e-6);
        assert!((plan.boxes[2][2] - 1420.0 * scale).abs() < 1e-6);
        let margin = canvas[0].min(canvas[1]) * 0.07;
        let right = plan.boxes.iter().map(|b| b[0] + b[2]).fold(0.0, f64::max);
        let bottom = plan.boxes.iter().map(|b| b[1] + b[3]).fold(0.0, f64::max);
        assert!(
            plan.boxes
                .iter()
                .all(|b| b[0] >= margin - 1e-9 && b[1] >= margin - 1e-9)
        );
        assert!(right <= canvas[0] - margin + 1e-9 && bottom <= canvas[1] - margin + 1e-9);
        // One spine clip, two overlays, three media entries in the take directory.
        let project = store.project();
        assert_eq!(project.clips.order().len(), 1);
        assert_eq!(project.media.len(), 3);
        let id = project.media[0].id;
        assert_eq!(project.media[2].path, format!("takes/{id}/window-2.mov"));
        assert_eq!(project.media[1].kind, MediaKind::Video);
        let mut summary = String::new();
        splice_json::write(&take, &mut summary).unwrap();
        assert!(
            summary.contains(&format!(
                r#""arrangement":"captured","layers":[{{"file":"screen.mov","window":"win_40","mediaId":"{id}""#
            )),
            "{summary}"
        );
        let listed = take.field("layers").unwrap().as_array().unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[2].string("file").unwrap(), "window-2.mov");
        assert!(listed[0].string("item").unwrap().starts_with("clp_"));
        assert!(listed[1].string("item").unwrap().starts_with("ovl_"));
        // The take and its windows are one undo step.
        write.if_revision = Some(store.project().rev);
        store.undo(Scope::Last, &write).unwrap();
        assert!(store.project().media.is_empty() && store.project().clips.order().is_empty());
    }

    #[test]
    fn multi_window_take_stands_overlapping_windows_in_a_row_inside_the_spine_padding() {
        let temp = TestDirectory::new("record-windows-row");
        let mut store = Store::create(&temp.0.join("p.splice"), &Project::default()).unwrap();
        let write = WriteOptions {
            source: Source::Agent,
            session: Some(store.start_session("look", 1).unwrap().id),
            if_revision: Some(0),
            ..Default::default()
        };
        store
            .apply(
                &[splice_json::parse(
                    r#"{"cmd":"effect.add","target":"lan_spine","type":"window","params":{"padding":0.12}}"#,
                )
                .unwrap()],
                &write,
            )
            .unwrap();
        let canvas = store.project().size.map(f64::from);
        let layers = [
            layer(0, [0.0, 0.0, 1200.0, 800.0], 60),
            layer(1, [300.0, 200.0, 800.0, 800.0], 60),
            layer(2, [0.0, 0.0, 600.0, 900.0], 60),
            layer(3, [10.0, 10.0, 600.0, 900.0], 60),
        ];
        let (drawn, plan, _, _) = record(&mut store, 2, &layers);
        assert_eq!(plan.arrangement, "row");
        assert!(plan.has_window && plan.padding == 0.12);
        assert_near(&drawn, &plan.boxes);
        // Equal heights, in source order, the row centered inside the 12 % padding.
        assert!(plan.boxes.windows(2).all(|pair| {
            (pair[0][3] - pair[1][3]).abs() < 1e-9 && pair[0][0] + pair[0][2] < pair[1][0]
        }));
        let margin = canvas[0].min(canvas[1]) * 0.12;
        let last = plan.boxes[3];
        assert!(
            (plan.boxes[0][0] - margin - (canvas[0] - margin - last[0] - last[2])).abs() < 1e-6
        );
        // The spine's look applies: the take gets no window effect of its own.
        let clip = store.project().clips.order()[0];
        let effects = &store.project().clips.data(clip).unwrap().appearance.effects;
        assert!(effects.iter().all(|e| e.kind != "window"));
        // Without bounds (a crash recovery without events) the windows stand in a row too.
        let unknown: Vec<Layer> = layers
            .iter()
            .map(|layer| Layer {
                rect: None,
                ..layer.clone()
            })
            .collect();
        assert_eq!(arrange(&unknown).0, "row");
        let [w, h] = picture(&unknown);
        assert!(w > h * 3);
    }

    #[test]
    fn multi_window_canvas_follows_project_set_aspect() {
        let project = Project::default();
        let [w, h] = project.size.map(f64::from);
        assert_eq!(canvas(&project, None), [w, h]);
        let short = w.min(h);
        assert_eq!(
            canvas(&project, Some("9:16")),
            [short, (short * 16.0 / 9.0).round()]
        );
        assert_eq!(
            canvas(&project, Some("16:10")),
            [(short * 1.6).round(), short]
        );
    }
}
