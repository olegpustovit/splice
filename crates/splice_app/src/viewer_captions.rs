use pxui::{
    Fill, Label, Paint, Point, Rect, TextSpan, Ui,
    geom::Rgba,
    ui::{Menu, MenuItem},
};
use splice_model::{
    Effect, EffectValue, ItemKind, Lane, LaneId, LaneKind, Project, TimelineId,
    timing::visible_words,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Bold,
    Karaoke,
    Pop,
    Box,
    Clean,
    Typewriter,
}

impl Style {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Bold => "bold",
            Self::Karaoke => "karaoke",
            Self::Pop => "pop",
            Self::Box => "box",
            Self::Clean => "clean",
            Self::Typewriter => "typewriter",
        }
    }
}

pub(super) const STYLES: [(Style, &str); 6] = [
    (Style::Bold, "Bold"),
    (Style::Karaoke, "Karaoke"),
    (Style::Pop, "Pop"),
    (Style::Box, "Box"),
    (Style::Clean, "Clean"),
    (Style::Typewriter, "Typewriter"),
];
const WORDS: [(i32, &str); 5] = [
    (1, "1 word"),
    (2, "2 words"),
    (3, "3 words"),
    (4, "4 words"),
    (6, "6 words"),
];

const GENERATE: i64 = 0;
const FILLERS: i64 = 9;
const VISIBILITY: i64 = 10;
const EDIT_LANE: i64 = 11;
const COPY_SRT: i64 = 12;
const STYLE_START: i64 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Settings {
    pub max_words: i32,
    pub fillers: bool,
    pub style: Option<Style>,
    pub lane: Option<LaneId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    SetStyle { lane: LaneId, style: Style },
    SetVisibility { lane: LaneId, enabled: bool },
    EditLane(LaneId),
    CopySrt,
}

pub(super) struct Intents {
    pub open: bool,
    pub generate: Option<Settings>,
    pub action: Option<Action>,
}

pub(super) fn paint(ui: &mut Ui, project: Option<&Project>, [width, height]: [f32; 2]) -> Intents {
    let lane = project.and_then(|project| {
        project
            .lanes
            .iter()
            .find(|lane| lane.kind == LaneKind::Captions)
    });
    let lane_id = lane.map(|lane| lane.id);
    let (visible, preset) = project
        .zip(lane)
        .map_or((false, None), |(project, lane)| appearance(project, lane));
    let has_cues = project.zip(lane_id).is_some_and(|(project, lane)| {
        project.items.order().iter().any(|&slot| {
            project.items.data(slot).is_some_and(|item| {
                item.lane == lane && matches!(item.kind, ItemKind::Caption { .. })
            })
        })
    });
    let (spoken, with_fillers) = project.map_or((false, false), available_words);
    let word_menu = WORDS.map(|(words, title)| MenuItem::new(title).id(i64::from(words)));
    let style_menu: [_; STYLES.len()] = std::array::from_fn(|index| {
        let (style, title) = STYLES[index];
        MenuItem::new(title)
            .checked(preset == Some(style))
            .id(STYLE_START + index as i64)
    });
    let title = if has_cues {
        "Regenerate Captions"
    } else {
        "Generate Captions"
    };
    let items = [
        MenuItem::new(title).id(GENERATE).disabled(!spoken),
        MenuItem::new("Words per Caption")
            .submenu(&word_menu)
            .disabled(!spoken),
        MenuItem::new("Include Filler Words")
            .id(FILLERS)
            .disabled(!with_fillers),
        MenuItem::new(""),
        MenuItem::new("Style")
            .submenu(&style_menu)
            .disabled(lane.is_none()),
        MenuItem::new("Show Captions")
            .checked(visible)
            .id(VISIBILITY)
            .disabled(lane.is_none()),
        MenuItem::new(""),
        MenuItem::new("Edit Captions Lane…")
            .id(EDIT_LANE)
            .disabled(lane.is_none()),
        MenuItem::new("Copy as SRT")
            .id(COPY_SRT)
            .disabled(!has_cues),
    ];
    let menu = Menu::DEFAULT
        .w_px(208.0)
        .row_h(22.0)
        .padding(4.0)
        .max_h((height - 8.0).max(1.0))
        .focus_on_click(true)
        .name("Captions");
    let x = if width <= 480.0 { 110.0_f32 } else { 239.0_f32 };
    let x = x.min((width - 212.0).max(4.0));
    let picked = ui.menu("viewer-captions", Point::new(x, 62.0), &items, &menu);

    let mut generate = None;
    let mut action = None;
    match picked.chosen_id {
        Some(id @ (GENERATE | FILLERS | 1 | 2 | 3 | 4 | 6)) => {
            let max_words = if matches!(id, GENERATE | FILLERS) {
                3
            } else {
                id as i32
            };
            generate = Some(Settings {
                max_words,
                fillers: id == FILLERS,
                style: None,
                lane: lane_id,
            });
        }
        Some(COPY_SRT) => action = Some(Action::CopySrt),
        Some(id) => {
            if let Some(lane) = lane_id {
                action = match id {
                    VISIBILITY => Some(Action::SetVisibility {
                        lane,
                        enabled: !visible,
                    }),
                    EDIT_LANE => Some(Action::EditLane(lane)),
                    _ => id
                        .checked_sub(STYLE_START)
                        .and_then(|index| STYLES.get(index as usize))
                        .map(|&(style, _)| Action::SetStyle { lane, style }),
                };
            }
        }
        None => {}
    }
    Intents {
        open: !picked.dismissed && picked.chosen.is_none(),
        generate,
        action,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Guide {
    label: &'static str,
    margins: [f32; 4],
}

const TIKTOK: Guide = Guide {
    label: "tiktok safe zone",
    margins: [0.10, 0.20, 0.05, 0.15],
};
const REELS: Guide = Guide {
    label: "reels safe zone",
    margins: [0.14, 0.20, 0.05, 0.12],
};
const SHORTS: Guide = Guide {
    label: "shorts safe zone",
    margins: [0.12, 0.18, 0.05, 0.14],
};

pub(super) fn safe_guide(
    project: &Project,
    selection: Option<TimelineId>,
    lane: Option<LaneId>,
) -> Option<Guide> {
    let cue = match selection {
        Some(TimelineId::Item(id)) => project
            .items
            .find(id)
            .and_then(|slot| project.items.data(slot))
            .filter(|item| matches!(item.kind, ItemKind::Caption { .. })),
        _ => None,
    };
    let lane = cue.map(|item| item.lane).or(lane).and_then(|id| {
        project
            .lanes
            .iter()
            .find(|lane| lane.id == id && lane.kind == LaneKind::Captions)
    });
    if cue.is_none() && lane.is_none() {
        return None;
    }
    let appearance = caption_appearance(&[
        &project.effects,
        lane.map_or(&[], |lane| lane.effects.as_slice()),
        cue.map_or(&[], |cue| cue.appearance.effects.as_slice()),
    ]);
    if appearance.enabled {
        appearance.guide
    } else {
        None
    }
}

impl Guide {
    pub(super) fn paint(&self, paint: &mut Paint<'_>, destination: Rect) {
        if destination.w <= 1.0 || destination.h <= 1.0 {
            return;
        }
        let [top, bottom, left, right] = self.margins;
        let left = destination.w * left;
        let right = destination.w * right;
        let top = destination.h * top;
        let bottom = destination.h * bottom;
        let inside = Rect::new(
            destination.x + left,
            destination.y + top,
            destination.w - left - right,
            destination.h - top - bottom,
        );
        let shade = Fill::new(Rgba::rgb(255, 90, 60).with_alpha(33));
        for band in [
            Rect::new(destination.x, destination.y, destination.w, top),
            Rect::new(destination.x, inside.y + inside.h, destination.w, bottom),
            Rect::new(destination.x, inside.y, left, inside.h),
            Rect::new(inside.x + inside.w, inside.y, right, inside.h),
        ] {
            paint.fill(band, shade);
        }

        let line = Fill::new(Rgba::rgb(255, 120, 90).with_alpha(204));
        let width = (inside.w - 1.0).max(0.0);
        let height = (inside.h - 1.0).max(0.0);
        for dash in 0..(width / 8.0).ceil() as u32 {
            let offset = dash as f32 * 8.0;
            let length = (width - offset).min(4.0);
            let x = inside.x + 0.5 + offset;
            paint.fill(Rect::new(x, inside.y, length, 1.0), line);
            paint.fill(Rect::new(x, inside.y + inside.h - 1.0, length, 1.0), line);
        }
        for dash in 0..(height / 8.0).ceil() as u32 {
            let offset = dash as f32 * 8.0;
            let length = (height - offset).min(4.0);
            let y = inside.y + 0.5 + offset;
            paint.fill(Rect::new(inside.x, y, 1.0, length), line);
            paint.fill(Rect::new(inside.x + inside.w - 1.0, y, 1.0, length), line);
        }
        if inside.w >= 40.0 && inside.h >= 14.0 {
            let ink = Rgba::rgb(255, 150, 120).with_alpha(242);
            let span = TextSpan::new(0, self.label.len() as u32).color(ink);
            let _ = paint.text_spans(
                Point::new(inside.x + 4.0, inside.y + 2.0),
                Label::MONO.size(10.0).ellipsis(),
                inside.w - 8.0,
                self.label,
                &[span],
            );
        }
    }
}

struct CaptionAppearance {
    enabled: bool,
    preset: Option<Style>,
    guide: Option<Guide>,
}

fn appearance(project: &Project, lane: &Lane) -> (bool, Option<Style>) {
    let style = caption_appearance(&[&project.effects, &lane.effects]);
    (style.enabled, style.preset)
}

pub(super) fn preset(scopes: &[&[Effect]]) -> Option<Style> {
    caption_appearance(scopes).preset
}

fn caption_appearance(scopes: &[&[Effect]]) -> CaptionAppearance {
    let mut appearance = CaptionAppearance {
        enabled: true,
        preset: Some(Style::Bold),
        guide: Some(TIKTOK),
    };
    for scope in scopes {
        for effect in scope.iter().filter(|effect| effect.kind == "captions") {
            appearance.enabled = effect.enabled.unwrap_or(appearance.enabled);
            for parameter in &effect.params {
                let EffectValue::Text(name) = &parameter.value else {
                    continue;
                };
                match parameter.name.as_str() {
                    "preset" => {
                        appearance.preset = STYLES
                            .iter()
                            .find(|(style, _)| style.label() == name)
                            .map(|&(style, _)| style)
                    }
                    "safe" => {
                        appearance.guide = match name.as_str() {
                            "none" => None,
                            "reels" => Some(REELS),
                            "shorts" => Some(SHORTS),
                            _ => Some(TIKTOK),
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    appearance
}
pub(super) fn available_words(project: &Project) -> (bool, bool) {
    let mut available_with_fillers = false;
    for &slot in project.clips.order() {
        let mut spoken = false;
        let mut found = false;
        let resolved = visible_words(project, slot, &mut |word, _| {
            found = true;
            if !spoken {
                let word = word.text.trim_matches(|c: char| !c.is_alphabetic());
                spoken = !["um", "uh", "er", "erm", "ah", "hmm"]
                    .iter()
                    .any(|filler| word.eq_ignore_ascii_case(filler));
            }
        });
        if resolved.is_ok() {
            if spoken {
                return (true, true);
            }
            available_with_fillers |= found;
        }
    }
    (false, available_with_fillers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use splice_model::{
        Clip, ClipData, ClipId, ClipTiming, Media, MediaId, MediaKind, Probe, Transcript, Word,
        WordId,
    };

    #[test]
    fn caption_safe_guide_resolves_project_lane_and_cue_overrides() {
        use splice_model::{
            Anchor, Appearance, EffectId, EffectParam, Item, ItemData, ItemId, ItemTiming,
        };

        let effect = |serial, name: &str, enabled| Effect {
            id: EffectId::from_serial(serial),
            kind: "captions".into(),
            enabled,
            params: vec![EffectParam {
                name: "safe".into(),
                value: EffectValue::Text(name.into()),
            }],
        };
        let mut project = Project::default();
        let lane = LaneId::from_serial(1);
        project.lanes.push(Lane {
            id: lane,
            name: "Captions".into(),
            kind: LaneKind::Captions,
            effects: Vec::new(),
        });
        assert_eq!(safe_guide(&project, None, None), None);
        assert_eq!(safe_guide(&project, None, Some(lane)), Some(TIKTOK));
        project.effects.push(effect(2, "reels", None));
        assert_eq!(safe_guide(&project, None, Some(lane)), Some(REELS));
        project.lanes[0]
            .effects
            .push(effect(3, "shorts", Some(false)));
        assert_eq!(safe_guide(&project, None, Some(lane)), None);
        let id = ItemId::from_serial(4);
        let slot = project
            .items
            .insert(Item {
                id,
                timing: ItemTiming {
                    anchor: Anchor::Pinned(0),
                    length: 15,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Caption {
                        words: Vec::new(),
                        edits: Vec::new(),
                        text: Some("Hello".into()),
                    },
                    appearance: Appearance {
                        effects: vec![Effect {
                            id: EffectId::from_serial(5),
                            kind: "captions".into(),
                            enabled: Some(true),
                            params: Vec::new(),
                        }],
                        ..Appearance::default()
                    },
                },
            })
            .unwrap();
        let selected = Some(TimelineId::Item(id));
        assert_eq!(safe_guide(&project, selected, None), Some(SHORTS));
        project.items.data_mut(slot).unwrap().appearance.effects[0] = effect(5, "none", Some(true));
        assert_eq!(safe_guide(&project, selected, None), None);
    }

    #[test]
    fn caption_safe_guide_paints_without_allocations_after_warmup() {
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&pxui::UiDesc::DEFAULT).unwrap();
        let frame = pxui::FrameIn::new(320.0, 240.0);
        let mut build = |ui: &mut Ui| {
            let node = ui.node("guide-test", pxui::Node::DEFAULT.w_px(120.0).h_px(190.0));
            ui.canvas(&node, |paint, rect| TIKTOK.paint(paint, rect));
        };
        for _ in 0..8 {
            ui.frame(&frame, &mut build).unwrap();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                ui.frame(&frame, &mut build).unwrap();
            },
        );
    }
    #[test]
    fn show_captions_uses_the_lane_override_and_emits_a_visibility_edit() {
        use pxui::{Role, testing};
        use splice_model::{Effect, EffectId, EffectParam};
        use std::cell::Cell;

        let id = LaneId::from_serial(1);
        let mut project = Project::default();
        project.effects.push(Effect {
            id: EffectId::from_serial(1),
            kind: "captions".into(),
            enabled: Some(true),
            params: vec![EffectParam {
                name: "preset".into(),
                value: EffectValue::Text("pop".into()),
            }],
        });
        project.lanes.push(Lane {
            id,
            name: "Captions".into(),
            kind: LaneKind::Captions,
            effects: vec![Effect {
                id: EffectId::from_serial(2),
                kind: "captions".into(),
                enabled: Some(false),
                params: Vec::new(),
            }],
        });
        assert_eq!(
            appearance(&project, &project.lanes[0]),
            (false, Some(Style::Pop))
        );

        let action = Cell::new(None);
        let build = &mut |ui: &mut Ui| {
            let result = paint(ui, Some(&project), [320.0, 240.0]);
            if let Some(intent) = result.action {
                assert!(!result.open);
                action.set(Some(intent));
            }
        };
        let mut ui = pxui::doc::frames(2, build);
        let row = testing::find(&ui, Role::MenuItem, "Show Captions").unwrap();
        testing::click(
            &mut ui,
            f64::from(row.x + row.w * 0.5),
            f64::from(row.y + row.h * 0.5),
            build,
        );
        assert_eq!(
            action.get(),
            Some(Action::SetVisibility {
                lane: id,
                enabled: true
            })
        );
    }
    #[test]
    fn captions_require_a_word_visible_in_the_spine_and_respect_fillers() {
        let mut project = Project::default();
        let media = MediaId::from_serial(1);
        project.media.push(Media {
            id: media,
            kind: MediaKind::Take,
            path: "take.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 60,
                width: 1920,
                height: 1080,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        project.transcripts.push(Transcript {
            media,
            hash: 1,
            language: "en".into(),
            words: vec![
                Word {
                    id: WordId::new(media, 1),
                    text: "UH!".into(),
                    from: 4,
                    to: 9,
                    confidence: None,
                },
                Word {
                    id: WordId::new(media, 2),
                    text: "Hello".into(),
                    from: 24,
                    to: 31,
                    confidence: None,
                },
            ],
        });
        assert_eq!(available_words(&project), (false, false));
        let slot = project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 20,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData {
                    media: Some(media),
                    ..ClipData::default()
                },
            })
            .unwrap();
        assert_eq!(available_words(&project), (false, true));
        project.clips.timing_mut(slot).unwrap().length = 40;
        assert_eq!(available_words(&project), (true, true));
    }
}
