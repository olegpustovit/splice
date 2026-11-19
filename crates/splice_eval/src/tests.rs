use super::*;
use splice_core::package::Package;
use splice_model::Project;
use std::path::Path;

pub(crate) fn sample() -> Project {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let path = std::env::temp_dir().join(format!("splice-eval-{}.splice", std::process::id()));
    std::fs::create_dir_all(path.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        std::fs::copy(source.join(file), path.join(file)).unwrap();
    }
    let (package, project) = Package::open(&path).unwrap();
    drop(package);
    std::fs::remove_dir_all(path).unwrap();
    project
}

const SAMPLE_FRAMES: [Frame; 10] = [0, 7, 16, 119, 120, 150, 240, 422, 550, 659];

#[test]
fn warm_render_plan_allocates_nothing_across_sample_frames() {
    let evaluator = Evaluator::prepare(&sample()).unwrap();
    let mut plan = evaluator.buffer();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for at in SAMPLE_FRAMES {
                evaluator.frame(at, &mut plan);
                std::hint::black_box(&plan);
            }
        },
    );
}

#[test]
fn render_plan_matches_ten_prototype_frames() {
    use std::fmt::Write;
    let evaluator = Evaluator::prepare(&sample()).unwrap();
    let mut plan = evaluator.buffer();
    let mut output = String::new();
    for at in SAMPLE_FRAMES {
        evaluator.frame(at, &mut plan);
        writeln!(&mut output, "frame {at} {}", plan.clip.unwrap()).unwrap();
        for layer in &plan.layers {
            match layer {
                Layer::Background {
                    bounds,
                    fill,
                    opacity,
                } => {
                    let fill = match fill {
                        Background::None => "-".into(),
                        Background::Solid(a) => format!("{a:06x}"),
                        Background::Gradient(a, b) => format!("{a:06x}/{b:06x}"),
                    };
                    writeln!(
                        &mut output,
                        "background {} {fill} {opacity:.3}",
                        rect_text(*bounds)
                    )
                    .unwrap();
                }
                Layer::Picture(p) => {
                    let padding = p.window.map_or("-".into(), |win| {
                        format!("{:.3}/{:.3}", win.padding, win.corner)
                    });
                    let cursor = p.cursor.map_or("-".into(), |style| {
                        format!("{:.3}/{}", style.scale, style.click_rings)
                    });
                    writeln!(
                        &mut output,
                        "picture {} {} {:.3} {} {} {:.3} {:.3} {padding} {cursor}",
                        p.item,
                        p.media,
                        p.source_frame,
                        rect_text(p.source),
                        rect_text(p.destination),
                        p.view.amount,
                        p.opacity
                    )
                    .unwrap();
                }
                Layer::Camera {
                    clip,
                    source_frame,
                    bounds,
                    radius,
                    shadow,
                    ..
                } => {
                    writeln!(
                        &mut output,
                        "camera {clip} {source_frame:.3} {} {radius:.3} {shadow:.3}",
                        rect_text(*bounds)
                    )
                    .unwrap();
                }
                Layer::Keystrokes {
                    clip,
                    source_frame,
                    style,
                    ..
                } => {
                    writeln!(
                        &mut output,
                        "keys {clip} {source_frame:.3} {:.3} {:?}",
                        style.scale, style.position
                    )
                    .unwrap();
                }
                Layer::Fade { opacity, .. } => writeln!(&mut output, "fade {opacity:.3}").unwrap(),
                Layer::Shape { item, bounds, .. } => {
                    writeln!(&mut output, "shape {item} {}", rect_text(*bounds)).unwrap();
                }
                Layer::Title {
                    item,
                    bounds,
                    opacity,
                    ..
                } => {
                    writeln!(
                        &mut output,
                        "title {item} {} {opacity:.3} {}",
                        rect_text(*bounds),
                        evaluator.title(*item).unwrap()
                    )
                    .unwrap();
                }
                Layer::Caption {
                    item,
                    word,
                    opacity,
                    style,
                    ..
                } => {
                    let words = evaluator
                        .caption_words(*item)
                        .unwrap()
                        .iter()
                        .map(|w| {
                            format!(
                                "{}:{}-{}:{}{}",
                                w.id,
                                w.start,
                                w.end,
                                w.text,
                                w.emoji.as_deref().unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    writeln!(
                        &mut output,
                        "caption {item} {} {opacity:.3} {:?} {words}",
                        word.map_or("-".into(), |index| index.to_string()),
                        style.preset
                    )
                    .unwrap();
                }
            }
        }
        if let Some(g) = plan.group {
            writeln!(
                &mut output,
                "group {:?} {:.3} {:.3} {:?} {:?}",
                g.layers, g.opacity, g.scale, g.offset, g.clip
            )
            .unwrap();
        }
        if let Some(s) = plan.shift {
            writeln!(
                &mut output,
                "shift {:?} {:.3} {:?}",
                s.layers, s.scale, s.offset
            )
            .unwrap();
        }
        for audio in &plan.audio {
            writeln!(
                &mut output,
                "audio {} {:?} {:.3} {:.3}",
                audio.item, audio.track, audio.source_frame, audio.gain
            )
            .unwrap();
        }
    }
    // Began as the JS prototype's frames; 0.4's spring camera differs on purpose, so the golden
    // is blessed (SPLICE_BLESS=1) and reviewed as a diff.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render/sample.txt");
    splice_testkit::golden(&path, output.as_bytes()).unwrap();
}

fn rect_text(rect: Rect) -> String {
    format!("{:.3},{:.3},{:.3},{:.3}", rect.x, rect.y, rect.w, rect.h)
}

#[test]
fn effect_parameters_merge_by_scope_and_enable_override() {
    use splice_model::{Effect, EffectId, EffectParam, EffectValue};
    let param = |name: &str, value| EffectParam {
        name: name.into(),
        value: EffectValue::Number(value),
    };
    let mut project = sample();
    project.spine.effects.push(Effect {
        id: EffectId::from_serial(900),
        kind: "window".into(),
        enabled: Some(false),
        params: vec![param("corner", 0.09)],
    });
    let disabled = Evaluator::prepare(&project).unwrap();
    let mut plan = disabled.buffer();
    disabled.frame(150, &mut plan);
    assert!(matches!(
        plan.layers[0],
        Layer::Picture(Picture { window: None, .. })
    ));

    let id = "clp_take".parse().unwrap();
    let slot = project.clips.find(id).unwrap();
    project.clips.data_mut(slot).unwrap().appearance.effects[0].enabled = Some(true);
    let enabled = Evaluator::prepare(&project).unwrap();
    enabled.frame(150, &mut plan);
    let picture = plan
        .layers
        .iter()
        .find_map(|layer| {
            if let Layer::Picture(p) = layer
                && p.item == TimelineId::Clip(id)
            {
                Some(p)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        picture.window.unwrap(),
        Window {
            padding: 0.05,
            corner: 0.09,
            shadow: 0.6,
            inset: 0.0,
            background: Background::Gradient(0x6b5a7d, 0x1e1a26),
        }
    );
    assert_eq!(picture.cursor.unwrap().scale, 1.5);
}

#[test]
fn take_fade_applies_to_audio_and_behind_presenter_layers() {
    use splice_model::ItemKind;
    let mut project = sample();
    let id = "clp_take".parse().unwrap();
    let slot = project.clips.find(id).unwrap();
    let data = project.clips.data_mut(slot).unwrap();
    data.appearance.fade_out = 120;
    data.enabled.mic = Some(false);
    let overlay = "ovl_broll".parse().unwrap();
    let slot = project.items.find(overlay).unwrap();
    let ItemKind::Media(item) = &mut project.items.data_mut(slot).unwrap().kind else {
        panic!()
    };
    item.behind_presenter = true;
    let plan = Evaluator::prepare(&project).unwrap();
    let mut output = plan.buffer();
    plan.frame(480, &mut output);
    let index = |needle: &dyn Fn(&Layer) -> bool| output.layers.iter().position(needle).unwrap();
    let overlay = TimelineId::Item(overlay);
    let other = TimelineId::Item("ovl_logo".parse().unwrap());
    let behind =
        index(&|layer| matches!(layer, Layer::Picture(picture) if picture.item == overlay));
    let front = index(&|layer| matches!(layer, Layer::Picture(picture) if picture.item == other));
    let camera = index(&|layer| matches!(layer, Layer::Camera { .. }));
    // The take (with its behind-presenter overlay and camera) fades as one group over its
    // backdrop; front overlays stay out of it.
    let group = output.group.expect("fade group");
    assert_eq!(group.opacity, 0.5);
    assert!(group.layers[0] <= behind && behind < camera && camera < group.layers[1]);
    assert!(group.layers[1] <= front);
    assert!(!matches!(
        output.layers[group.layers[0]],
        Layer::Background { .. }
    ));
    assert_eq!(output.audio.len(), 1);
    assert_eq!(output.audio[0].track, Track::System);
    assert_eq!(output.audio[0].gain, 0.4);
}

#[test]
fn composition_breakpoints_partition_the_sample_render_plan() {
    let evaluator = Evaluator::prepare(&sample()).unwrap();
    let cuts = evaluator.breakpoints();
    assert_eq!(cuts[0], 0);
    assert_eq!(cuts.last(), Some(&660));
    assert!(cuts.len() < 100);
    assert!(cuts.windows(2).all(|pair| pair[0] < pair[1]));
    let mut first = evaluator.buffer();
    let mut last = evaluator.buffer();
    for range in cuts.windows(2) {
        evaluator.frame(range[0], &mut first);
        evaluator.frame(range[1] - 1, &mut last);
        assert_eq!(first.clip, last.clip);
        assert_eq!(first.layers.len(), last.layers.len(), "range {range:?}");
        assert_eq!(first.audio.len(), last.audio.len(), "range {range:?}");
        for (a, b) in first.layers.iter().zip(&last.layers) {
            assert_eq!(std::mem::discriminant(a), std::mem::discriminant(b));
        }
    }
}

#[test]
fn music_ducks_under_transcript_speech_and_ramps_back() {
    use splice_model::{
        Anchor, Appearance, AudioTrack, Duck, Item, ItemData, ItemId, ItemKind, ItemTiming, Lane,
        LaneId, LaneKind, Media, MediaKind, Probe, Sound, TimelineId,
    };
    let mut project = sample();
    project.media.push(Media {
        id: "med_music".parse().unwrap(),
        kind: MediaKind::Audio,
        path: "music.m4a".into(),
        bookmark: None,
        size: 1,
        mtime: 0,
        probe: Probe {
            fps: 30.0,
            frames: 600,
            width: 0,
            height: 0,
            audio: vec![AudioTrack {
                name: "audio".into(),
                channels: Some(2),
            }],
            codec: None,
            loudness: Some(-20.0),
        },
        take: None,
    });
    let lane = LaneId::from_serial(9_999);
    project.lanes.push(Lane {
        id: lane,
        name: "Music".into(),
        kind: LaneKind::Audio,
        effects: Vec::new(),
    });
    let music: ItemId = "ovl_music".parse().unwrap();
    project
        .items
        .insert(Item {
            id: music,
            timing: ItemTiming {
                anchor: Anchor::Pinned(0),
                length: 600,
            },
            data: ItemData {
                lane,
                kind: ItemKind::Sound(Sound {
                    media: "med_music".parse().unwrap(),
                    source: 0,
                    volume: 0.5,
                    duck: Some(Duck {
                        amount: 0.3,
                        attack: 8,
                        release: 15,
                    }),
                }),
                appearance: Appearance::default(),
            },
        })
        .unwrap();
    let evaluator = Evaluator::prepare(&project).unwrap();
    let mut plan = evaluator.buffer();
    let gain = |plan: &mut FramePlan, at| {
        evaluator.frame(at, plan);
        plan.audio
            .iter()
            .find(|a| a.item == TimelineId::Item(music))
            .map(|a| a.gain)
            .unwrap()
    };
    // The opener has no speech: full volume. The take's first word starts at frame 126.
    assert!((gain(&mut plan, 60) - 0.5).abs() < 1e-9);
    assert!((gain(&mut plan, 130) - 0.15).abs() < 1e-9);
    let ramp = gain(&mut plan, 122);
    assert!(ramp < 0.5 && ramp > 0.15, "attack ramp {ramp}");
    // The duck's edges are breakpoints, so renderers ramp exactly.
    let cuts = evaluator.breakpoints();
    assert!(cuts.contains(&118) && cuts.contains(&126), "{cuts:?}");
}
