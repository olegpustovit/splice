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
        // T18.1: an identity LUT is bit-exact.
        assert_eq!(through, base, "identity LUT");

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

        // D14: the incoming side is flattened, then faded once. Per-layer opacity let the
        // incoming backdrop (green) bleed through the incoming window as t·(1−t).
        let windowed = |kind: &str| {
            let mut project = project(
                vec![red.clone(), blue.clone()],
                vec![
                    (1, 30, vec![]),
                    (
                        2,
                        30,
                        vec![fx(
                            1,
                            "transition",
                            vec![
                                ("kind", s(kind)),
                                ("length", n(20.0)),
                                ("direction", s("left")),
                            ],
                        )],
                    ),
                ],
            );
            project.effects.push(fx(
                30,
                "window",
                vec![
                    ("padding", n(0.2)),
                    ("corner", n(0.0)),
                    ("shadow", n(0.0)),
                    ("background", s("color")),
                    ("color", s("#00c800")),
                ],
            ));
            Composition::prepare(&project, &dir.0).unwrap()
        };
        let composition = windowed("dissolve");
        let middle = pixel(&composition, 29, [48, 32]);
        assert!(
            middle[0] > 90 && middle[2] > 90 && middle[1] < 45,
            "window content mixes without backdrop bleed: {middle:?}"
        );
        assert!(
            near(pixel(&composition, 29, [2, 2]), [0, 200, 0], 4),
            "one backdrop through the transition"
        );
        // A dip between windowed clips goes to the shared backdrop, not black.
        // At frame 29 (t = 0.475) only 5% of the outgoing content is left over it.
        let dipped = pixel(&windowed("dip"), 29, [48, 32]);
        assert!(near(dipped, [11, 192, 2], 4), "dip: {dipped:?}");
        for kind in [
            "wipe",
            "push",
            "slide",
            "zoom",
            "whip",
            "zoomPunch",
            "swipe",
        ] {
            let composition = windowed(kind);
            // The backdrop never moves or doubles; inside, every pixel is a clip or the backdrop.
            // A punch to 1.6× covers the corner with content (its backdrop staying put is
            // content_only_transitions_never_move_the_backdrop in splice_eval).
            let corner = pixel(&composition, 30, [1, 1]);
            assert!(
                kind == "zoomPunch" || near(corner, [0, 200, 0], 4),
                "{kind}: corner {corner:?}"
            );
            for x in [28, 48, 66] {
                let p = pixel(&composition, 30, [x, 32]);
                let pure = [[220, 30, 30], [30, 30, 220], [0, 200, 0]]
                    .iter()
                    .any(|&c| near(p, c, 6));
                assert!(
                    pure || matches!(kind, "zoom" | "zoomPunch"),
                    "{kind} at x {x}: {p:?}"
                );
            }
        }

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

    /// T19.2: the cleanup chain drops a −40 dBFS noise floor by at least 15 dB between phrases
    /// and keeps the phrases' level within 1 dB.
    pub(super) fn cleanup_chain_lowers_the_noise_floor_and_keeps_speech() {
        let dir = scratch("noise");
        let rate = 48_000u32;
        let mut seed = 0x1234_5678_u32;
        let mut samples = Vec::new();
        // 0.5 s phrases (a voiced 180 Hz tone with harmonics at about -14 dBFS RMS) and 0.5 s
        // pauses of -40 dBFS white noise, eight times.
        for _ in 0..8 {
            for i in 0..rate / 2 {
                let t = f64::from(i) / f64::from(rate);
                let voiced = (0.5 * (2.0 * std::f64::consts::PI * 180.0 * t).sin()
                    + 0.2 * (2.0 * std::f64::consts::PI * 360.0 * t).sin())
                    * 0.3;
                samples.push(voiced as f32);
            }
            for _ in 0..rate / 2 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let white =
                    (f64::from(seed) / f64::from(u32::MAX) * 2.0 - 1.0) * 0.01 * 3f64.sqrt();
                samples.push(white as f32);
            }
        }
        let mut wav = Vec::new();
        let data = (samples.len() * 2) as u32;
        for chunk in [
            &b"RIFF"[..],
            &(36 + data).to_le_bytes(),
            b"WAVEfmt ",
            &16u32.to_le_bytes(),
            &1u16.to_le_bytes(),
            &1u16.to_le_bytes(),
            &rate.to_le_bytes(),
            &(rate * 2).to_le_bytes(),
            &2u16.to_le_bytes(),
            &16u16.to_le_bytes(),
            b"data",
            &data.to_le_bytes(),
        ] {
            wav.extend_from_slice(chunk);
        }
        for s in &samples {
            wav.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let input = dir.0.join("noisy.wav");
        fs::write(&input, wav).unwrap();
        let output = dir.0.join("clean.caf");
        let chain = apple_sys::audio::Chain {
            eq: Some([80.0, 0.0, 0.0, 0.0, 1000.0]),
            compressor: None,
            gate: Some(-30.0),
            isolation: None,
            pan: 0.0,
        };
        apple_sys::audio::render(&input, 0, &output, &chain).unwrap();
        let level = |file: &std::path::Path, phrase: bool| {
            let envelope = apple_sys::audio::envelope(file, 0, 100.0).unwrap();
            // Middles of each half second, away from the gate's attack and release.
            let picked: Vec<f64> = (0..8)
                .flat_map(|n| {
                    let base = n * 100 + if phrase { 0 } else { 50 };
                    (base + 15..base + 35).map(|i| f64::from(envelope[i]))
                })
                .collect();
            let rms = (picked.iter().map(|v| v * v).sum::<f64>() / picked.len() as f64).sqrt();
            20.0 * rms.max(1e-9).log10()
        };
        let (speech_in, speech_out) = (level(&input, true), level(&output, true));
        let (noise_in, noise_out) = (level(&input, false), level(&output, false));
        eprintln!(
            "speech {speech_in:.1} → {speech_out:.1} dB, noise {noise_in:.1} → {noise_out:.1} dB"
        );
        assert!(
            noise_in - noise_out >= 15.0,
            "noise floor fell {:.1} dB",
            noise_in - noise_out
        );
        assert!(
            (speech_in - speech_out).abs() <= 1.0,
            "speech moved {:.1} dB",
            speech_out - speech_in
        );
    }

    /// T20.1: Vision feature prints jump at a hard cut in an exported movie.
    pub(super) fn shot_analysis_finds_a_hard_cut() {
        let dir = scratch("shots");
        let mut project = project(
            vec![
                still(&dir.0, 1, [30, 80, 220]),
                still(&dir.0, 2, [240, 160, 30]),
            ],
            vec![(1, 45, vec![]), (2, 45, vec![])],
        );
        project.size = [320, 180];
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let file = dir.0.join("cut.mp4");
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::H264))
            .unwrap();
        while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
        assert_eq!(job.status(), ExportState::Complete);
        let prints: Vec<(f64, f64)> =
            apple_sys::vision::analyze(&file, apple_sys::vision::SHOTS, 0.25)
                .unwrap()
                .into_iter()
                .filter_map(|finding| match finding {
                    apple_sys::vision::Finding::Print { t, distance } => Some((t, distance)),
                    _ => None,
                })
                .collect();
        let (at, jump) = prints
            .iter()
            .copied()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let calm = prints
            .iter()
            .filter(|p| (p.0 - at).abs() > 0.3)
            .map(|p| p.1)
            .fold(0.0, f64::max);
        eprintln!("shot jump {jump:.3} at {at:.2} s (others ≤ {calm:.3})");
        assert!((at - 1.5).abs() <= 0.25, "cut found at {at} s");
        assert!(jump > 3.0 * calm.max(0.01), "jump {jump} vs {calm}");
    }

    /// A still image as a media overlay (a logo) composites over the picture.
    pub(super) fn image_overlay_renders() {
        let dir = scratch("image-overlay");
        let mut project = project(
            vec![
                still(&dir.0, 1, [30, 60, 200]),
                still(&dir.0, 2, [250, 250, 250]),
            ],
            vec![(1, 30, vec![])],
        );
        project.lanes.push(Lane {
            id: LaneId::from_serial(31),
            name: String::new(),
            kind: LaneKind::Overlay,
            effects: Vec::new(),
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(41),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(5),
                    length: 20,
                },
                data: ItemData {
                    lane: LaneId::from_serial(31),
                    kind: ItemKind::Media(Overlay {
                        media: MediaId::from_serial(2),
                        source: 0,
                        placement: Placement::Full,
                        behind_presenter: false,
                        muted: true,
                        framing: Framing {
                            crop: None,
                            fit: Fit::Fill,
                        },
                    }),
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        assert!(near(pixel(&composition, 2, [48, 32]), [30, 60, 200], 6));
        assert!(near(pixel(&composition, 10, [48, 32]), [250, 250, 250], 6));
    }

    /// A committed camera-footage fixture (fixtures/media/camera-footage.swift) as video media.
    fn footage(dir: &std::path::Path, serial: u64, name: &str) -> Media {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/media")
            .join(name);
        fs::copy(&source, dir.join(name)).unwrap();
        let probe = apple_sys::media::probe(&dir.join(name)).unwrap();
        let video = probe.video.unwrap();
        let fps = video.nominal_fps.unwrap_or(30.0);
        Media {
            id: MediaId::from_serial(serial),
            kind: MediaKind::Video,
            path: name.into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps,
                frames: (probe.duration_seconds * fps).round() as Frame,
                width: video.width,
                height: video.height,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        }
    }

    /// A Rec. 709-tagged value in the sRGB working space: Apple decodes BT.709 with a 1.961
    /// display gamma.
    fn rec709_to_srgb(value: u8) -> u8 {
        let linear = (f64::from(value) / 255.0).powf(1.961);
        (255.0 * (1.055 * linear.powf(1.0 / 2.4) - 0.055)).round() as u8
    }

    /// T15.2: phone and camera footage. A portrait clip renders upright, variable frame rate
    /// sources land on integer project frames by time, and Rec. 709 and Display P3 footage
    /// convert to the sRGB working space.
    pub(super) fn portrait_vfr_and_p3_footage_render_correctly() {
        let dir = scratch("footage");
        // Portrait: stored landscape (left red, right blue) with a 90° transform.
        let media = footage(&dir.0, 1, "portrait.mov");
        assert_eq!([media.probe.width, media.probe.height], [36, 64]);
        let mut upright = project(vec![media], vec![(1, 10, vec![])]);
        upright.size = [36, 64];
        let composition = Composition::prepare(&upright, &dir.0).unwrap();
        let (top, bottom) = (
            pixel(&composition, 3, [18, 12]),
            pixel(&composition, 3, [18, 52]),
        );
        assert!(
            near(top, [220, 30, 30], 30) && near(bottom, [30, 30, 220], 30),
            "{top:?} {bottom:?}"
        );
        // VFR: frame n (grey 20 + 8n) starts at the cumulative sum of 1001, 2002, 500 / 30000 s.
        let mut vfr = project(vec![footage(&dir.0, 2, "vfr.mov")], vec![(2, 20, vec![])]);
        vfr.size = [32, 18];
        let composition = Composition::prepare(&vfr, &dir.0).unwrap();
        let starts: Vec<f64> = (0..24)
            .scan(0i64, |at, n| {
                let start = *at;
                *at += [1001, 2002, 500][n % 3];
                Some(start as f64 / 30000.0)
            })
            .collect();
        for frame in 0..20 {
            let t = f64::from(frame) / 30.0;
            // By time, not by index: the frame showing at t, or the next one when it starts
            // within 1 ms (decoders snap to the nearest sample).
            let shown = starts.iter().rposition(|&s| s <= t + 1e-6).unwrap();
            let near_next = starts.get(shown + 1).is_some_and(|&s| s - t < 0.001);
            let [grey, ..] = pixel(&composition, frame, [16, 9]);
            let matches = |n: usize| grey.abs_diff(rec709_to_srgb(20 + 8 * n as u8)) <= 4;
            assert!(
                matches(shown) || (near_next && matches(shown + 1)),
                "frame {frame}: grey {grey}, want source frame {shown}"
            );
        }
        // P3: (200, 120, 60) with P3 primaries and the BT.709 transfer is about (219, 127, 54)
        // in sRGB; converting only the transfer (primaries passed through) gives (205, 129, 69).
        let mut p3 = project(vec![footage(&dir.0, 3, "p3.mov")], vec![(3, 10, vec![])]);
        p3.size = [32, 18];
        let [r, g, b] = pixel(&Composition::prepare(&p3, &dir.0).unwrap(), 3, [16, 9]);
        eprintln!("P3 (200, 120, 60) rendered as ({r}, {g}, {b})");
        assert!(
            near([r, g, b], [219, 127, 54], 3),
            "P3 not converted: ({r}, {g}, {b})"
        );
    }

    /// H8, T36.5: a thick caption stroke outlines the glyphs instead of covering them, two
    /// lines that do not fit shrink instead of dropping the second, and the spoken word draws
    /// T45.1: html.add's page renders on a virtual clock into a movie the compositor plays:
    /// a CSS animation shows its state at each frame, and a page that draws a frame
    /// differently the second time (randomness) is refused.
    pub(super) fn t45_html_pages_render_deterministically() {
        let dir = scratch("html");
        let page = "<style>html,body{margin:0;height:100%}body{animation:c 1s linear}\
            @keyframes c{0%,49%{background:#f00}50%,100%{background:#00f}}</style>";
        let movie = dir.0.join("page.mov");
        apple_sys::html::render(page, None, [96, 64], 30, 30, &movie).unwrap();
        let mut media = still(&dir.0, 1, [0, 0, 0]);
        media.kind = MediaKind::Video;
        media.path = "page.mov".into();
        media.probe.frames = 30;
        let composition =
            Composition::prepare(&project(vec![media], vec![(1, 30, vec![])]), &dir.0).unwrap();
        let at = |frame| pixel(&composition, frame, [48, 32]);
        let [r, _, b] = at(3);
        assert!(r > 200 && b < 60, "red at 0.1 s: {:?}", at(3));
        let [r, _, b] = at(25);
        assert!(b > 200 && r < 60, "blue at 0.83 s: {:?}", at(25));
        let random = "<body></body><script>requestAnimationFrame(function f(){\
            document.body.style.background='rgb('+(Math.random()*255|0)+',0,0)';\
            requestAnimationFrame(f)})</script>";
        let error = apple_sys::html::render(random, None, [96, 64], 30, 10, &dir.0.join("r.mov"))
            .unwrap_err()
            .to_string();
        assert!(error.contains("not deterministic"), "{error}");
    }

    /// M23: an animated GIF sticker plays its frames at their own delays and loops, instead of
    /// showing its first frame for the whole item.
    pub(super) fn m23_animated_gifs_play_and_loop() {
        let dir = scratch("gif");
        // 1x1, two frames from a red/blue palette, 0.5 s each (hand-encoded LZW, code size 2).
        let frame = |index_code: u8| {
            let mut f = vec![0x21, 0xf9, 0x04, 0x00, 50, 0x00, 0x00, 0x00];
            f.extend([0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
            f.extend([0x02, 0x02, index_code, 0x01, 0x00]);
            f
        };
        let mut gif = b"GIF89a".to_vec();
        gif.extend([1, 0, 1, 0, 0x80, 0, 0, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff]);
        gif.extend(frame(0x44)); // palette 0: red
        gif.extend(frame(0x4c)); // palette 1: blue
        gif.push(0x3b);
        fs::write(dir.0.join("sticker.gif"), gif).unwrap();
        let mut media = still(&dir.0, 1, [0, 0, 0]);
        media.path = "sticker.gif".into();
        (media.probe.width, media.probe.height) = (1, 1);
        let composition =
            Composition::prepare(&project(vec![media], vec![(1, 60, vec![])]), &dir.0).unwrap();
        let red = |frame| {
            let p = pixel(&composition, frame, [48, 32]);
            p[0] > 200 && p[2] < 60
        };
        let blue = |frame| {
            let p = pixel(&composition, frame, [48, 32]);
            p[2] > 200 && p[0] < 60
        };
        assert!(red(5), "frame 0 at 0.17 s");
        assert!(blue(20), "frame 1 at 0.67 s");
        assert!(red(35), "loops at 1.17 s");
    }

    /// T45.2: a Lottie overlay plays its frames through ThorVG at its own rate and loops, and
    /// where it draws nothing the picture beneath shows through (ProRes 4444 keeps the alpha).
    pub(super) fn lottie_overlays_play_loop_and_stay_transparent() {
        let dir = scratch("lottie");
        fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../thorvg_sys/fixtures/square.json"),
            dir.0.join("square.json"),
        )
        .unwrap();
        // A 16 px red square moves from (8, 32) to (56, 32) over 30 frames of a 64 x 64 canvas.
        let mut lottie = still(&dir.0, 2, [0, 0, 0]);
        lottie.path = "square.json".into();
        lottie.probe.codec = Some("lottie".into());
        (lottie.probe.width, lottie.probe.height) = (64, 64);
        let mut project = project(
            vec![still(&dir.0, 1, [30, 60, 200]), lottie],
            vec![(1, 60, vec![])],
        );
        project.lanes.push(Lane {
            id: LaneId::from_serial(31),
            name: String::new(),
            kind: LaneKind::Overlay,
            effects: Vec::new(),
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(41),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 60,
                },
                data: ItemData {
                    lane: LaneId::from_serial(31),
                    kind: ItemKind::Media(Overlay {
                        media: MediaId::from_serial(2),
                        source: 0,
                        placement: Placement::Full,
                        behind_presenter: false,
                        muted: true,
                        framing: Framing {
                            crop: None,
                            fit: Fit::Fill,
                        },
                    }),
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        // Filled into 96 x 64, the Lottie is scaled 1.5x: its x lands at 1.5 x on the canvas.
        let red = |frame, x| {
            let p = pixel(&composition, frame, [x, 32]);
            p[0] > 200 && p[2] < 60
        };
        let blue = |frame, x| near(pixel(&composition, frame, [x, 32]), [30, 60, 200], 12);
        assert!(red(1, 12) && blue(1, 74), "starts on the left");
        assert!(red(25, 74) && blue(25, 12), "moves right by frame 25");
        assert!(red(31, 12) && blue(31, 74), "loops after 30 frames");
        for frame in [1, 25, 31] {
            let p = pixel(&composition, frame, [48, 4]);
            assert!(near(p, [30, 60, 200], 12), "transparent at {frame}: {p:?}");
        }
    }

    /// M22: a title animated per character draws every character (120 units is past the old
    /// 64-layer cap, which dropped the rest silently): once the animation ends it covers the
    /// same pixels as the static title.
    pub(super) fn m22_per_character_titles_draw_every_character() {
        let dir = scratch("letters");
        let black = still(&dir.0, 1, [0, 0, 0]);
        let titled = |animate: bool| {
            let mut project = project(vec![black.clone()], vec![(1, 30, vec![])]);
            project.size = [640, 360];
            let lane = LaneId::from_serial(9);
            project.lanes.push(Lane {
                id: lane,
                name: "Graphics".into(),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            });
            let effects = if animate {
                vec![fx(
                    3,
                    "animate",
                    vec![
                        ("preset", s("fade")),
                        ("unit", s("character")),
                        ("duration", n(1.0)),
                        ("stagger", n(0.0)),
                    ],
                )]
            } else {
                Vec::new()
            };
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(2),
                    timing: ItemTiming {
                        anchor: Anchor::Pinned(0),
                        length: 30,
                    },
                    data: ItemData {
                        lane,
                        kind: ItemKind::Title {
                            text: "ABCDEFGHIJ".repeat(12),
                        },
                        appearance: Appearance {
                            effects,
                            ..Appearance::default()
                        },
                    },
                })
                .unwrap();
            let composition = Composition::prepare(&project, &dir.0).unwrap();
            let mut rgba = vec![0; 640 * 360 * 4];
            composition.read_frame(20, &mut rgba).unwrap();
            rgba.chunks_exact(4).filter(|p| p[0] > 128).count()
        };
        let (still_text, animated) = (titled(false), titled(true));
        assert!(still_text > 500, "{still_text}");
        // Per-unit rasters anti-alias their edges separately (a few % fewer bright pixels at
        // this size); dropping units at the old cap lost about half.
        assert!(
            animated * 10 >= still_text * 9 && animated <= still_text,
            "animated {animated} vs static {still_text}"
        );
    }

    /// H7: a window backing draws its own evaluated values; the picture's look (here half
    /// opacity) never applies to it a second time.
    pub(super) fn h7_the_backing_never_borrows_the_picture_look() {
        let dir = scratch("backing");
        let red = still(&dir.0, 1, [220, 30, 30]);
        let mut project = project(
            vec![red],
            vec![(1, 10, vec![fx(1, "transform", vec![("opacity", n(0.5))])])],
        );
        project.effects.push(fx(
            2,
            "window",
            vec![
                ("padding", n(0.2)),
                ("corner", n(0.0)),
                ("shadow", n(0.0)),
                ("background", s("color")),
                ("color", s("#00c800")),
            ],
        ));
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let corner = pixel(&composition, 5, [2, 2]);
        assert!(near(corner, [0, 200, 0], 4), "backdrop {corner:?}");
        // The card itself is half red over the green backdrop.
        let card = pixel(&composition, 5, [48, 32]);
        assert!(near(card, [110, 115, 15], 8), "card {card:?}");
    }

    /// Recorder parity (inset): the window `inset` draws an opaque inner border of `insetColor`
    /// (black by default) over the picture's edge, inside the card; 0 draws none.
    pub(super) fn window_inset_draws_an_inner_border() {
        let dir = scratch("inset");
        let grey = still(&dir.0, 1, [128, 128, 128]);
        let render = |extra: Vec<(&'static str, EffectValue)>| {
            let mut project = project(vec![grey.clone()], vec![(1, 10, vec![])]);
            // 960 x 640, padding 0.25, fill: the card is 640 x 320 at (160, 160); 0.02 is 6.4 px.
            project.size = [960, 640];
            let mut params = vec![
                ("padding", n(0.25)),
                ("corner", n(0.0)),
                ("shadow", n(0.0)),
                ("background", s("color")),
                ("color", s("#00c800")),
            ];
            params.extend(extra);
            project.effects.push(fx(9, "window", params));
            Composition::prepare(&project, &dir.0).unwrap()
        };
        let plain = render(vec![]);
        assert!(near(pixel(&plain, 5, [162, 320]), [128, 128, 128], 4));
        let black = render(vec![("inset", n(0.02))]);
        for at in [[162, 320], [480, 162], [797, 320], [480, 477]] {
            let p = pixel(&black, 5, at);
            assert!(
                near(p, [0, 0, 0], 4),
                "default inset is black at {at:?}: {p:?}"
            );
        }
        assert!(
            near(pixel(&black, 5, [170, 320]), [128, 128, 128], 4),
            "inside the border"
        );
        assert!(
            near(pixel(&black, 5, [156, 320]), [0, 200, 0], 4),
            "backdrop outside"
        );
        let red = render(vec![("inset", n(0.02)), ("insetColor", s("#ff0000"))]);
        let p = pixel(&red, 5, [162, 320]);
        assert!(near(p, [255, 0, 0], 4), "insetColor: {p:?}");
    }

    /// A 96 × 64 PNG still: left half red, right half blue.
    fn split_still(dir: &std::path::Path, serial: u64) -> Media {
        let pixels: Vec<u8> = (0..96 * 64)
            .flat_map(|i| {
                if i % 96 < 48 {
                    [220, 30, 30, 255]
                } else {
                    [30, 30, 220, 255]
                }
            })
            .collect();
        let name = format!("split{serial}.png");
        fs::write(
            dir.join(&name),
            apple_sys::image::png(&pixels, [96, 64], 96).unwrap(),
        )
        .unwrap();
        Media {
            path: name,
            ..still(dir, serial, [0, 0, 0])
        }
    }

    /// Exports `project` (in `dir`) as an H.264 movie named `name`.
    fn movie(project: &Project, dir: &std::path::Path, name: &str) -> PathBuf {
        let composition = Composition::prepare(project, dir).unwrap();
        let file = dir.join(name);
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::H264))
            .unwrap();
        while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
        assert_eq!(job.status(), ExportState::Complete, "{name}");
        file
    }

    /// T46.3: an image backdrop cover-fits and blurs, a gradient follows its angle, and the
    /// camera bubble mirrors, sits where x/y put it and eases to the full canvas or away.
    pub(super) fn t46_backdrops_and_the_camera_bubble_render() {
        let dir = scratch("t46");
        let grey = still(&dir.0, 2, [128, 128, 128]);
        let window = |extra: Vec<(&'static str, EffectValue)>| {
            let mut params = vec![("padding", n(0.25)), ("corner", n(0.0)), ("shadow", n(0.0))];
            params.extend(extra);
            fx(9, "window", params)
        };
        // The split image behind a grey card: each half shows in the padding.
        let backdrop = |blur: f64| {
            let mut project = project(
                vec![grey.clone(), split_still(&dir.0, 1)],
                vec![(2, 10, vec![])],
            );
            project.effects.push(window(vec![
                ("background", s("image")),
                ("image", s(&MediaId::from_serial(1).to_string())),
                ("blur", n(blur)),
            ]));
            Composition::prepare(&project, &dir.0).unwrap()
        };
        let sharp = backdrop(0.0);
        let (left, right) = (pixel(&sharp, 5, [2, 4]), pixel(&sharp, 5, [93, 4]));
        assert!(near(left, [220, 30, 30], 6) && near(right, [30, 30, 220], 6));
        assert!(near(pixel(&sharp, 5, [46, 4]), [220, 30, 30], 6));
        // Blurred, the edge mixes both halves; far from it the colors hold.
        let soft = backdrop(1.0);
        let edge = pixel(&soft, 5, [46, 4]);
        assert!(edge[2] > 60 && edge[0] < 200, "blurred edge {edge:?}");
        assert!(near(pixel(&soft, 5, [2, 32]), [220, 30, 30], 12));
        // A 90° gradient runs top to bottom: rows are uniform.
        let mut angled = project(vec![grey.clone()], vec![(2, 10, vec![])]);
        angled.effects.push(window(vec![
            ("color", s("#ff0000")),
            ("color2", s("#0000ff")),
            ("angle", n(90.0)),
        ]));
        let angled = Composition::prepare(&angled, &dir.0).unwrap();
        let (top, bottom) = (pixel(&angled, 5, [48, 1]), pixel(&angled, 5, [48, 62]));
        assert!(top[0] > 220 && bottom[2] > 220, "{top:?} {bottom:?}");
        let (a, b) = (pixel(&angled, 5, [2, 32]), pixel(&angled, 5, [93, 32]));
        assert!(
            near(a, b, 3),
            "a row of a 90° gradient is uniform: {a:?} {b:?}"
        );

        // A take: grey screen, the split picture as its camera.
        let take = dir.0.join("takes/med_7");
        fs::create_dir_all(&take).unwrap();
        let screen = project(vec![grey.clone()], vec![(2, 30, vec![])]);
        fs::rename(
            movie(&screen, &dir.0, "screen.mov"),
            take.join("screen.mov"),
        )
        .unwrap();
        let camera = project(vec![split_still(&dir.0, 1)], vec![(1, 30, vec![])]);
        fs::rename(
            movie(&camera, &dir.0, "camera.mov"),
            take.join("camera.mov"),
        )
        .unwrap();
        let bubble = |params: Vec<(&'static str, EffectValue)>| {
            let mut project = project(vec![], vec![]);
            project.media = vec![Media {
                id: MediaId::from_serial(7),
                kind: MediaKind::Take,
                path: "takes/med_7".into(),
                bookmark: None,
                size: 0,
                mtime: 0,
                probe: Probe {
                    fps: 30.0,
                    frames: 30,
                    width: 96,
                    height: 64,
                    audio: Vec::new(),
                    codec: None,
                    loudness: None,
                },
                take: Some(TakeComponents {
                    screen: Some("screen.mov".into()),
                    camera: Some("camera.mov".into()),
                    ..TakeComponents::default()
                }),
            }];
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(1),
                    timing: ClipTiming {
                        start: 0,
                        length: 30,
                        source: 0,
                        speed: 1.0,
                    },
                    data: ClipData {
                        media: Some(MediaId::from_serial(7)),
                        enabled: ComponentStates {
                            screen: Some(true),
                            camera: Some(true),
                            ..ComponentStates::default()
                        },
                        appearance: Appearance {
                            effects: vec![fx(3, "camera", params)],
                            ..Appearance::default()
                        },
                        ..ClipData::default()
                    },
                })
                .unwrap();
            Composition::prepare(&project, &dir.0).unwrap()
        };
        // Centered, half the short side, square: the bubble covers 32..64 × 16..48 and shows the
        // middle of the camera (red left of its center, blue right).
        let centered = vec![
            ("x", n(0.5)),
            ("y", n(0.5)),
            ("size", n(0.5)),
            ("roundness", n(0.0)),
            ("shadow", n(0.0)),
        ];
        let plain = bubble(centered.clone());
        assert!(near(pixel(&plain, 5, [40, 32]), [220, 30, 30], 30));
        assert!(near(pixel(&plain, 5, [56, 32]), [30, 30, 220], 30));
        assert!(
            near(pixel(&plain, 5, [20, 32]), [128, 128, 128], 12),
            "screen beside it"
        );
        let mut mirrored = centered.clone();
        mirrored.push(("mirror", EffectValue::Bool(true)));
        let mirrored = bubble(mirrored);
        assert!(near(pixel(&mirrored, 5, [40, 32]), [30, 30, 220], 30));
        assert!(near(pixel(&mirrored, 5, [56, 32]), [220, 30, 30], 30));
        // Full from frame 10 (a cut with hold): the camera covers the canvas; hidden shows the
        // screen where the bubble was.
        let mut full = centered.clone();
        full.push((
            "layout",
            EffectValue::Keys(vec![
                Key {
                    at: 0,
                    value: 0.0,
                    ease: Ease::Hold,
                },
                Key {
                    at: 10,
                    value: 1.0,
                    ease: Ease::Hold,
                },
            ]),
        ));
        let full = bubble(full);
        assert!(near(pixel(&full, 5, [10, 32]), [128, 128, 128], 12));
        assert!(near(pixel(&full, 15, [10, 32]), [220, 30, 30], 30));
        assert!(near(pixel(&full, 15, [86, 32]), [30, 30, 220], 30));
        let mut hidden = centered;
        hidden.push(("layout", s("hidden")));
        let hidden = bubble(hidden);
        assert!(near(pixel(&hidden, 5, [40, 32]), [128, 128, 128], 12));
    }

    /// in the active color.
    pub(super) fn h8_caption_stroke_sits_under_the_fill_and_the_active_word_is_colored() {
        use apple_sys::composition::{TextLayer, TextStyle, text_rgba};
        let size = [400u32, 120];
        let layer = |text: &str, active: Option<([usize; 2], u32)>| TextLayer {
            text: text.into(),
            bounds: [0.0, 0.0, 400.0, 120.0],
            font: 80.0,
            opacity: [1.0; 2],
            color: 0xffffff,
            style: TextStyle {
                weight: 5,
                stroke: 0x000000,
                stroke_width: 0.14,
                ..TextStyle::default()
            },
            visible: None,
            active,
        };
        let count = |pixels: &[u8], test: &dyn Fn(&[u8]) -> bool, half: Option<bool>| {
            pixels
                .chunks_exact(4)
                .enumerate()
                .filter(|(i, p)| {
                    let right = (i % size[0] as usize) >= size[0] as usize / 2;
                    half.is_none_or(|h| h == right) && p[3] > 250 && test(p)
                })
                .count()
        };
        let white = |p: &[u8]| p[0] > 240 && p[1] > 240 && p[2] > 240;
        let red = |p: &[u8]| p[0] > 200 && p[1] < 60 && p[2] < 60;
        let pixels = text_rgba(&layer("HI", None), size).unwrap();
        let (fill, outline) = (
            count(&pixels, &white, None),
            count(&pixels, &|p| p[0] < 20 && p[1] < 20 && p[2] < 20, None),
        );
        // Stroke over fill left almost no white in bold 80 px glyphs.
        assert!(
            fill > 1500 && outline > 1500,
            "fill {fill}, outline {outline}"
        );
        let pixels = text_rgba(&layer("AB CD", Some(([3, 5], 0xff2020))), size).unwrap();
        assert!(
            count(&pixels, &red, Some(true)) > 500,
            "active word not red"
        );
        assert_eq!(
            count(&pixels, &red, Some(false)),
            0,
            "inactive word colored"
        );
        // Two lines in a box one line tall: shrunk, both drawn (nothing in the bottom-left
        // corner would mean the second line was dropped).
        let tall = TextLayer {
            bounds: [0.0, 0.0, 400.0, 120.0],
            ..layer("ONE LONG LINE THAT WRAPS TWICE", None)
        };
        let pixels = text_rgba(&tall, size).unwrap();
        let lower = pixels[(size[0] * size[1] * 2) as usize..]
            .chunks_exact(4)
            .filter(|p| p[3] > 250)
            .count();
        assert!(lower > 300, "second line dropped: {lower}");
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
                active: None,
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
            let mut job = composition
                .start_export(&file, &Encoding::new(Codec::H264))
                .unwrap();
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

    pub(super) fn exports_alpha_audio_ranges_and_chapters() {
        let dir = scratch("exports");
        let red = still(&dir.0, 1, [220, 30, 30]);
        let mut project = project(
            vec![red],
            vec![(1, 60, vec![fx(1, "transform", vec![("scale", n(0.5))])])],
        );
        project.markers = vec![
            Marker {
                id: MarkerId::from_serial(1),
                at: 10,
                name: "Intro".into(),
                color: None,
            },
            Marker {
                id: MarkerId::from_serial(2),
                at: 40,
                name: "Demo".into(),
                color: None,
            },
        ];
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let export = |name: &str, encoding: Encoding| {
            let file = dir.0.join(name);
            let mut job = composition.start_export(&file, &encoding).unwrap();
            while let ExportState::Running(_) = job
                .step(Duration::from_millis(16))
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {}
            assert_eq!(job.status(), ExportState::Complete, "{name}");
            file
        };
        // ProRes 4444 keeps transparency outside the half-size picture.
        let master = export("master.mov", Encoding::new(Codec::ProRes4444));
        let (pixels, [w, _]) = apple_sys::media::first_frame(&master).unwrap();
        assert_eq!(pixels[3], 0, "corner is transparent");
        let center = ((32 * w + 48) * 4) as usize;
        assert_eq!(pixels[center + 3], 255, "picture is opaque");
        // A range with chapters from markers.
        let mut ranged = Encoding::new(Codec::H264);
        ranged.range = [5, 50];
        ranged.chapters = vec![(10, "Intro".into()), (40, "Demo".into())];
        let cut = export("cut.mp4", ranged);
        let probe = apple_sys::media::probe(&cut).unwrap();
        assert!(
            (probe.duration_seconds - 45.0 / 30.0).abs() < 0.05,
            "{}",
            probe.duration_seconds
        );
        let chapters = apple_sys::media::chapters(&cut).unwrap();
        let names: Vec<_> = chapters.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(names, ["Intro", "Demo"]);
        assert!((chapters[1].0 - 35.0 / 30.0).abs() < 0.05, "{chapters:?}");
    }

    /// AC-ED-2: what the agent sees is what export makes. Flat patches read back with
    /// `read_frame` (view.frames) and decoded from every export format with the system
    /// decoders agree within each codec's bound, and the movies are tagged with the sRGB
    /// working space (BT.709 primaries and matrix, sRGB transfer) so players show the same.
    pub(super) fn exports_decode_to_the_rendered_colors() {
        const PATCHES: [[u8; 3]; 8] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [220, 60, 30],
            [128, 128, 128],
            [16, 16, 16],
            [40, 180, 80],
            [255, 255, 255],
        ];
        let dir = scratch("colors");
        let picture: Vec<u8> = (0..64)
            .flat_map(|y| {
                (0..96).flat_map(move |x| {
                    let [r, g, b] = PATCHES[y / 32 * 4 + x / 24];
                    [r, g, b, 255]
                })
            })
            .collect();
        let media = still(&dir.0, 1, [0, 0, 0]);
        let png = apple_sys::image::png(&picture, [96, 64], 0).unwrap();
        fs::write(dir.0.join(&media.path), png).unwrap();
        let composition =
            Composition::prepare(&project(vec![media], vec![(1, 2, vec![])]), &dir.0).unwrap();
        let mut rendered = vec![0; 96 * 64 * 4];
        composition.read_frame(0, &mut rendered).unwrap();
        // Largest channel difference over each patch's middle (away from chroma edges).
        let worst = |decoded: &[u8]| {
            let mut worst = (0, [0; 3], [0; 3]);
            for patch in 0..PATCHES.len() {
                let (x0, y0) = (patch % 4 * 24 + 8, patch / 4 * 32 + 12);
                for at in (y0..y0 + 8).flat_map(|y| (x0..x0 + 8).map(move |x| (y * 96 + x) * 4)) {
                    let a: [u8; 3] = rendered[at..at + 3].try_into().unwrap();
                    let b: [u8; 3] = decoded[at..at + 3].try_into().unwrap();
                    let delta = a.iter().zip(b).map(|(x, y)| x.abs_diff(y)).max().unwrap();
                    if delta > worst.0 {
                        worst = (delta, a, b);
                    }
                }
            }
            worst
        };
        // The render itself keeps the still's sRGB values.
        let (delta, a, b) = worst(&picture);
        assert!(delta <= 1, "render {a:?} vs still {b:?}");
        // (primaries, transfer, matrix) from the movie's colr atom: nclx in MP4, nclc in QuickTime.
        let colr = |path: &std::path::Path| {
            let bytes = fs::read(path).unwrap();
            let at = bytes.windows(4).position(|w| w == b"colr")?;
            let field =
                |i: usize| u16::from_be_bytes([bytes[at + 8 + i * 2], bytes[at + 9 + i * 2]]);
            Some((field(0), field(1), field(2)))
        };
        // Bounds per format, measured on the Mini (docs/RECORDER-PARITY.md AC-ED-2): ProRes,
        // GIF and PNG round-trip exactly (0); HEVC is off by 1; H.264 at the default rate is off
        // by 3 on the 220,60,30 patch (1 at 20 Mbps: its quantizer, not the color path). Before the
        // sRGB working space every movie and the PNG were 11-12 levels off (128 came back 139).
        let mut report = Vec::new();
        for (name, codec, bound) in [
            ("prores4444.mov", Codec::ProRes4444, 1),
            ("prores422.mov", Codec::ProRes422, 1),
            ("h264.mp4", Codec::H264, 3),
            ("hevc.mp4", Codec::Hevc, 2),
            ("hevc-alpha.mov", Codec::HevcAlpha, 2),
        ] {
            let file = dir.0.join(name);
            let mut job = composition
                .start_export(&file, &Encoding::new(codec))
                .unwrap();
            while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
            assert_eq!(job.status(), ExportState::Complete, "{name}");
            let (decoded, size) = apple_sys::media::first_frame(&file).unwrap();
            assert_eq!(size, [96, 64], "{name}");
            report.push((name, worst(&decoded), bound, colr(&file)));
        }
        // GIFs draw through Composition::frames (the export's sequential decode) into Splice's
        // GIF writer, which stores the values verbatim and ImageIO reads back exactly
        // (splice_mcp::gif::tests::imageio_decodes_the_written_colors_exactly).
        let mut frames = composition.frames([0, 1]).unwrap();
        let mut drawn = vec![0; rendered.len()];
        assert_eq!(frames.next(Some(&mut drawn)).unwrap(), Some(0));
        report.push(("gif", worst(&drawn), 1, None));
        // The poster is encoded from read_frame and decoded with ImageIO.
        let poster = dir.0.join("poster.png");
        fs::write(&poster, encode_png(&rendered, [96, 64], 0).unwrap()).unwrap();
        report.push((
            "poster",
            worst(&apple_sys::image::decode_rgba(&poster).unwrap().0),
            0,
            None,
        ));
        for (name, (delta, a, b), _, tags) in &report {
            eprintln!("{name}: max delta {delta} (rendered {a:?}, decoded {b:?}), colr {tags:?}");
        }
        for (name, (delta, a, b), bound, tags) in report {
            assert!(
                delta <= bound,
                "{name}: rendered {a:?} decoded {b:?}, over {bound}"
            );
            if name.contains('.') {
                assert_eq!(
                    tags,
                    Some((1, 13, 1)),
                    "{name}: BT.709 primaries, sRGB transfer, BT.709 matrix"
                );
            }
        }
    }

    pub(super) fn cutout_uses_the_person_matte() {
        let dir = scratch("cutout");
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media/probe.mov");
        fs::copy(&fixture, dir.0.join("probe.mov")).unwrap();
        let mut video = still(&dir.0, 1, [0, 0, 0]);
        video.kind = MediaKind::Video;
        video.path = "probe.mov".into();
        video.probe.frames = 30;
        // No person in the fixture: the matte is empty and the cutout removes everything.
        let matte = matte_path(&dir.0, video.id);
        fs::create_dir_all(matte.parent().unwrap()).unwrap();
        apple_sys::vision::matte(&dir.0.join("probe.mov"), &matte).unwrap();
        let plain = project(vec![video.clone()], vec![(1, 20, vec![])]);
        let before = pixel(&Composition::prepare(&plain, &dir.0).unwrap(), 5, [48, 32]);
        assert!(
            before.iter().map(|&c| u32::from(c)).sum::<u32>() > 50,
            "{before:?}"
        );
        let cut = project(
            vec![video],
            vec![(
                1,
                20,
                vec![fx(1, "cutout", vec![("background", s("remove"))])],
            )],
        );
        let after = pixel(&Composition::prepare(&cut, &dir.0).unwrap(), 5, [48, 32]);
        assert!(after.iter().all(|&c| c < 12), "{after:?}");
    }

    /// T24.3: a 4K frame with the 0.3 stack (window frame, two stills under a dissolve, keyed
    /// transform, color with a 17³ LUT, blur and pixelate masks, two media overlays, a styled
    /// title and an arrow) composes; decode, compose and readback are reported. Cursor, click zoom and
    /// captions need a recorded take; the 0.2 window test covers them.
    pub(super) fn full_stack_4k_frame_composes_under_16_ms() {
        let dir = scratch("budget-4k");
        let project = full_stack(&dir.0, [3840, 2160], 30, 120);
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let [w, h] = composition.size().map(|n| n as usize);
        assert_eq!([w, h], [3840, 2160]);
        let mut rgba = vec![0; w * h * 4];
        // Inside the dissolve and the overlays: every layer is live.
        let frames: Vec<Frame> = (95..125).step_by(3).collect();
        for &at in &frames[..5] {
            composition.read_frame(at, &mut rgba).unwrap();
        }
        // The viewer decodes ahead and draws to its drawable; a grab decodes from scratch and reads
        // 33 MB back. The budget is the compose step both share; the others are reported.
        let mut phases: [Vec<Duration>; 3] = Default::default();
        for &at in &frames {
            composition.read_frame_warm(at, &mut rgba, 3).unwrap();
            for (list, time) in phases.iter_mut().zip(composition.last_frame_timings()) {
                list.push(time);
            }
        }
        let p90 = |list: &mut Vec<Duration>| {
            list.sort_unstable();
            list[list.len() * 9 / 10]
        };
        let [decode, compose, readback] = phases.each_mut().map(p90);
        eprintln!(
            "4K full stack p90: decode {decode:?}, compose {compose:?}, readback {readback:?}"
        );
        // Warm compose (the GPU work the viewer repeats each frame) is the 16 ms budget.
        assert!(
            compose < Duration::from_millis(16),
            "compose p90 {compose:?}"
        );
    }

    /// T24.3: 8 s of 1080p60 with the same stack exports at least twice as fast as realtime
    /// (the plan's 10-minute run, scaled to the 5 s per-test deadline).
    pub(super) fn full_stack_1080p60_exports_twice_realtime() {
        let dir = scratch("budget-export");
        let project = full_stack(&dir.0, [1920, 1080], 60, 240);
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let file = dir.0.join("budget.mp4");
        let started = std::time::Instant::now();
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::Hevc))
            .unwrap();
        while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
        let took = started.elapsed();
        assert_eq!(job.status(), ExportState::Complete);
        eprintln!("1080p60 full stack export: 8 s in {took:?}");
        assert!(took < Duration::from_secs(4), "export took {took:?}");
    }

    /// Two 1080p stills under a dissolve with the 0.3 look stack, two overlays, a title and an
    /// arrow, at `size` and `fps`; each clip is `clip_length` frames.
    fn full_stack(dir: &std::path::Path, size: [u32; 2], fps: u32, clip_length: Frame) -> Project {
        let big = |serial: u64, rgb: [u8; 3]| {
            let [w, h] = [1920u32, 1080];
            let pixels: Vec<u8> = (0..w * h)
                .flat_map(|i| [rgb[0], rgb[1].wrapping_add((i % w) as u8), rgb[2], 255])
                .collect();
            let name = format!("big{serial}.png");
            fs::write(
                dir.join(&name),
                apple_sys::image::png(&pixels, [w, h], 0).unwrap(),
            )
            .unwrap();
            let mut media = still(dir, serial + 100, rgb);
            media.id = MediaId::from_serial(serial);
            media.path = name;
            media.probe.width = w;
            media.probe.height = h;
            media
        };
        let mut cube = String::from("LUT_3D_SIZE 17\n");
        for b in 0..17 {
            for g in 0..17 {
                for r in 0..17 {
                    let v = |c: i32| f64::from(c) / 16.0;
                    cube.push_str(&format!("{} {} {}\n", v(r), v(g), v(b)));
                }
            }
        }
        fs::write(dir.join("look.cube"), cube).unwrap();
        let keys = EffectValue::Keys(vec![
            keys::Key {
                at: 0,
                value: 1.0,
                ease: keys::Ease::InOut,
            },
            keys::Key {
                at: 90,
                value: 1.3,
                ease: keys::Ease::Linear,
            },
        ]);
        let mut project = project(
            vec![
                big(1, [30, 60, 200]),
                big(2, [200, 120, 40]),
                still(dir, 3, [250, 250, 250]),
            ],
            vec![
                (1, clip_length, vec![]),
                (
                    2,
                    clip_length,
                    vec![
                        fx(
                            10,
                            "transition",
                            vec![("kind", s("dissolve")), ("length", n(30.0))],
                        ),
                        fx(11, "transform", vec![("scale", keys)]),
                        fx(
                            12,
                            "color",
                            vec![("saturation", n(1.2)), ("lut", s("look.cube"))],
                        ),
                        fx(
                            13,
                            "mask",
                            vec![
                                ("mode", s("blur")),
                                ("x", n(0.6)),
                                ("y", n(0.05)),
                                ("w", n(0.3)),
                                ("h", n(0.1)),
                            ],
                        ),
                        fx(
                            14,
                            "mask",
                            vec![
                                ("mode", s("pixelate")),
                                ("x", n(0.1)),
                                ("y", n(0.7)),
                                ("w", n(0.3)),
                                ("h", n(0.2)),
                            ],
                        ),
                    ],
                ),
            ],
        );
        project.size = size;
        project.fps = fps;

        project.effects.push(fx(
            20,
            "window",
            vec![
                ("padding", n(0.05)),
                ("corner", n(0.03)),
                ("shadow", n(0.6)),
            ],
        ));
        let lanes = [
            (31, LaneKind::Overlay),
            (32, LaneKind::Overlay),
            (33, LaneKind::Graphics),
            (34, LaneKind::Graphics),
        ];
        for (serial, kind) in lanes {
            project.lanes.push(Lane {
                id: LaneId::from_serial(serial),
                name: String::new(),
                kind,
                effects: Vec::new(),
            });
        }
        let item = |serial: u64, lane: u64, kind: ItemKind, effects: Vec<Effect>| Item {
            id: ItemId::from_serial(serial),
            timing: ItemTiming {
                anchor: Anchor::Pinned(clip_length / 2),
                length: clip_length,
            },
            data: ItemData {
                lane: LaneId::from_serial(lane),
                kind,
                appearance: Appearance {
                    effects,
                    ..Appearance::default()
                },
            },
        };
        let overlay = |placement| {
            ItemKind::Media(Overlay {
                media: MediaId::from_serial(3),
                source: 0,
                placement,
                behind_presenter: false,
                muted: true,
                framing: Framing {
                    crop: None,
                    fit: Fit::Fill,
                },
            })
        };
        for built in [
            item(41, 31, overlay(Placement::TopRight), vec![]),
            item(
                42,
                32,
                overlay(Placement::Card),
                vec![fx(43, "transform", vec![("rotation", n(-4.0))])],
            ),
            item(
                44,
                33,
                ItemKind::Title {
                    text: "Field notes, part two".into(),
                },
                vec![fx(45, "animate", vec![("preset", s("rise"))])],
            ),
            item(
                46,
                34,
                ItemKind::Shape {
                    shape: ShapeKind::Arrow,
                    rect: Rect {
                        x: 0.4,
                        y: 0.4,
                        w: 0.2,
                        h: 0.1,
                    },
                },
                vec![fx(
                    47,
                    "shape",
                    vec![("stroke", s("#ff3b30")), ("strokeWidth", n(0.01))],
                )],
            ),
        ] {
            project.items.insert(built).unwrap();
        }

        project
    }
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[
        (
            "gpu_full_stack_4k_frame_composes_under_16_ms",
            looks::full_stack_4k_frame_composes_under_16_ms,
        ),
        (
            "gpu_full_stack_1080p60_exports_twice_realtime",
            looks::full_stack_1080p60_exports_twice_realtime,
        ),
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
            "looks_cleanup_chain_lowers_the_noise_floor_and_keeps_speech",
            looks::cleanup_chain_lowers_the_noise_floor_and_keeps_speech,
        ),
        ("looks_image_overlay_renders", looks::image_overlay_renders),
        (
            "looks_shot_analysis_finds_a_hard_cut",
            looks::shot_analysis_finds_a_hard_cut,
        ),
        (
            "looks_portrait_vfr_and_p3_footage_render_correctly",
            looks::portrait_vfr_and_p3_footage_render_correctly,
        ),
        (
            "looks_ocr_reads_text_and_a_blur_mask_hides_it",
            looks::ocr_reads_text_and_a_blur_mask_hides_it,
        ),
        (
            "looks_t45_html_pages_render_deterministically",
            looks::t45_html_pages_render_deterministically,
        ),
        (
            "looks_m23_animated_gifs_play_and_loop",
            looks::m23_animated_gifs_play_and_loop,
        ),
        (
            "looks_lottie_overlays_play_loop_and_stay_transparent",
            looks::lottie_overlays_play_loop_and_stay_transparent,
        ),
        (
            "looks_m22_per_character_titles_draw_every_character",
            looks::m22_per_character_titles_draw_every_character,
        ),
        (
            "looks_h7_the_backing_never_borrows_the_picture_look",
            looks::h7_the_backing_never_borrows_the_picture_look,
        ),
        (
            "looks_h8_caption_stroke_sits_under_the_fill_and_the_active_word_is_colored",
            looks::h8_caption_stroke_sits_under_the_fill_and_the_active_word_is_colored,
        ),
        (
            "looks_exports_alpha_audio_ranges_and_chapters",
            looks::exports_alpha_audio_ranges_and_chapters,
        ),
        (
            "looks_exports_decode_to_the_rendered_colors",
            looks::exports_decode_to_the_rendered_colors,
        ),
        (
            "looks_cutout_uses_the_person_matte",
            looks::cutout_uses_the_person_matte,
        ),
        (
            "looks_window_inset_draws_an_inner_border",
            looks::window_inset_draws_an_inner_border,
        ),
        (
            "looks_t46_backdrops_and_the_camera_bubble_render",
            looks::t46_backdrops_and_the_camera_bubble_render,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
