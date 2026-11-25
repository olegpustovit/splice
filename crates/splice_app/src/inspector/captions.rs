use super::{
    BODY, CAPTION, CHOICE_LOOK, FIELD, INK, Intents, LINE, MUTED, QUIET, SELECTED, STRONG_LINE,
    Selection, ink, item_setting, lane_choice, numeric, property_row, section, style_field,
    text_field,
};
use crate::{
    EDITOR_HEADING, caption_appearance,
    viewer_captions::{self, STYLES, Style},
};
use pxui::{
    Button, Fill, Label, Node, Rect, Toggle, Ui, ValueCallbacks,
    geom::{Edges, Rgba},
    ui::{Align, Justify, Look, Size, States},
};
use splice_json::Value as Json;
use splice_model::{ItemId, ItemKind, Lane, LaneKind, MediaId, Project, TimelineId, WordId};

const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const CAPTION_INK: Rgba = Rgba::rgb(242, 230, 166);
const ACTIVE: Rgba = Rgba::rgb(255, 210, 63);
const CHIP_LABEL: Label = EDITOR_HEADING.size(14.0).ellipsis();

#[derive(Default)]
pub(crate) struct Cache {
    selection: Option<(*const Project, u64, u64, Selection)>,
    pub(super) cue: Option<Cue>,
    pub(super) lane: Option<LaneDetails>,
}

pub(super) struct Cue {
    title: String,
    text: String,
    words: Vec<Word>,
    first: Option<WordId>,
    next: Option<ItemId>,
    range: [i32; 2],
    from_words: bool,
}

struct Word {
    id: WordId,
    original: String,
    text: Option<String>,
    emoji: Option<String>,
    label: String,
    width: f32,
    emph: bool,
    hidden: bool,
    span: [i32; 2],
}

impl Word {
    fn text(&self) -> &str {
        self.text.as_deref().unwrap_or(&self.original)
    }
}

pub(super) struct LaneDetails {
    count: usize,
    hash: Option<u64>,
    can_generate: bool,
}

impl Cache {
    pub(crate) fn refresh(&mut self, ui: &mut Ui, project: Option<&Project>, selected: Selection) {
        let key = project.map(|p| (std::ptr::from_ref(p), p.rev, p.hash, selected));
        if self.selection == key {
            return;
        }
        self.selection = key;
        self.cue = None;
        self.lane = None;
        if let Some(project) = project {
            match selected {
                Selection::Item(id) => self.cue = Cue::new(ui, project, id),
                Selection::Lane(id) => {
                    if project
                        .lanes
                        .iter()
                        .any(|l| l.id == id && l.kind == LaneKind::Captions)
                    {
                        let count = project
                            .items
                            .order()
                            .iter()
                            .filter(|&&slot| {
                                project.items.data(slot).is_some_and(|item| item.lane == id)
                            })
                            .count();
                        self.lane = Some(LaneDetails {
                            count,
                            hash: match project.transcripts.as_slice() {
                                [transcript] => Some(transcript.hash),
                                _ => None,
                            },
                            can_generate: viewer_captions::available_words(project).0,
                        });
                    }
                }
                _ => {}
            }
        }
    }
}

impl Cue {
    fn new(ui: &mut Ui, project: &Project, id: ItemId) -> Option<Self> {
        let slot = project.items.find(id)?;
        let data = project.items.data(slot)?;
        let ItemKind::Caption { words, edits, text } = &data.kind else {
            return None;
        };
        let timing = project.items.timing(slot)?;
        let start = splice_model::timing::anchor_start(project, timing.anchor).ok()?;
        let mut cue = Self {
            title: String::new(),
            text: String::new(),
            words: Vec::new(),
            first: words.first().copied(),
            next: None,
            range: [start, start.saturating_add(timing.length)],
            from_words: text.is_none(),
        };
        if let Some(text) = text {
            cue.text.push_str(text);
            cue.title.push_str(text);
            return Some(cue);
        }
        // Resolve the original words even when an edit hides them on the canvas.
        for word in splice_av::caption_tokens(project, timing.anchor, words, &[], None, false)
            .unwrap_or_default()
        {
            let edit = edits.iter().find(|edit| edit.word == word.id);
            let text = edit
                .and_then(|edit| edit.text.as_ref())
                .filter(|text| !text.is_empty());
            let emoji = edit.and_then(|edit| edit.emoji.as_ref());
            let shown = text.unwrap_or(&word.text);
            let mut label = shown.clone();
            if let Some(emoji) = emoji {
                label.push(' ');
                label.push_str(emoji);
            }
            let hidden = edit.is_some_and(splice_model::CaptionEdit::is_hidden);
            if !hidden {
                if !cue.text.is_empty() {
                    cue.text.push(' ');
                    cue.title.push(' ');
                }
                cue.text.push_str(shown);
                cue.title.push_str(&label);
            }
            let width = ui
                .text_service()
                .measure(&label, CHIP_LABEL, None)
                .map_or(12.0, |m| m.width)
                .ceil()
                + 18.0;
            cue.words.push(Word {
                id: word.id,
                original: word.text,
                text: text.cloned(),
                emoji: emoji.cloned(),
                label,
                width,
                emph: edit.is_some_and(|edit| edit.emph),
                hidden,
                span: [word.start, word.end],
            });
        }
        if cue.title.is_empty() {
            cue.title.push_str("(all words hidden)");
        }
        // A free-text cue is not a valid merge target. Respect the next cue's place on the lane.
        let mut next = None;
        for (order, &other) in project.items.order().iter().enumerate() {
            let other_data = project.items.data(other)?;
            if other == slot || other_data.lane != data.lane {
                continue;
            }
            let ItemKind::Caption { text, .. } = &other_data.kind else {
                continue;
            };
            let other_timing = project.items.timing(other)?;
            let Ok(at) = splice_model::timing::anchor_start(project, other_timing.anchor) else {
                continue;
            };
            if at > start && next.is_none_or(|(key, _)| (at, order) < key) {
                next = Some((
                    (at, order),
                    text.is_none().then(|| project.items.id(other)).flatten(),
                ));
            }
        }
        cue.next = next.and_then(|(_, id)| id);
        Some(cue)
    }

    pub(super) fn title(&self) -> &str {
        &self.title
    }

    pub(super) fn uses_words(&self) -> bool {
        self.from_words
    }
}

pub(super) fn cue_fields(
    ui: &mut Ui,
    project: &Project,
    target: (ItemId, Option<&Lane>),
    (cue, at): (&Cue, i32),
    (field, edit): (Rect, bool),
    intents: &mut Intents,
) {
    let (id, lane) = target;
    section(ui, "TEXT", &mut |ui| {
        text_field(
            ui,
            ("Text", "Caption text"),
            &cue.text,
            "",
            field,
            (120, edit.then_some(&mut intents.focused_field)),
            &mut |text| {
                let text = text.trim();
                if !text.is_empty() && text != cue.text {
                    intents.op = Some(item_setting(
                        id,
                        "caption.set",
                        "text",
                        Json::String(text.into()),
                    ));
                }
            },
        );
        if !cue.words.is_empty() {
            cue_words(ui, id, cue, at, field, intents);
        }
    });
    if cue.from_words {
        section(ui, "TIMING", &mut |ui| {
            let [a, b, c] = crate::playbar::clock(cue.range[0], project.fps);
            let [d, e, f] = crate::playbar::clock(cue.range[1], project.fps);
            numeric(
                ui,
                "From words",
                format_args!("{a:02}:{b:02}:{c:02} – {d:02}:{e:02}:{f:02}"),
            );
            if let Some(lane) = lane
                && project
                    .lanes
                    .iter()
                    .filter(|lane| lane.kind == LaneKind::Captions)
                    .nth(1)
                    .is_some()
            {
                lane_choice(ui, project, id, lane, field.w, intents);
            }
            let split = cue
                .words
                .iter()
                .skip(1)
                .find(|word| word.span[0] >= at.saturating_sub(1));
            numeric(ui, "Words", format_args!("{}", cue.words.len()));
            ui.row(Node::row().w(Size::Grow).gap(6.0).wrap(), |ui| {
                if control(ui, "Split at playhead", 108.0, split.is_some())
                    && let Some(word) = split
                {
                    intents.op = Some(split_at(id, word.id));
                }
                if control(ui, "Merge with next", 106.0, cue.next.is_some())
                    && let Some(next) = cue.next
                {
                    intents.op = Some(Json::Object(super::group_command(
                        "caption.merge",
                        &[TimelineId::Item(id), TimelineId::Item(next)],
                    )));
                }
            });
            hint(
                ui,
                "Timing follows the transcript when the take moves or is cut.",
            );
        });
    }
    let lane_effects = lane.map_or(&[][..], |lane| lane.effects.as_slice());
    let local = project
        .items
        .find(id)
        .and_then(|slot| project.items.data(slot))
        .map_or(&[][..], |data| data.appearance.effects.as_slice());
    presets(
        ui,
        "STYLE · THIS CAPTION",
        &[&project.effects, lane_effects, local],
        &mut |style| {
            intents.op = Some(style_op(id.to_string(), style));
        },
    );
}

fn cue_words(ui: &mut Ui, id: ItemId, cue: &Cue, at: i32, field: Rect, intents: &mut Intents) {
    let index = ui.node("word-index", Node::DEFAULT.w_px(0.0).h_px(0.0));
    let media = ui.node("word-media", Node::DEFAULT.w_px(0.0).h_px(0.0));
    let serial = ui.state(&media, 0) as u64;
    let previous = match ui.state(&index, 0) as u32 {
        0 => None,
        index if serial == 0 => Some(WordId::legacy(index)),
        index => Some(WordId::new(MediaId::from_serial(serial), index)),
    };
    let mut selected = previous.filter(|&id| cue.words.iter().any(|word| word.id == id));
    ui.row(
        Node::row()
            .w(Size::Grow)
            .gap(4.0)
            .row_gap(4.0)
            .wrap()
            .padding(Edges::new(0.0, 8.0, 0.0, 0.0)),
        |ui| {
            for word in &cue.words {
                let media = word.id.media().map_or(0, MediaId::serial);
                ui.id(media, |ui| {
                    let checked = selected == Some(word.id);
                    if word_chip(ui, word, at, checked, field.x + field.w) {
                        selected = (!checked).then_some(word.id);
                    }
                });
            }
        },
    );
    if let Some(word) = cue.words.iter().find(|word| Some(word.id) == selected) {
        ui.id(word.id.media().map_or(0, MediaId::serial), |ui| {
            ui.id(u64::from(word.id.index()), |ui| {
                if word_editor(ui, id, word, Some(word.id) != cue.first, field, intents) {
                    selected = None;
                }
            });
        });
    }
    if selected != previous {
        ui.set_state(&index, selected.map_or(0, |id| i64::from(id.index())));
        ui.set_state(
            &media,
            selected.and_then(WordId::media).map_or(0, MediaId::serial) as i64,
        );
    }
}

fn word_chip(ui: &mut Ui, word: &Word, at: i32, selected: bool, limit: f32) -> bool {
    let color = if word.hidden {
        QUIET
    } else if word.emph {
        CAPTION_INK
    } else {
        INK
    };
    let border = if word.emph { CAPTION_INK } else { LINE };
    let idle = Look::NONE.border(border, 1.0).text(color);
    let looks = States::of(idle)
        .hover(idle.border(STRONG_LINE, 1.0))
        .checked(idle.fill(SELECTED).border(INK, 1.0))
        .focus(idle.border(INK, 1.0));
    let width = word.width.min(limit);
    let response = ui.button(
        u64::from(word.id.index()),
        &word.label,
        &Button::sized(width, 24.0)
            .look(&looks)
            .label(CHIP_LABEL)
            .selected(selected),
    );
    let active = (word.span[0]..word.span[1]).contains(&at);
    let hidden = word.hidden;
    // Install the same paint hook on every frame so changing the playhead reuses its slot.
    ui.canvas_over(&response.handle, move |paint, rect| {
        if active {
            paint.fill(
                Rect::new(rect.x + 1.0, rect.y + rect.h - 2.0, rect.w - 2.0, 2.0),
                Fill::new(ACCENT),
            );
        }
        if hidden {
            paint.fill(
                Rect::new(rect.x + 8.0, rect.y + 12.0, (rect.w - 16.0).max(0.0), 1.0),
                Fill::new(QUIET),
            );
        }
    });
    response.clicked
}

fn word_editor(
    ui: &mut Ui,
    id: ItemId,
    word: &Word,
    can_split: bool,
    field: Rect,
    intents: &mut Intents,
) -> bool {
    let mut split = false;
    ui.column(
        Node::column()
            .padding(Edges::all(8.0))
            .look(Look::NONE.border(LINE, 1.0).fill(Rgba::rgb(24, 24, 22))),
        |ui| {
            let size = Rect::new(field.x, field.y, (field.w - 16.0).max(0.0), field.h);
            text_field(
                ui,
                ("Word", "Word text"),
                word.text(),
                &word.original,
                size,
                (40, None),
                &mut |text| {
                    let text = text.trim();
                    let value = if text.is_empty() {
                        Json::Null
                    } else {
                        Json::String(text.into())
                    };
                    intents.op = Some(edit_word(id, word.id, Some(("text", value))));
                },
            );
            property_row(ui, "Emoji", &mut |ui| {
                let emoji = word.emoji.as_deref().unwrap_or("");
                let valid = |text: &str| {
                    // The v13 input has a maximum of eight UTF-16 code units.
                    text.encode_utf16().count() <= 8 && !text.chars().any(char::is_control)
                };
                let response = pxui::ext::value_field(
                    ui,
                    "Word emoji",
                    emoji,
                    size,
                    &FIELD.name("Word emoji"),
                    &ValueCallbacks::DEFAULT.valid(&valid),
                );
                if let Some(value) = response.changed.filter(|&value| value != emoji) {
                    let value = value.trim();
                    let value = if value.is_empty() {
                        Json::Null
                    } else {
                        Json::String(value.into())
                    };
                    intents.op = Some(edit_word(id, word.id, Some(("emoji", value))));
                }
                let (response, editing) = (response.response, response.editing);
                style_field(ui, response, editing, emoji, "none");
            });
            let toggle = Toggle::DEFAULT.row_h(26.0).label(BODY);
            for (name, key, on) in [
                ("Emphasis", "emph", word.emph),
                ("Hide word", "hidden", word.hidden),
            ] {
                if ui.toggle(key, name, on, &toggle).changed {
                    intents.op = Some(edit_word(id, word.id, Some((key, Json::Bool(!on)))));
                }
            }
            ui.row(
                Node::row().w(Size::Grow).gap(6.0).row_gap(4.0).wrap(),
                |ui| {
                    if control(ui, "Split before", 88.0, can_split) {
                        intents.op = Some(split_at(id, word.id));
                        split = true;
                    }
                    if control(ui, "Jump to word", 90.0, true) {
                        intents.seek = Some(word.span[0]);
                    }
                    if control(ui, "Reset word", 84.0, true) {
                        intents.op = Some(edit_word(id, word.id, None));
                    }
                },
            );
            let detail = ui.label_fmt(
                format_args!("Transcript says “{}” ({}).", word.original, word.id),
                CAPTION.grow().wrap_glyph(),
            );
            ink(ui, detail, MUTED);
        },
    );
    split
}

pub(super) fn lane_fields(
    ui: &mut Ui,
    project: &Project,
    lane: &Lane,
    details: &LaneDetails,
    intents: &mut Intents,
) {
    section(ui, "CAPTIONS", &mut |ui| {
        numeric(ui, "Cues", format_args!("{}", details.count));
        if let Some(hash) = details.hash {
            let provenance = ui.label_fmt(
                format_args!("Transcript · {hash:016x}"),
                CAPTION.grow().ellipsis(),
            );
            ink(ui, provenance, MUTED);
        }
        ui.row(Node::row().gap(4.0), |ui| {
            for (words, name) in [(1, "1w"), (2, "2w"), (3, "3w"), (4, "4w"), (6, "6w")] {
                if control(ui, name, 34.0, details.can_generate) {
                    intents.op = Some(Json::Object(vec![
                        ("cmd".into(), Json::String("captions.generate".into())),
                        ("lane".into(), Json::String(lane.id.to_string())),
                        ("maxWords".into(), Json::Signed(words)),
                    ]));
                }
            }
        });
        if control(ui, "Copy as SRT", 88.0, details.count != 0) {
            intents.copy_srt = true;
        }
    });
    presets(
        ui,
        "STYLE · WHOLE LANE",
        &[&project.effects, &lane.effects],
        &mut |style| {
            intents.op = Some(style_op(lane.id.to_string(), style));
        },
    );
}

fn presets(
    ui: &mut Ui,
    title: &str,
    scopes: &[&[splice_model::Effect]],
    pick: &mut dyn FnMut(Style),
) {
    let current = viewer_captions::preset(scopes);
    section(ui, title, &mut |ui| {
        let idle = Look::NONE.border(LINE, 1.0);
        let tile = States::of(idle)
            .hover(idle.border(STRONG_LINE, 1.0))
            .focus(idle.border(MUTED, 1.0))
            .checked(idle.fill(SELECTED).border(ACCENT, 1.0));
        for styles in STYLES.chunks_exact(3) {
            ui.row(
                Node::row().gap(6.0).padding(Edges::new(0.0, 0.0, 0.0, 6.0)),
                |ui| {
                    for &(style, name) in styles {
                        let result = ui.button_content(
                            name,
                            &Button::DEFAULT
                                .size(Size::Grow, Size::Px(64.0))
                                .name(name)
                                .pad(Edges::all(5.0))
                                .look(&tile)
                                .selected(Some(style) == current),
                            |ui, _| {
                                ui.column(Node::column().gap(4.0).align(Align::Center), |ui| {
                                    preset_sample(ui, style);
                                    let label = ui.label(name, CAPTION);
                                    ink(ui, label, MUTED);
                                });
                            },
                        );
                        if result.clicked {
                            pick(style);
                        }
                    }
                },
            );
        }
    });
}

fn preset_sample(ui: &mut Ui, style: Style) {
    ui.row(
        Node::row()
            .w(Size::Grow)
            .h_px(34.0)
            .align(Align::Center)
            .justify(Justify::Center)
            .gap(2.0)
            .look(
                Look::NONE
                    .fill(Rgba::rgb(59, 52, 70))
                    .gradient(Rgba::rgb(22, 21, 27)),
            ),
        |ui| {
            let font = if style == Style::Typewriter {
                CAPTION
            } else {
                EDITOR_HEADING.size(12.0).weight(800)
            };
            let (first, last) = if style == Style::Clean {
                ("The", "frame")
            } else {
                ("THE", "FRAME")
            };
            let left = ui.label(first, font);
            ink(
                ui,
                left,
                match style {
                    Style::Karaoke => ACTIVE,
                    Style::Pop => Rgba::rgba(255, 255, 255, 0),
                    _ => INK,
                },
            );
            let right = ui.label(
                last,
                font.size(if matches!(style, Style::Bold | Style::Pop) {
                    14.0
                } else {
                    12.0
                }),
            );
            let (fill, text) = match style {
                Style::Bold | Style::Pop => (None, ACTIVE),
                Style::Karaoke => (None, MUTED),
                Style::Box => (Some(ACTIVE), Rgba::rgb(17, 17, 17)),
                Style::Clean => (Some(Rgba::rgb(10, 10, 10)), INK),
                _ => (None, INK),
            };
            let look = fill
                .map_or(Look::NONE, |fill| Look::NONE.fill(fill))
                .text(text);
            ui.style(&right, &States::of(look), pxui::ui::State::default());
        },
    );
}

fn hint(ui: &mut Ui, message: &str) {
    let label = ui.label(message, CAPTION.grow().wrap_glyph());
    ink(ui, label, MUTED);
}

fn control(ui: &mut Ui, title: &'static str, width: f32, enabled: bool) -> bool {
    ui.button(
        title,
        title,
        &Button::sized(width, 22.0)
            .label(CAPTION)
            .look(&CHOICE_LOOK)
            .disabled(!enabled),
    )
    .clicked
}

fn edit_word(id: ItemId, word: WordId, change: Option<(&str, Json)>) -> Json {
    let value = change.map_or(Json::Null, |(key, value)| {
        Json::Object(vec![(key.into(), value)])
    });
    item_setting(
        id,
        "caption.set",
        "words",
        Json::Object(vec![(word.to_string(), value)]),
    )
}

pub(crate) fn split_at(id: ItemId, word: WordId) -> Json {
    item_setting(id, "caption.split", "word", Json::String(word.to_string()))
}

fn style_op(target: String, style: Style) -> Json {
    caption_appearance(
        target,
        "params",
        Json::Object(vec![("preset".into(), Json::String(style.label().into()))]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Event, FrameIn, KeyCode, Role, testing};
    use splice_core::history::{Store, WriteOptions};
    use std::cell::{Cell, RefCell};
    use std::fs;

    #[test]
    fn caption_inspector_edits_words_and_lane_through_core() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/projects/sample.splice");
        let path = std::env::temp_dir().join(format!(
            "splice-caption-inspector-{}.splice",
            std::process::id()
        ));
        fs::create_dir_all(path.join("transcripts")).unwrap();
        for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
            fs::copy(fixture.join(file), path.join(file)).unwrap();
        }
        let store = RefCell::new(Store::open(&path).unwrap());
        let cue: ItemId = "ovl_cap2".parse().unwrap();
        let lane = "lan_cap".parse().unwrap();
        let word: WordId = "w:screen:5".parse().unwrap();
        let selection = Cell::new(Selection::Item(cue));
        let playhead = Cell::new(170);
        let op = Cell::new(None);
        let seek = Cell::new(None);
        let copy = Cell::new(false);
        let mut cache = Cache::default();
        let mut paint = |ui: &mut Ui| {
            let store = store.borrow();
            let project = store.project();
            cache.refresh(ui, Some(project), selection.get());
            let intents = super::super::paint(
                ui,
                Some(project),
                selection.get(),
                super::super::Scope::Selection,
                660,
                (268.0, false),
                (&[], true, None, Some((&cache, playhead.get())), None),
            );
            if let Some(operation) = intents.op {
                assert!(op.replace(Some(operation)).is_none());
            }
            seek.set(seek.get().or(intents.seek));
            copy.set(copy.get() || intents.copy_srt);
        };
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let input = FrameIn::new(268.0, 960.0);
        let settle = |ui: &mut Ui, paint: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..2 {
                ui.frame(&input, paint).unwrap();
            }
        };
        let click = |ui: &mut Ui, role, name, paint: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {name}"));
            let x = if role == Role::Toggle {
                rect.x + rect.w - 20.0
            } else {
                rect.x + 0.5 * rect.w
            };
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                let event = Event::pointer(kind, 0, f64::from(x), f64::from(rect.y + 0.5 * rect.h));
                ui.frame(&input.events(&[event]), paint).unwrap();
            }
        };
        let commit = |action, command| {
            let op = op
                .take()
                .unwrap_or_else(|| panic!("{action} expected {command}"));
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            store
                .borrow_mut()
                .apply(&[op], &WriteOptions::default())
                .unwrap();
        };
        let word_edit = || {
            let store = store.borrow();
            let project = store.project();
            let slot = project.items.find(cue).unwrap();
            let ItemKind::Caption { edits, .. } = &project.items.data(slot).unwrap().kind else {
                panic!("expected caption");
            };
            edits.iter().find(|edit| edit.word == word).cloned()
        };
        settle(&mut ui, &mut paint);
        assert!(testing::find(&ui, Role::Label, "00:05:12 – 00:06:15").is_some());
        click(&mut ui, Role::Button, "frame 🎯", &mut paint);
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Button, "Jump to word", &mut paint);
        assert_eq!(seek.take(), Some(170));
        let replace_field = |ui: &mut Ui, name, new: &str, paint: &mut dyn FnMut(&mut Ui)| {
            click(ui, Role::TextField, name, paint);
            assert!(ui.text_input_focused());
            if new.is_empty() {
                testing::key(ui, KeyCode::Backspace, paint);
            } else {
                let event = Event {
                    kind: Event::TEXT_INPUT,
                    text: pxui::Str {
                        ptr: new.as_ptr(),
                        len: new.len(),
                    },
                    ..Event::default()
                };
                ui.frame(&input.events(&[event]), paint).unwrap();
            }
            testing::key(ui, KeyCode::Enter, paint);
        };
        replace_field(&mut ui, "Word text", "wide shot", &mut paint);
        commit("editing word text", "caption.set");
        settle(&mut ui, &mut paint);
        assert!(testing::find(&ui, Role::Button, "wide shot 🎯").is_some());
        click(&mut ui, Role::Toggle, "Hide word", &mut paint);
        commit("hiding the word", "caption.set");
        let hidden = word_edit().unwrap();
        assert!(hidden.is_hidden());
        assert_eq!(hidden.text.as_deref(), Some("wide shot"));
        assert!(hidden.emph);
        assert_eq!(hidden.emoji.as_deref(), Some("🎯"));
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Toggle, "Hide word", &mut paint);
        commit("showing the word", "caption.set");
        assert!(!word_edit().unwrap().is_hidden());
        settle(&mut ui, &mut paint);
        for emoji in ["👩‍💻", "❤️‍🔥", "🧪🧪🧪🧪"] {
            replace_field(&mut ui, "Word emoji", emoji, &mut paint);
            commit("editing emoji", "caption.set");
            let edit = word_edit().unwrap();
            assert_eq!(edit.emoji.as_deref(), Some(emoji));
            assert_eq!(edit.text.as_deref(), Some("wide shot"));
            assert!(edit.emph && !edit.is_hidden());
            settle(&mut ui, &mut paint);
        }
        mem_sys::set_peer(pxui::library_counter);
        for _ in 0..8 {
            ui.frame(&input, &mut paint).unwrap();
        }
        let meter =
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
        meter.warm(&mut || {
            ui.frame(&input, &mut paint).unwrap();
        });
        replace_field(&mut ui, "Word emoji", "🧪🧪🧪🧪a", &mut paint);
        assert!(op.take().is_none());
        assert_eq!(word_edit().unwrap().emoji.as_deref(), Some("🧪🧪🧪🧪"));
        testing::key(&mut ui, KeyCode::Escape, &mut paint);
        settle(&mut ui, &mut paint);
        replace_field(&mut ui, "Word emoji", "", &mut paint);
        commit("clearing emoji", "caption.set");
        assert!(word_edit().unwrap().emoji.is_none());
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Button, "Pop", &mut paint);
        commit("choosing a cue preset", "effect.set");
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Button, "Split at playhead", &mut paint);
        commit("splitting at the playhead", "caption.split");
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Button, "Merge with next", &mut paint);
        commit("merging adjacent captions", "caption.merge");
        settle(&mut ui, &mut paint);

        for selection_now in [Selection::Item(cue), Selection::Lane(lane)] {
            selection.set(selection_now);
            for _ in 0..8 {
                ui.frame(&input, &mut paint).unwrap();
            }
            meter.warm(&mut || {
                for at in [162, 170, 184, 195] {
                    playhead.set(at);
                    ui.frame(&input, &mut paint).unwrap();
                }
            });
        }
        click(&mut ui, Role::Button, "Copy as SRT", &mut paint);
        assert!(copy.replace(false));
        click(&mut ui, Role::Button, "Clean", &mut paint);
        commit("choosing a lane preset", "effect.set");
        settle(&mut ui, &mut paint);
        click(&mut ui, Role::Button, "2w", &mut paint);
        commit("regenerating the lane", "captions.generate");
        assert!(op.take().is_none());
        let store = store.into_inner();
        let revision = store.project().rev;
        let stamp = store.verify().unwrap();
        store.close().unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.project().rev, revision);
        assert_eq!(reopened.verify().unwrap(), stamp);
        reopened.close().unwrap();
        fs::remove_dir_all(path).unwrap();
    }
}
