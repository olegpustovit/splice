use super::*;
use splice_model::*;
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

#[test]
fn native_composition_uses_source_time_and_composites_overlay_fades_over_a_gap() {
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

#[test]
fn native_composition_covers_a_title_and_audio_without_spine_video() {
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
    let job = composition.start_export(&file, false).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match job.status().unwrap() {
            ExportState::Running(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
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
    assert!(pixels.chunks_exact(4).all(|pixel| pixel[0] < 40));
    delivered.read_frame(5, &mut pixels).unwrap();
    let brightest = pixels.chunks_exact(4).map(|pixel| pixel[0]).max().unwrap();
    assert!(brightest > 120, "brightest title pixel was {brightest}");
}
