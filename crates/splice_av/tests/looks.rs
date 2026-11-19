//! 0.3 looks on the GPU path (PLAN.md T16.3, T17.3, T17.5, T18.1–T18.2, T19.2): every check
//! renders a known still and reads pixels back from the same compositor export uses.
#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
mod looks {
    use splice_av::*;
    use splice_model::*;
    use std::{fs, path::PathBuf, time::Duration};

    pub(super) struct Scratch(pub PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    pub(super) fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("splice-looks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// A solid PNG still, registered as image media.
    fn still(dir: &std::path::Path, serial: u64, rgb: [u8; 3]) -> Media {
        let pixels: Vec<u8> = (0..96 * 64)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect();
        let png = apple_sys::image::png(&pixels, [96, 64], 96).unwrap();
        let name = format!("still{serial}.png");
        fs::write(dir.join(&name), png).unwrap();
        Media {
            id: MediaId::from_serial(serial),
            kind: MediaKind::Image,
            path: name,
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 1_000_000,
                width: 96,
                height: 64,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        }
    }

    fn fx(serial: u64, kind: &str, params: Vec<(&str, EffectValue)>) -> Effect {
        Effect {
            id: EffectId::from_serial(serial),
            kind: kind.into(),
            enabled: None,
            params: params
                .into_iter()
                .map(|(name, value)| EffectParam {
                    name: name.into(),
                    value,
                })
                .collect(),
        }
    }

    fn n(v: f64) -> EffectValue {
        EffectValue::Number(v)
    }

    fn s(v: &str) -> EffectValue {
        EffectValue::Text(v.into())
    }

    /// One still clip of `length` frames per (media serial, effects).
    fn project(media: Vec<Media>, clips: Vec<(u64, Frame, Vec<Effect>)>) -> Project {
        let mut project = Project {
            size: [96, 64],
            media,
            ..Project::default()
        };
        let mut start = 0;
        for (serial, (media, length, effects)) in clips.into_iter().enumerate() {
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial as u64 + 1),
                    timing: ClipTiming {
                        start,
                        length,
                        source: 0,
                        speed: 1.0,
                    },
                    data: ClipData {
                        media: Some(MediaId::from_serial(media)),
                        appearance: Appearance {
                            effects,
                            ..Appearance::default()
                        },
                        framing: Framing {
                            crop: None,
                            fit: Fit::Fill,
                        },
                        ..ClipData::default()
                    },
                })
                .unwrap();
            start += length;
        }
        project
    }

    fn pixel(composition: &Composition, frame: Frame, [x, y]: [usize; 2]) -> [u8; 3] {
        let [w, h] = composition.size().map(|n| n as usize);
        let mut rgba = vec![0; w * h * 4];
        composition.read_frame(frame, &mut rgba).unwrap();
        let at = (y * w + x) * 4;
        rgba[at..at + 3].try_into().unwrap()
    }

    fn near(a: [u8; 3], b: [u8; 3], tolerance: u8) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
    }

    pub(super) fn color_lut_masks_and_key_render() {
        let dir = scratch("color");
        let red = still(&dir.0, 1, [220, 30, 30]);
        // Identity LUT (2³).
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    cube.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        fs::write(dir.0.join("identity.cube"), cube).unwrap();
        let plain =
            Composition::prepare(&project(vec![red.clone()], vec![(1, 30, vec![])]), &dir.0)
                .unwrap();
        let base = pixel(&plain, 5, [48, 32]);
        assert!(near(base, [220, 30, 30], 3), "{base:?}");

        let grey = project(
            vec![red.clone()],
            vec![(1, 30, vec![fx(1, "color", vec![("saturation", n(0.0))])])],
        );
        let [r, g, b] = pixel(&Composition::prepare(&grey, &dir.0).unwrap(), 5, [48, 32]);
        assert!(r.abs_diff(g) <= 3 && g.abs_diff(b) <= 3, "{r} {g} {b}");

        let lut = project(
            vec![red.clone()],
            vec![(
                1,
                30,
                vec![fx(1, "color", vec![("lut", s("identity.cube"))])],
            )],
        );
        let through = pixel(&Composition::prepare(&lut, &dir.0).unwrap(), 5, [48, 32]);
        assert!(
            near(through, base, 1),
            "identity LUT {through:?} vs {base:?}"
        );

        let masked = project(
            vec![red.clone()],
            vec![(
                1,
                30,
                vec![fx(
                    1,
                    "mask",
                    vec![
                        ("mode", s("cover")),
                        ("color", s("#00ff00")),
                        ("x", n(0.0)),
                        ("y", n(0.0)),
                        ("w", n(0.5)),
                        ("h", n(0.5)),
                        ("feather", n(0.0)),
                    ],
                )],
            )],
        );
        let composition = Composition::prepare(&masked, &dir.0).unwrap();
        assert!(near(pixel(&composition, 5, [10, 10]), [0, 255, 0], 6));
        assert!(near(pixel(&composition, 5, [80, 50]), base, 3));

        let keyed = project(
            vec![red.clone()],
            vec![(
                1,
                30,
                vec![fx(
                    1,
                    "key",
                    vec![("color", s("#dc1e1e")), ("tolerance", n(0.5))],
                )],
            )],
        );
        let gone = pixel(&Composition::prepare(&keyed, &dir.0).unwrap(), 5, [48, 32]);
        assert!(gone.iter().all(|&c| c < 20), "keyed out: {gone:?}");
    }

    pub(super) fn transform_shapes_and_dissolve_render() {
        let dir = scratch("transform");
        let red = still(&dir.0, 1, [220, 30, 30]);
        let blue = still(&dir.0, 2, [30, 30, 220]);
        let small = project(
            vec![red.clone()],
            vec![(
                1,
                30,
                vec![fx(
                    1,
                    "transform",
                    vec![("scale", n(0.5)), ("rotation", n(0.0))],
                )],
            )],
        );
        let composition = Composition::prepare(&small, &dir.0).unwrap();
        assert!(near(pixel(&composition, 5, [48, 32]), [220, 30, 30], 4));
        assert!(
            pixel(&composition, 5, [4, 4]).iter().all(|&c| c < 12),
            "scaled away from the corner"
        );

        let mut dissolve = project(
            vec![red.clone(), blue.clone()],
            vec![
                (1, 30, vec![]),
                (
                    2,
                    30,
                    vec![fx(
                        1,
                        "transition",
                        vec![("kind", s("dissolve")), ("length", n(20.0))],
                    )],
                ),
            ],
        );
        let composition = Composition::prepare(&dissolve, &dir.0).unwrap();
        let middle = pixel(&composition, 29, [48, 32]);
        assert!(
            middle[0] > 90 && middle[2] > 90,
            "mid-dissolve mixes both: {middle:?}"
        );
        assert!(near(pixel(&composition, 45, [48, 32]), [30, 30, 220], 4));

        // A blue ellipse over the red clip.
        let lane = LaneId::from_serial(9);
        dissolve.lanes.push(Lane {
            id: lane,
            name: "Graphics".into(),
            kind: LaneKind::Graphics,
            effects: Vec::new(),
        });
        dissolve
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 10,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Shape {
                        shape: ShapeKind::Ellipse,
                        rect: Rect {
                            x: 0.25,
                            y: 0.25,
                            w: 0.5,
                            h: 0.5,
                        },
                    },
                    appearance: Appearance {
                        effects: vec![fx(2, "shape", vec![("fill", s("#0000ff"))])],
                        ..Appearance::default()
                    },
                },
            })
            .unwrap();
        let composition = Composition::prepare(&dissolve, &dir.0).unwrap();
        assert!(near(pixel(&composition, 3, [48, 32]), [0, 0, 255], 4));
        assert!(near(pixel(&composition, 3, [2, 2]), [220, 30, 30], 4));
    }

    pub(super) fn audio_chain_renders_offline() {
        let dir = scratch("audio");
        let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/media/stereo.wav");
        let output = dir.0.join("processed.caf");
        let chain = apple_sys::audio::Chain {
            eq: Some([120.0, -3.0, 2.0, 1.0, 1000.0]),
            compressor: Some([-24.0, 4.0, 3.0]),
            gate: Some(-60.0),
            isolation: None,
            pan: -0.5,
        };
        apple_sys::audio::render(&input, 0, &output, &chain).unwrap();
        let probe = apple_sys::media::probe(&output).unwrap();
        assert_eq!(probe.audio.first().and_then(|t| t.channels), Some(2));
        let envelope = apple_sys::audio::envelope(&output, 0, 100.0).unwrap();
        assert!(!envelope.is_empty());
    }

    pub(super) fn ocr_reads_text_and_a_blur_mask_hides_it() {
        let dir = scratch("ocr");
        // A "screen" still with black text on white, like a recorded window.
        let size = [640u32, 360];
        let text = apple_sys::composition::text_rgba(
            &apple_sys::composition::TextLayer {
                text: "SECRET 4821".into(),
                bounds: [0.0, 0.0, 640.0, 120.0],
                font: 64.0,
                opacity: [1.0; 2],
                color: 0x000000,
                style: apple_sys::composition::TextStyle::default(),
                visible: None,
            },
            [640, 120],
        )
        .unwrap();
        let mut pixels = vec![250u8; (size[0] * size[1] * 4) as usize];
        for y in 0..120usize {
            for x in 0..640usize {
                let s = (y * 640 + x) * 4;
                let d = ((y + 120) * 640 + x) * 4;
                let a = u32::from(text[s + 3]);
                for c in 0..3 {
                    pixels[d + c] = ((u32::from(text[s + c]) * a + 250 * (255 - a)) / 255) as u8;
                }
            }
        }
        fs::write(
            dir.0.join("screen.png"),
            apple_sys::image::png(&pixels, size, 640).unwrap(),
        )
        .unwrap();
        let mut screen = still(&dir.0, 1, [250, 250, 250]);
        screen.path = "screen.png".into();
        (screen.probe.width, screen.probe.height) = (640, 360);
        let mut project = project(vec![screen], vec![(1, 6, vec![])]);
        project.size = size;
        let export = |project: &Project, name: &str| {
            let composition = Composition::prepare(project, &dir.0).unwrap();
            let file = dir.0.join(name);
            let mut job = composition.start_export(&file, false).unwrap();
            while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
            assert_eq!(job.status(), ExportState::Complete);
            apple_sys::vision::analyze(&file, apple_sys::vision::TEXT, 2.0)
                .unwrap()
                .into_iter()
                .filter_map(|f| match f {
                    apple_sys::vision::Finding::Text { text, .. } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let seen = export(&project, "plain.mp4");
        assert!(seen.contains("4821"), "OCR read: {seen:?}");
        // Redaction (T18.2 acceptance): a pixelate mask over the text band leaves nothing to read.
        let slot = project.clips.order()[0];
        project
            .clips
            .data_mut(slot)
            .unwrap()
            .appearance
            .effects
            .push(fx(
                6,
                "mask",
                vec![
                    ("mode", s("pixelate")),
                    ("x", n(0.0)),
                    ("y", n(0.3)),
                    ("w", n(1.0)),
                    ("h", n(0.4)),
                    ("amount", n(1.0)),
                ],
            ));
        let hidden = export(&project, "redacted.mp4");
        assert!(!hidden.contains("4821"), "still readable: {hidden:?}");
    }
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[
        (
            "looks_color_lut_masks_and_key_render",
            looks::color_lut_masks_and_key_render,
        ),
        (
            "looks_transform_shapes_and_dissolve_render",
            looks::transform_shapes_and_dissolve_render,
        ),
        (
            "looks_audio_chain_renders_offline",
            looks::audio_chain_renders_offline,
        ),
        (
            "looks_ocr_reads_text_and_a_blur_mask_hides_it",
            looks::ocr_reads_text_and_a_blur_mask_hides_it,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
