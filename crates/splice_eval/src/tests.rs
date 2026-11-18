use super::*;
use splice_core::package::Package;
use splice_model::Project;
use std::path::Path;

fn sample() -> Project {
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
                        p.scale,
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
                Layer::Fade { opacity } => writeln!(&mut output, "fade {opacity:.3}").unwrap(),
                Layer::Title {
                    item,
                    bounds,
                    opacity,
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
        for audio in &plan.audio {
            writeln!(
                &mut output,
                "audio {} {:?} {:.3} {:.3}",
                audio.item, audio.track, audio.source_frame, audio.gain
            )
            .unwrap();
        }
    }
    assert_eq!(output, include_str!("../../../fixtures/render/sample.txt"));
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
    let fade = index(&|layer| matches!(layer, Layer::Fade { opacity: 0.5 }));
    assert!(behind < camera && camera < fade && fade < front);
    assert_eq!(output.audio.len(), 1);
    assert_eq!(output.audio[0].track, Track::System);
    assert_eq!(output.audio[0].gain, 0.4);
}
