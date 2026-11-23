#[cfg(target_os = "macos")]
#[rustfmt::skip]
#[path = "../../../pxui/tools/t/window_harness.rs"]
mod harness;

#[cfg(target_os = "macos")]
fn release_keepalive() {}

#[cfg(target_os = "macos")]
mod native {
    use splice_av::*;
    use splice_model::*;
    use std::path::Path;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    struct Output(std::path::PathBuf);

    impl Drop for Output {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media")
    }

    fn video() -> Media {
        Media {
            id: MediaId::from_serial(1),
            kind: MediaKind::Video,
            path: "probe.mov".into(),
            bookmark: None,
            size: 3014,
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
            take: None,
        }
    }

    fn pixel(composition: &Composition, frame: Frame, [x, y]: [usize; 2]) -> [u8; 3] {
        let [w, h] = composition.size().map(|n| n as usize);
        let mut rgba = vec![0; w * h * 4];
        composition.read_frame(frame, &mut rgba).unwrap();
        let at = (y * w + x) * 4;
        rgba[at..at + 3].try_into().unwrap()
    }

    fn brightness(color: [u8; 3]) -> u32 {
        color.into_iter().map(u32::from).sum()
    }

    pub(super) fn native_composition_uses_source_time_and_composites_overlay_fades_over_a_gap() {
        let lane = LaneId::from_serial(1);
        let mut project = Project {
            size: [96, 64],
            media: vec![video()],
            lanes: vec![Lane {
                id: lane,
                name: "Overlay".into(),
                kind: LaneKind::Overlay,
                effects: Vec::new(),
            }],
            ..Project::default()
        };
        let media = project.media[0].id;
        for (serial, start, length, source, speed) in [(1, 0, 10, 4, 2.0), (2, 20, 30, 0, 1.0)] {
            project
                .clips
                .insert(Clip {
                    id: ClipId::from_serial(serial),
                    timing: ClipTiming {
                        start,
                        length,
                        source,
                        speed,
                    },
                    data: ClipData {
                        media: Some(media),
                        ..ClipData::default()
                    },
                })
                .unwrap();
        }
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(11),
                    length: 6,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Media(Overlay {
                        media,
                        source: 22,
                        placement: Placement::TopLeft,
                        behind_presenter: false,
                        muted: true,
                        framing: Framing::default(),
                    }),
                    appearance: Appearance {
                        fade_in: 4,
                        fade_out: 4,
                        ..Appearance::default()
                    },
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &fixture()).unwrap();
        assert_eq!(composition.end(), 50);
        let fast = pixel(&composition, 5, [48, 32]);
        let normal = pixel(&composition, 34, [48, 32]);
        assert!(brightness(normal) > 50, "{normal:?}");
        assert!(
            fast.iter().zip(normal).all(|(a, b)| a.abs_diff(b) <= 12),
            "{fast:?} {normal:?}"
        );
        let fading = brightness(pixel(&composition, 11, [19, 14]));
        let visible = brightness(pixel(&composition, 13, [19, 14]));
        assert!(fading > 8 && visible > fading * 2, "{fading} {visible}");
        assert!(brightness(pixel(&composition, 13, [48, 32])) <= 12);
        assert!(brightness(pixel(&composition, 18, [19, 14])) <= 12);
    }

    pub(super) fn native_composition_covers_a_title_and_audio_without_spine_video() {
        let size = [320, 192];
        let lane = LaneId::from_serial(1);
        let mut audio = video();
        audio.kind = MediaKind::Audio;
        audio.path = "stereo.wav".into();
        audio.probe = Probe {
            fps: 30.0,
            frames: 3,
            width: 0,
            height: 0,
            audio: vec![AudioTrack {
                name: "stereo".into(),
                channels: Some(2),
            }],
            codec: Some("pcm".into()),
            loudness: None,
        };
        let media = audio.id;
        let mut project = Project {
            size,
            media: vec![audio],
            lanes: vec![Lane {
                id: lane,
                name: "Titles".into(),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            }],
            ..Project::default()
        };
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 3,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media),
                    appearance: Appearance {
                        fade_in: 2,
                        ..Appearance::default()
                    },
                    ..ClipData::default()
                },
            })
            .unwrap();
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(2),
                    length: 7,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Title {
                        text: "Splice".into(),
                    },
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &fixture()).unwrap();
        assert_eq!(composition.end(), 9);
        assert!(brightness(pixel(&composition, 8, [48, 32])) <= 12);
        let mut frame = composition.evaluator().buffer();
        composition.evaluator().frame(0, &mut frame);
        assert_eq!(frame.audio[0].gain, 0.5);
        composition.evaluator().frame(8, &mut frame);
        assert!(
            frame
                .layers
                .iter()
                .any(|layer| matches!(layer, splice_eval::Layer::Title { .. }))
        );

        let scratch =
            Output(std::env::temp_dir().join(format!("splice-av-test-{}", std::process::id())));
        fs::create_dir(&scratch.0).unwrap();
        let file = scratch.0.join("title.mp4");
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::H264))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match job.step(Duration::from_millis(16)).unwrap() {
                ExportState::Running(_) if Instant::now() < deadline => {}
                ExportState::Complete => break,
                status => panic!("Native title export ended with {status:?}"),
            }
        }
        let exported = apple_sys::media::probe(&file).unwrap();
        assert!((exported.duration_seconds - 0.3).abs() < 1.0 / 30.0);
        assert_eq!(exported.audio[0].channels, Some(2));
        let video_track = exported.video.unwrap();
        assert_eq!([video_track.width, video_track.height], size);
        let mut exported_media = video();
        exported_media.path = "title.mp4".into();
        exported_media.probe.frames = 9;
        [exported_media.probe.width, exported_media.probe.height] = size;
        let mut delivered = Project {
            size,
            media: vec![exported_media],
            ..Project::default()
        };
        delivered
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 9,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(MediaId::from_serial(1)),
                    ..ClipData::default()
                },
            })
            .unwrap();
        let delivered = Composition::prepare(&delivered, &scratch.0).unwrap();
        let mut pixels = vec![0; size[0] as usize * size[1] as usize * 4];
        delivered.read_frame(0, &mut pixels).unwrap();
        let first = pixels.chunks_exact(4).map(|pixel| pixel[0]).max().unwrap();
        let lit = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[0] >= 40)
            .count();
        assert!(
            first < 40,
            "frame 0 brightest {first}, {lit} lit pixels of {}",
            pixels.len() / 4
        );
        delivered.read_frame(5, &mut pixels).unwrap();
        let brightest = pixels.chunks_exact(4).map(|pixel| pixel[0]).max().unwrap();
        assert!(brightest > 120, "brightest title pixel was {brightest}");
    }

    /// An animated title still draws its backing box: units skip the box, so lowering adds one
    /// box-only raster under them (the box once vanished for every animator but highlight).
    pub(super) fn animated_title_keeps_its_box() {
        let lane = LaneId::from_serial(1);
        let param = |name: &str, value: &str| EffectParam {
            name: name.into(),
            value: EffectValue::Text(value.into()),
        };
        let effect = |serial, kind: &str, params| Effect {
            id: EffectId::from_serial(serial),
            kind: kind.into(),
            enabled: None,
            params,
        };
        let mut project = Project {
            size: [320, 192],
            lanes: vec![Lane {
                id: lane,
                name: "Titles".into(),
                kind: LaneKind::Graphics,
                effects: Vec::new(),
            }],
            ..Project::default()
        };
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 30,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Title {
                        text: "Boxed words".into(),
                    },
                    appearance: Appearance {
                        effects: vec![
                            effect(
                                1,
                                "text",
                                vec![param("box", "solid"), param("boxColor", "#ff0000")],
                            ),
                            effect(2, "animate", vec![param("preset", "pop")]),
                        ],
                        ..Appearance::default()
                    },
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &fixture()).unwrap();
        let [w, h] = composition.size().map(|n| n as usize);
        let mut rgba = vec![0; w * h * 4];
        composition.read_frame(25, &mut rgba).unwrap();
        let red = rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60)
            .count();
        assert!(red > 200, "animated title box drew {red} red pixels");
    }

    /// T12.2–T12.3 on a take with recorded events: the cursor draws where the events put it, scaled
    /// by the Cursor effect, and a camera segment on the click zooms the picture in toward it.
    pub(super) fn native_take_draws_the_recorded_cursor_and_zooms_on_click() {
        let scratch =
            Output(std::env::temp_dir().join(format!("splice-av-take-{}", std::process::id())));
        let take = scratch.0.join("takes/med_1");
        fs::create_dir_all(take.join("cursors")).unwrap();
        fs::copy(fixture().join("probe.mov"), take.join("screen.mov")).unwrap();
        // An 8×8 red cursor whose hotspot is its top-left pixel.
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
        // The source is 96 points wide, one pixel per point. Click on the right at 0.6 s, far enough
        // from the first move that the cursor does not glide between them.
        fs::write(
            take.join("events.json"),
            r#"{"t0":0,"source":{"kind":"display","rect":{"x":0,"y":0,"width":96,"height":64},"scale":1},"events":[
                {"t":0.0,"k":"cursor","id":"c"},{"t":0.0,"k":"move","x":0.25,"y":0.25},
                {"t":0.6,"k":"down","x":0.75,"y":0.5,"b":0},{"t":0.65,"k":"up","x":0.75,"y":0.5,"b":0}]}"#,
        )
        .unwrap();
        let mut media = video();
        media.kind = MediaKind::Take;
        media.path = "takes/med_1".into();
        media.take = Some(TakeComponents {
            screen: Some("screen.mov".into()),
            events: Some("events.json".into()),
            cursors: Some("cursors".into()),
            ..TakeComponents::default()
        });
        let effect = |id, kind: &str, params: Vec<(&str, EffectValue)>| Effect {
            id: EffectId::from_serial(id),
            kind: kind.into(),
            enabled: None,
            params: params
                .into_iter()
                .map(|(name, value)| EffectParam {
                    name: name.into(),
                    value,
                })
                .collect(),
        };
        let mut project = Project {
            size: [96, 64],
            media: vec![media],
            effects: vec![effect(
                1,
                "cursor",
                vec![
                    ("size", EffectValue::Number(1.0)),
                    ("clickRings", EffectValue::Bool(false)),
                ],
            )],
            ..Project::default()
        };
        // A camera segment on the click (what intent.autoZoom writes): 2× from frame 12, instant
        // so frame 19 is exactly zoomed.
        project.lanes.push(Lane {
            id: LaneId::from_serial(5),
            name: "Camera".into(),
            kind: LaneKind::Camera,
            effects: Vec::new(),
        });
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(6),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(12),
                    length: 18,
                },
                data: ItemData {
                    lane: LaneId::from_serial(5),
                    kind: ItemKind::Zoom(Zoom {
                        amount: 2.0,
                        focus: Some([0.75, 0.5]),
                        mode: CameraMode::Auto,
                        spring: Spring::Instant,
                        follow: false,
                        motion_blur: 0.0,
                    }),
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
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
        let composition = Composition::prepare(&project, &scratch.0).unwrap();
        let red_at = |frame| {
            let [w, h] = composition.size().map(|n| n as usize);
            let mut rgba = vec![0; w * h * 4];
            composition.read_frame(frame, &mut rgba).unwrap();
            let (mut sum, mut n) = ([0usize; 2], 0);
            for y in 0..h {
                for x in 0..w {
                    let p = &rgba[(y * w + x) * 4..][..3];
                    if p[0] > 200 && p[1] < 60 && p[2] < 60 {
                        sum[0] += x;
                        sum[1] += y;
                        n += 1;
                    }
                }
            }
            (
                n,
                if n > 0 {
                    [sum[0] / n, sum[1] / n]
                } else {
                    [0, 0]
                },
            )
        };
        // Frame 2 (0.067 s): no zoom yet; the cursor's top-left sits at a quarter of the picture.
        let (before, at) = red_at(2);
        assert!(before >= 30, "cursor pixels before the click: {before}");
        assert!(
            (20..=34).contains(&at[0]) && (12..=24).contains(&at[1]),
            "cursor center {at:?}"
        );
        // Frame 19 (0.63 s, just after the click): the camera frames the click at 2× (the
        // viewport is the source's right half), so the cursor sits mid-canvas at about four times
        // the area.
        let (zoomed, at) = red_at(19);
        assert!(
            zoomed >= before * 3,
            "cursor pixels at the zoomed click: {zoomed} vs {before}"
        );
        assert!(
            (48..=66).contains(&at[0]),
            "cursor center after zoom {at:?}"
        );
        // Found on a real take: with the capture cropped to its right half, the cursor at a
        // quarter of the width is in the cropped-away part and draws nowhere (not on the
        // backdrop beside the card).
        {
            let mut cropped = project.clone();
            let slot = cropped.clips.order()[0];
            cropped.clips.data_mut(slot).unwrap().framing.crop = Some(Rect {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            });
            let composition = Composition::prepare(&cropped, &scratch.0).unwrap();
            let [w, h] = composition.size().map(|n| n as usize);
            let mut rgba = vec![0; w * h * 4];
            composition.read_frame(2, &mut rgba).unwrap();
            let red = rgba
                .chunks(4)
                .filter(|p| p[0] > 200 && p[1] < 60 && p[2] < 60)
                .count();
            assert_eq!(red, 0, "cursor drawn outside the visible crop");
        }
        // D19, A1: a typed key shows as a chip for people; renders an agent sees draw the same
        // chip at the same place with its text redacted.
        fs::write(
            take.join("events.json"),
            r#"{"t0":0,"source":{"kind":"display","rect":{"x":0,"y":0,"width":96,"height":64},"scale":1},"events":[
                {"t":0.0,"k":"cursor","id":"c"},{"t":0.0,"k":"move","x":0.25,"y":0.25},
                {"t":0.1,"k":"key","ch":"W","keyCode":13}]}"#,
        )
        .unwrap();
        let slot = project.clips.order()[0];
        let data = project.clips.data_mut(slot).unwrap();
        data.enabled.keys = Some(true);
        data.appearance
            .effects
            .push(effect(2, "keystrokes", Vec::new()));
        let frame = |composition: &Composition| {
            let [w, h] = composition.size().map(|n| n as usize);
            let mut rgba = vec![0; w * h * 4];
            composition.read_frame(5, &mut rgba).unwrap();
            rgba
        };
        let human = frame(&Composition::prepare(&project, &scratch.0).unwrap());
        let agent = frame(&Composition::prepare_for_agent(&project, &scratch.0).unwrap());
        let width = project.size[0] as usize;
        // The bounding box of the pixels that differ between two renders.
        let changed = |a: &[u8], b: &[u8]| {
            let mut bounds: Option<[usize; 4]> = None;
            for (index, _) in a
                .chunks(4)
                .zip(b.chunks(4))
                .enumerate()
                .filter(|(_, (p, q))| p != q)
            {
                let (x, y) = (index % width, index / width);
                let b = bounds.get_or_insert([x, y, x, y]);
                *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
            }
            bounds
        };
        assert!(
            changed(&human, &agent).is_some(),
            "the agent chip is redacted"
        );
        // Shortcuts only: the typed key draws no chip for anyone.
        let slot = project.clips.order()[0];
        let data = project.clips.data_mut(slot).unwrap();
        data.appearance.effects.retain(|e| e.kind != "keystrokes");
        data.appearance.effects.push(effect(
            2,
            "keystrokes",
            vec![("show", EffectValue::Text("shortcuts".into()))],
        ));
        let shortcuts = frame(&Composition::prepare(&project, &scratch.0).unwrap());
        let slot = project.clips.order()[0];
        let data = project.clips.data_mut(slot).unwrap();
        data.appearance.effects.retain(|e| e.kind != "keystrokes");
        data.enabled.keys = Some(false);
        let plain = frame(&Composition::prepare(&project, &scratch.0).unwrap());
        assert!(shortcuts == plain, "show:shortcuts hides a typed key");
        let people = changed(&human, &plain).expect("the chip draws for people");
        let agents = changed(&agent, &plain).expect("the agent sees a chip");
        // Same place: the chips start at the same corner (their text widths differ).
        assert!(
            people[0].abs_diff(agents[0]) <= 1 && people[1].abs_diff(agents[1]) <= 1,
            "chip for people {people:?}, for agents {agents:?}"
        );
        // Recorder's overlay: categories, the 3×3 grid and the display duration. An arrow key
        // is navigation (a glyph, drawn for agents too); a chip at top-right lands in that
        // quadrant; `hold` ends it.
        fs::write(
            take.join("events.json"),
            r#"{"t0":0,"source":{"kind":"display","rect":{"x":0,"y":0,"width":96,"height":64},"scale":1},"events":[
                {"t":0.0,"k":"cursor","id":"c"},{"t":0.0,"k":"move","x":0.25,"y":0.25},
                {"t":0.1,"k":"key","ch":"\u001c","keyCode":123,"mods":0}]}"#,
        )
        .unwrap();
        let render = |project: &Project, params: Vec<(&str, EffectValue)>, at: Frame| {
            let mut project = project.clone();
            let slot = project.clips.order()[0];
            let data = project.clips.data_mut(slot).unwrap();
            data.enabled.keys = Some(true);
            data.appearance.effects.retain(|e| e.kind != "keystrokes");
            data.appearance
                .effects
                .push(effect(2, "keystrokes", params));
            let composition = Composition::prepare(&project, &scratch.0).unwrap();
            let [w, h] = composition.size().map(|n| n as usize);
            let mut rgba = vec![0; w * h * 4];
            composition.read_frame(at, &mut rgba).unwrap();
            rgba
        };
        let text = |name: &str| EffectValue::Text(name.into());
        let bare = |at: Frame| render(&project, vec![("show", text("shortcuts"))], at);
        let plain = bare(5);
        assert!(
            render(&project, vec![("show", text("shortcuts+typing"))], 5) == plain,
            "an arrow key is not typing or a shortcut"
        );
        let corner = changed(
            &render(
                &project,
                vec![
                    ("show", text("navigation")),
                    ("position", text("top-right")),
                ],
                5,
            ),
            &plain,
        )
        .expect("a navigation chip draws");
        let [w, h] = project.size.map(|n| n as usize);
        assert!(
            corner[0] > w / 3 && corner[3] < h / 2,
            "top-right chip at {corner:?}"
        );
        let held = |hold: f64| {
            let params = vec![
                ("show", text("navigation")),
                ("hold", EffectValue::Number(hold)),
            ];
            render(&project, params, 20) != bare(20)
        };
        assert!(
            held(1.2),
            "drawn 0.57 s after the key with the default hold"
        );
        assert!(!held(0.3), "gone 0.3 s after the key");
    }

    /// T13.1: a sound item on an audio lane reaches the exported mix; export.start {fps}
    /// resamples the picture.
    pub(super) fn native_music_on_an_audio_lane_is_exported() {
        let mut project = Project {
            size: [96, 64],
            media: vec![video()],
            ..Project::default()
        };
        let mut music = video();
        music.id = MediaId::from_serial(2);
        music.kind = MediaKind::Audio;
        music.path = "stereo.wav".into();
        music.probe = Probe {
            fps: 30.0,
            frames: 3,
            width: 0,
            height: 0,
            audio: vec![AudioTrack {
                name: "track 1".into(),
                channels: Some(2),
            }],
            codec: None,
            loudness: None,
        };
        project.media.push(music);
        let lane = LaneId::from_serial(7);
        project.lanes.push(Lane {
            id: lane,
            name: "Music".into(),
            kind: LaneKind::Audio,
            effects: Vec::new(),
        });
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 9,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(MediaId::from_serial(1)),
                    ..ClipData::default()
                },
            })
            .unwrap();
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 3,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Sound(Sound {
                        media: MediaId::from_serial(2),
                        source: 0,
                        volume: 0.5,
                        duck: None,
                    }),
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
        let composition = Composition::prepare(&project, &fixture()).unwrap();
        let mut plan = composition.evaluator().buffer();
        composition.evaluator().frame(1, &mut plan);
        assert!(
            plan.audio
                .iter()
                .any(|a| a.item == TimelineId::Item(ItemId::from_serial(1)) && a.gain == 0.5)
        );
        let scratch =
            Output(std::env::temp_dir().join(format!("splice-av-music-{}", std::process::id())));
        fs::create_dir(&scratch.0).unwrap();
        let file = scratch.0.join("music.mp4");
        let mut job = composition
            .start_export(&file, &Encoding::new(Codec::H264))
            .unwrap();
        while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
        assert_eq!(job.status(), ExportState::Complete);
        let exported = apple_sys::media::probe(&file).unwrap();
        assert_eq!(
            exported.audio.first().and_then(|track| track.channels),
            Some(2)
        );
        // export.start {fps}: the 30 fps timeline (9 frames, 0.3 s) at 60 and 24 fps keeps its
        // length; frames repeat or drop (frame sampling, splice_eval::resample).
        for fps in [60, 24] {
            let file = scratch.0.join(format!("music-{fps}.mp4"));
            let encoding = Encoding {
                fps,
                ..Encoding::new(Codec::H264)
            };
            let mut job = composition.start_export(&file, &encoding).unwrap();
            while let ExportState::Running(_) = job.step(Duration::from_millis(16)).unwrap() {}
            assert_eq!(job.status(), ExportState::Complete);
            let exported = apple_sys::media::probe(&file).unwrap();
            let rate = exported.video.and_then(|v| v.nominal_fps).unwrap();
            assert!((rate - f64::from(fps)).abs() < 1.0, "{fps} fps: {rate}");
            assert!(
                (exported.duration_seconds - 0.3).abs() <= 1.0 / 24.0 + 1e-3,
                "{fps} fps: {} s",
                exported.duration_seconds
            );
        }
    }
    /// Agent and viewer renders draw a missing file as a hatched, labeled placeholder at the
    /// item's geometry and report it; delivery refuses it (an export never has placeholders).
    pub(super) fn offline_media_draws_a_placeholder_for_agents_and_blocks_export() {
        let lane = LaneId::from_serial(1);
        let mut missing = video();
        missing.path = "missing-offline-take.mov".into();
        missing.probe.width = 160;
        missing.probe.height = 90;
        missing.probe.audio = vec![AudioTrack {
            name: "stereo".into(),
            channels: Some(2),
        }];
        let media = missing.id;
        let mut project = Project {
            size: [320, 180],
            media: vec![missing],
            lanes: vec![Lane {
                id: lane,
                name: "Overlay".into(),
                kind: LaneKind::Overlay,
                effects: Vec::new(),
            }],
            ..Project::default()
        };
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 10,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media),
                    ..ClipData::default()
                },
            })
            .unwrap();
        project
            .items
            .insert(Item {
                id: ItemId::from_serial(1),
                timing: ItemTiming {
                    anchor: Anchor::Pinned(20),
                    length: 6,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Media(Overlay {
                        media,
                        source: 0,
                        placement: Placement::TopLeft,
                        behind_presenter: false,
                        muted: true,
                        framing: Framing::default(),
                    }),
                    appearance: Appearance::default(),
                },
            })
            .unwrap();
        let file = fixture().join("missing-offline-take.mov");
        assert!(!file.exists());
        match Composition::prepare(&project, &fixture()) {
            Err(Error::Offline(path)) => assert_eq!(path, file),
            Err(error) => panic!("{error}"),
            Ok(_) => panic!("export composition built with offline media"),
        }
        for composition in [
            Composition::prepare_for_agent(&project, &fixture()).unwrap(),
            Composition::prepare_for_viewer(&project, &fixture()).unwrap(),
        ] {
            assert_eq!(composition.offline(), [(media, file.clone())]);
            // The spine clip fills the canvas: neutral dark fill or hatch, never black.
            let fill = pixel(&composition, 5, [12, 12]);
            assert!(
                fill.iter().all(|c| (28..=76).contains(c)) && fill[0].abs_diff(fill[1]) <= 4,
                "{fill:?}"
            );
            // Both shades of the hatch and the light label text are drawn.
            let [w, h] = composition.size().map(|n| n as usize);
            let mut rgba = vec![0; w * h * 4];
            composition.read_frame(5, &mut rgba).unwrap();
            let shade = |p: &[u8]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
            let dark = rgba
                .chunks(4)
                .filter(|p| (90..140).contains(&shade(p)))
                .count();
            let light = rgba
                .chunks(4)
                .filter(|p| (160..230).contains(&shade(p)))
                .count();
            let text = rgba.chunks(4).filter(|p| shade(p) > 400).count();
            assert!(
                dark > w * h / 4 && light > w * h / 20 && text > 40,
                "{dark} {light} {text}"
            );
            // The overlay keeps its place and size: placeholder inside, the black gap outside.
            assert!(brightness(pixel(&composition, 22, [63, 39])) > 90);
            assert!(brightness(pixel(&composition, 22, [240, 150])) <= 12);
            let scratch = std::env::temp_dir().join("splice-av-offline-export.mp4");
            match composition.start_export(&scratch, &Encoding::new(Codec::H264)) {
                Err(Error::Offline(path)) => assert_eq!(path, file),
                Err(error) => panic!("{error}"),
                Ok(_) => panic!("exported placeholders"),
            }
            assert!(!scratch.exists());
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    // OGPU's Metal backend owns its handles on the process main thread.
    harness::run(&[
        (
            "native_composition_uses_source_time_and_composites_overlay_fades_over_a_gap",
            native::native_composition_uses_source_time_and_composites_overlay_fades_over_a_gap,
        ),
        (
            "native_composition_covers_a_title_and_audio_without_spine_video",
            native::native_composition_covers_a_title_and_audio_without_spine_video,
        ),
        (
            "animated_title_keeps_its_box",
            native::animated_title_keeps_its_box,
        ),
        (
            "native_take_draws_the_recorded_cursor_and_zooms_on_click",
            native::native_take_draws_the_recorded_cursor_and_zooms_on_click,
        ),
        (
            "native_music_on_an_audio_lane_is_exported",
            native::native_music_on_an_audio_lane_is_exported,
        ),
        (
            "offline_media_draws_a_placeholder_for_agents_and_blocks_export",
            native::offline_media_draws_a_placeholder_for_agents_and_blocks_export,
        ),
    ]);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
