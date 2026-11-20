//! T46.2 on the GPU path: zooms that follow the cursor, motion blur on the camera and the cursor,
//! the always-arrow and looping cursor, and the click sound in exported audio. Every check renders
//! a recorded take fixture and reads pixels (or samples) back from what export makes.
#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
mod motion {
    use splice_av::*;
    use splice_model::*;
    use std::{fs, path::Path, path::PathBuf, time::Duration};

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("splice-motion-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
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

    /// Pointer samples at 120 Hz over 1 s: `at(t)` gives the position, `down` the press times.
    fn events(at: &dyn Fn(f64) -> [f64; 2], down: &[f64]) -> String {
        let mut list = vec![r#"{"t":0.0,"k":"cursor","id":"c"}"#.to_string()];
        for i in 0..=120 {
            let t = f64::from(i) / 120.0;
            let [x, y] = at(t);
            let kind = if down.iter().any(|d| (d - t).abs() < 1e-9) {
                r#""down","b":0"#
            } else {
                r#""move""#
            };
            list.push(format!(r#"{{"t":{t},"k":{kind},"x":{x},"y":{y}}}"#));
        }
        format!(
            r#"{{"t0":0,"source":{{"kind":"display","rect":{{"x":0,"y":0,"width":96,"height":64}},"scale":1}},"events":[{}]}}"#,
            list.join(",")
        )
    }

    /// A 1 s, 96×64 screen take (the probe movie) with an 8×8 red cursor whose hotspot is its
    /// top-left pixel, on one 30-frame clip, with `cursor` effect params and optional zooms.
    fn take(dir: &Path, events: &str, cursor: Vec<(&str, EffectValue)>, zooms: &[Zoom]) -> Project {
        let take = dir.join("takes/med_1");
        fs::create_dir_all(take.join("cursors")).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media/probe.mov");
        fs::copy(fixture, take.join("screen.mov")).unwrap();
        let red: Vec<u8> = [255, 0, 0, 255].repeat(64);
        fs::write(
            take.join("cursors/c.png"),
            encode_png(&red, [8, 8], 0).unwrap(),
        )
        .unwrap();
        fs::write(
            take.join("cursors/c.json"),
            r#"{"hotX":0,"hotY":0,"scale":1}"#,
        )
        .unwrap();
        fs::write(take.join("events.json"), events).unwrap();
        let media = Media {
            id: MediaId::from_serial(1),
            kind: MediaKind::Take,
            path: "takes/med_1".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 30,
                width: 96,
                height: 64,
                audio: Vec::new(),
                codec: Some("h264".into()),
                loudness: None,
            },
            take: Some(TakeComponents {
                screen: Some("screen.mov".into()),
                events: Some("events.json".into()),
                cursors: Some("cursors".into()),
                ..TakeComponents::default()
            }),
        };
        let mut params = vec![
            ("size", EffectValue::Number(1.0)),
            ("clickRings", EffectValue::Bool(false)),
            ("hideWhenIdle", EffectValue::Bool(false)),
            ("clickPulse", EffectValue::Bool(false)),
        ];
        params.extend(cursor);
        let mut project = Project {
            size: [96, 64],
            media: vec![media],
            effects: vec![fx(1, "cursor", params)],
            ..Project::default()
        };
        project.lanes.push(Lane {
            id: LaneId::from_serial(5),
            name: "Camera".into(),
            kind: LaneKind::Camera,
            effects: Vec::new(),
        });
        for (i, zoom) in zooms.iter().enumerate() {
            project
                .items
                .insert(Item {
                    id: ItemId::from_serial(10 + i as u64),
                    timing: ItemTiming {
                        anchor: Anchor::Pinned(0),
                        length: 30,
                    },
                    data: ItemData {
                        lane: LaneId::from_serial(5),
                        kind: ItemKind::Zoom(*zoom),
                        appearance: Appearance::default(),
                    },
                })
                .unwrap();
        }
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
                    media: Some(MediaId::from_serial(1)),
                    enabled: ComponentStates {
                        screen: Some(true),
                        pointer: Some(true),
                        ..ComponentStates::default()
                    },
                    ..ClipData::default()
                },
            })
            .unwrap();
        project
    }

    fn zoom(amount: f64, focus: Option<[f64; 2]>, follow: bool, blur: f64) -> Zoom {
        Zoom {
            amount,
            focus,
            mode: CameraMode::Content,
            spring: Spring::Instant,
            follow,
            motion_blur: blur,
        }
    }

    fn frame(composition: &Composition, at: Frame) -> Vec<u8> {
        let [w, h] = composition.size().map(|n| n as usize);
        let mut rgba = vec![0; w * h * 4];
        composition.read_frame(at, &mut rgba).unwrap();
        rgba
    }

    /// Pure-red pixel count, reddish (partly covered) count and the pure-red centroid.
    fn red(rgba: &[u8]) -> (usize, usize, [f64; 2]) {
        let (mut pure, mut reddish, mut sum) = (0, 0, [0.0, 0.0]);
        for (i, p) in rgba.chunks_exact(4).enumerate() {
            if p[0] > 200 && p[1] < 60 && p[2] < 60 {
                pure += 1;
                sum[0] += (i % 96) as f64;
                sum[1] += (i / 96) as f64;
            }
            if i32::from(p[0]) - i32::from(p[1]).max(i32::from(p[2])) > 40 {
                reddish += 1;
            }
        }
        let n = pure.max(1) as f64;
        (pure, reddish, [sum[0] / n, sum[1] / n])
    }

    /// Rests top-left, moves by hand to the bottom-right between 0.3 and 0.4 s, rests.
    fn across(t: f64) -> [f64; 2] {
        let k = ((t - 0.3) / 0.1).clamp(0.0, 1.0);
        [0.15 + 0.7 * k, 0.2 + 0.6 * k]
    }

    /// A 2.5× zoom with `focus:"cursor"` keeps the cursor in view after it crosses the screen; the
    /// same zoom on the center loses it.
    pub(super) fn following_zoom_keeps_the_cursor_in_view() {
        let dir = scratch("follow");
        let events = events(&across, &[]);
        let follow = take(&dir.0, &events, vec![], &[zoom(2.5, None, true, 0.0)]);
        let center = take(
            &dir.0,
            &events,
            vec![],
            &[zoom(2.5, Some([0.5, 0.5]), false, 0.0)],
        );
        let follow = Composition::prepare(&follow, &dir.0).unwrap();
        let center = Composition::prepare(&center, &dir.0).unwrap();
        // Before the move both frame the top-left cursor differently: following shows it.
        assert!(
            red(&frame(&follow, 3)).0 >= 30,
            "{:?}",
            red(&frame(&follow, 3))
        );
        // 0.83 s: the cursor rests at 0.85, 0.8 of the source.
        let (seen, _, at) = red(&frame(&follow, 25));
        assert!(seen >= 30, "following: {seen} red pixels");
        // Inside the central 60 % (plus the 8 px sprite) of the view.
        assert!(
            (10.0..=86.0).contains(&at[0]) && (6.0..=60.0).contains(&at[1]),
            "{at:?}"
        );
        assert_eq!(
            red(&frame(&center, 25)).0,
            0,
            "a center zoom crops the cursor away"
        );
    }

    /// Luminance edge energy: lower is blurrier.
    fn sharpness(rgba: &[u8]) -> u64 {
        let luma = |i: usize| u64::from(rgba[i * 4]) + u64::from(rgba[i * 4 + 1]);
        (0..64)
            .flat_map(|y| (0..95).map(move |x| y * 96 + x))
            .map(|i| luma(i).abs_diff(luma(i + 1)))
            .sum()
    }

    /// Camera `motionBlur` smears the picture while the camera moves and changes nothing while
    /// it holds still; the cursor's own `motionBlur` smears the sprite along its move.
    pub(super) fn motion_blur_smears_moving_camera_and_cursor() {
        let dir = scratch("blur");
        // A checkerboard still makes camera blur measurable.
        let board: Vec<u8> = (0..96 * 64)
            .flat_map(|i| {
                let v = if ((i % 96) / 6 + (i / 96) / 6) % 2 == 0 {
                    230
                } else {
                    20
                };
                [v, v, v, 255]
            })
            .collect();
        fs::write(
            dir.0.join("board.png"),
            encode_png(&board, [96, 64], 96).unwrap(),
        )
        .unwrap();
        // At 3x the camera holds on the left until frame 12, then pans right with the snappy
        // spring (tens of pixels a frame).
        let still = |blur: f64| {
            let mut project = take(&dir.0, &events(&|_| [0.5, 0.5], &[]), vec![], &[]);
            project.media[0] = Media {
                kind: MediaKind::Image,
                path: "board.png".into(),
                take: None,
                ..project.media[0].clone()
            };
            project.media[0].probe.codec = None;
            for (serial, start, length, x) in [(20, 0, 12, 0.2), (21, 12, 18, 0.8)] {
                project
                    .items
                    .insert(Item {
                        id: ItemId::from_serial(serial),
                        timing: ItemTiming {
                            anchor: Anchor::Pinned(start),
                            length,
                        },
                        data: ItemData {
                            lane: LaneId::from_serial(5),
                            kind: ItemKind::Zoom(Zoom {
                                spring: Spring::Snappy,
                                ..zoom(3.0, Some([x, 0.5]), false, blur)
                            }),
                            appearance: Appearance::default(),
                        },
                    })
                    .unwrap();
            }
            Composition::prepare(&project, &dir.0).unwrap()
        };
        let (sharp, blurred) = (still(0.0), still(1.0));
        // Held still on the left: identical (moves under half a pixel draw no taps).
        assert_eq!(frame(&sharp, 10), frame(&blurred, 10));
        // During the pan vertical edges smear: a frame loses over a quarter of its edge energy.
        let ratios: Vec<f64> = [13, 15, 17, 19]
            .into_iter()
            .map(|at| {
                let (a, b) = (
                    sharpness(&frame(&sharp, at)),
                    sharpness(&frame(&blurred, at)),
                );
                b as f64 / a as f64
            })
            .collect();
        assert!(ratios.iter().any(|r| *r < 0.75), "{ratios:?}");
        // Settled on the right again: identical.
        assert_eq!(frame(&sharp, 29), frame(&blurred, 29));
        // The cursor: mid-move it spreads over more pixels and fewer stay pure red.
        let events = events(&across, &[]);
        let plain = take(&dir.0, &events, vec![], &[]);
        let smeared = take(
            &dir.0,
            &events,
            vec![("motionBlur", EffectValue::Number(1.0))],
            &[],
        );
        let plain = red(&frame(&Composition::prepare(&plain, &dir.0).unwrap(), 11));
        let smeared = red(&frame(&Composition::prepare(&smeared, &dir.0).unwrap(), 11));
        assert!(
            smeared.0 < plain.0 && smeared.1 > plain.1,
            "{plain:?} -> {smeared:?}"
        );
        // At rest nothing changes.
        let rest = |params| {
            red(&frame(
                &Composition::prepare(&take(&dir.0, &events, params, &[]), &dir.0).unwrap(),
                2,
            ))
        };
        assert_eq!(
            rest(vec![]),
            rest(vec![("motionBlur", EffectValue::Number(1.0))])
        );
    }

    /// `alwaysArrow` draws the system arrow instead of the recorded (red) image; `loop` brings the
    /// cursor back to where the clip starts on its last frame.
    pub(super) fn cursor_always_arrow_and_loop() {
        let dir = scratch("arrow");
        let events = events(&across, &[]);
        let render =
            |params| Composition::prepare(&take(&dir.0, &events, params, &[]), &dir.0).unwrap();
        let plain = render(vec![]);
        let recorded = frame(&plain, 3);
        let arrow = frame(&render(vec![("alwaysArrow", EffectValue::Bool(true))]), 3);
        let hidden = frame(&render(vec![("hidden", EffectValue::Bool(true))]), 3);
        assert!(red(&recorded).0 >= 30);
        assert_eq!(red(&arrow).0, 0, "no recorded image with alwaysArrow");
        // The arrow sits at the hotspot (0.15, 0.2 → 14, 13 px): pixels there differ from the
        // picture without a cursor.
        let changed = (13..30)
            .flat_map(|y| (14..26).map(move |x| (y * 96 + x) * 4))
            .filter(|&i| (0..3).any(|c| arrow[i + c].abs_diff(hidden[i + c]) > 40))
            .count();
        assert!(changed >= 10, "arrow pixels: {changed}");
        let first = red(&frame(&plain, 0)).2;
        let unlooped = red(&frame(&plain, 29)).2;
        let looped = red(&frame(
            &render(vec![("loop", EffectValue::Number(12.0))]),
            29,
        ))
        .2;
        assert!(
            (unlooped[0] - first[0]).abs() > 30.0,
            "{first:?} {unlooped:?}"
        );
        assert!(
            (looped[0] - first[0]).abs() <= 1.5 && (looped[1] - first[1]).abs() <= 1.5,
            "{first:?} {looped:?}"
        );
    }

    /// `clickSound` mixes a click into the exported audio at each recorded press, and nothing
    /// between them.
    pub(super) fn click_sound_is_mixed_at_each_press() {
        let dir = scratch("click");
        let events = events(&|_| [0.5, 0.5], &[0.2, 0.7]);
        let project = take(
            &dir.0,
            &events,
            vec![("clickSound", EffectValue::Bool(true))],
            &[],
        );
        let composition = Composition::prepare(&project, &dir.0).unwrap();
        let file = dir.0.join("clicks.wav");
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::Wav))
            .unwrap();
        while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
        assert_eq!(job.status(), ExportState::Complete);
        let envelope = apple_sys::audio::envelope(&file, 0, 100.0).unwrap();
        let loud = |from: usize, to: usize| {
            envelope[from..to.min(envelope.len())]
                .iter()
                .copied()
                .fold(0.0f32, f32::max)
        };
        assert!(loud(19, 27) > 0.05, "first click {envelope:?}");
        assert!(loud(69, 77) > 0.05, "second click {envelope:?}");
        assert!(loud(35, 60) < 0.005, "quiet between clicks {envelope:?}");
    }
}

#[cfg(target_os = "macos")]
fn main() {
    harness::run(&[
        (
            "motion_following_zoom_keeps_the_cursor_in_view",
            motion::following_zoom_keeps_the_cursor_in_view,
        ),
        (
            "motion_blur_smears_moving_camera_and_cursor",
            motion::motion_blur_smears_moving_camera_and_cursor,
        ),
        (
            "motion_cursor_always_arrow_and_loop",
            motion::cursor_always_arrow_and_loop,
        ),
        (
            "motion_click_sound_is_mixed_at_each_press",
            motion::click_sound_is_mixed_at_each_press,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
