use super::commands::{BindingDescription, catalog};
use pxui::{
    Button, Flags, KeyCode, Label, Mods, Node, Part, ReturnKey, SearchField, ThemeHandle, Ui,
    bodies::SearchFieldConfig,
    geom::{Edges, Rgba},
    theme::Font,
    ui::{Along, Axis, Look, Size, State, States},
};
use splice_kit::{Command, CommandId, CommandState};

const SURFACE: Rgba = Rgba::rgb(11, 11, 12);
const PANEL: Rgba = Rgba::rgb(20, 20, 21);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const TAG: Rgba = Rgba::rgb(191, 224, 234);
const SELECTED: Rgba = Rgba::rgb(42, 42, 45);
const BODY: Label = Label::MONO.size(11.0);
const DETAIL: Label = Label::MONO.size(10.0);
const ROW: Look = Look::NONE.text(INK);
const ROW_STATES: States = States::of(ROW)
    .hover(ROW.fill(SELECTED))
    .focus(ROW.fill(SELECTED))
    .checked(ROW.fill(SELECTED))
    .disabled(ROW.text(QUIET));

const SEARCH_HEIGHT: f32 = 36.0;
const ROW_HEIGHT: f32 = 26.0;
const LIST_PADDING: f32 = 4.0;

thread_local! {
    static QUERY_THEME: ThemeHandle = ThemeHandle::parse("search_field.body = palette_query")
        .expect("palette query theme");
}

pub(super) fn install(ui: &mut Ui) -> Result<(), pxui::Error> {
    ui.register_search_field_body("palette_query", query_body)
        .map(|_| ())
}

fn query_body(ui: &mut Ui, config: &SearchFieldConfig) {
    let h = SEARCH_HEIGHT - 1.0;
    ui.set_node_layout(&config.line, &Node::row().w(Size::Grow).h_px(h));
    ui.style(
        &config.line,
        &States::of(Look::NONE.fill(SURFACE)),
        State::default(),
    );
    ui.set_node_layout(&config.well, &Node::row().w(Size::Grow).h_px(h).pad_x(10.0));
    const ENTRY: Look = Look::NONE.text(INK).font(Font::BODY.size(13.0));
    let field = ui.part(
        Part::EDITOR,
        Node::DEFAULT.w(Size::Grow).h_px(h).look(ENTRY),
    );
    if !ui.slot(config.editor) {
        ui.style(&field, &States::of(ENTRY.text(QUIET)), State::default());
    }
}

pub(super) struct Intents {
    pub query: Option<String>,
    pub command: Option<CommandId>,
    pub selected: Option<CommandId>,
    pub open: bool,
}

pub(super) fn paint(
    ui: &mut Ui,
    query: &str,
    (states, bindings): (&[CommandState], &[BindingDescription]),
    selected: Option<CommandId>,
    focus_search: bool,
) -> Intents {
    let mut intents = Intents {
        query: None,
        command: None,
        selected,
        open: true,
    };
    ui.column(
        Node::column().w(Size::Grow).look(Look::NONE.fill(PANEL)),
        |ui| {
            QUERY_THEME.with(|theme| {
                ui.themed(theme, &mut |ui| {
                    let style = SearchField::DEFAULT
                        .name("Search commands")
                        .placeholder("Run a command\u{2026}")
                        .action(ReturnKey::Search);
                    let response = ui.search_field("command-query", query, &style);
                    intents.query = response.changed.map(str::to_owned);
                    let handle = response.response.handle;
                    if focus_search {
                        ui.focus_box(&handle);
                    }
                });
            });
            ui.node(
                "palette-line",
                Node::DEFAULT.h_px(1.0).look(Look::NONE.fill(LINE)),
            );

            let query_changed = intents.query.is_some();
            let query = intents.query.as_deref().unwrap_or(query).trim();
            let catalog = catalog();
            let rows = || {
                catalog.iter().filter_map(move |command| {
                    let state =
                        states
                            .get(command.id.0 as usize)
                            .copied()
                            .unwrap_or(CommandState {
                                enabled: false,
                                ..CommandState::default()
                            });
                    matches(command, state, query).then_some((command, state))
                })
            };
            let count = rows().count();
            let enabled = || rows().enumerate().filter(|(_, (_, state))| state.enabled);
            let first = enabled().next().map(|(index, _)| index);
            let current = if query_changed {
                first
            } else {
                selected
                    .and_then(|id| {
                        enabled()
                            .find(|(_, (command, _))| command.id == id)
                            .map(|(index, _)| index)
                    })
                    .or(first)
            };
            let down = ui.key_pressed(KeyCode::ArrowDown, Mods::NONE);
            let up = ui.key_pressed(KeyCode::ArrowUp, Mods::NONE);
            let search_focused = ui.text_input_focused();
            let mut active = current;
            if let Some(index) = current {
                if down {
                    active = enabled()
                        .find(|(row, _)| *row > index)
                        .map(|(row, _)| row)
                        .or(current);
                } else if up {
                    active = enabled()
                        .take_while(|(row, _)| *row < index)
                        .last()
                        .map(|(row, _)| row)
                        .or(current);
                }
            }

            let viewport = ui.view_size();
            let content_height = count.max(1) as f32 * ROW_HEIGHT + 2.0 * LIST_PADDING;
            let list_height = content_height
                .min(420.0)
                .min(viewport.height * 3.0 / 5.0)
                .min((viewport.height * 4.0 / 5.0 - SEARCH_HEIGHT - 2.0).max(0.0));
            let gutter = if list_height < content_height {
                15.0
            } else {
                0.0
            };
            let modal_width = (viewport.width - 32.0).clamp(180.0, 620.0);
            let row_width = modal_width - 2.0 - 2.0 * LIST_PADDING - gutter;
            let mut reveal = if focus_search || query_changed || active != current {
                Some(active.unwrap_or(0))
            } else {
                None
            };
            let scroll = ui.scroll_view(
                "command-results",
                Node::column()
                    .w(Size::Grow)
                    .h_px(list_height)
                    .flags(Flags::OVER_THUMB)
                    // ScrollView supplies a one-point inset in addition to these edges.
                    .padding(Edges::new(3.0, 3.0, 3.0 + gutter, 3.0)),
                |ui| {
                    if count == 0 {
                        ui.row(Node::DEFAULT.h_px(ROW_HEIGHT).pad_x(8.0).center(), |ui| {
                            ui.label("No command matches.", BODY);
                        });
                    }
                    for (index, (command, state)) in rows().enumerate() {
                        let title = state.title.unwrap_or(command.title);
                        let shortcut = bindings
                            .iter()
                            .find(|binding| binding.id == command.id)
                            .map_or("", |binding| binding.keys.as_str());
                        let button = Button::DEFAULT
                            .size(Size::Grow, Size::Px(ROW_HEIGHT))
                            .axis(Axis::Row)
                            .centered()
                            .content_align(Along::Start)
                            .name(title)
                            .look(&ROW_STATES)
                            .pad(Edges::new(8.0, 0.0, 8.0, 0.0))
                            .gap(12.0)
                            .disabled(!state.enabled)
                            .selected(active == Some(index));
                        let tail = if state.enabled {
                            shortcut
                        } else {
                            unavailable(command)
                        };
                        let row = ui.button_content(command.id.0 as u64, &button, |ui, _| {
                            result_labels(ui, title, command.name, tail, state.enabled, row_width);
                        });
                        if active == Some(index) && active != current && !search_focused {
                            ui.focus_box(&row.handle);
                        }
                        if state.enabled && row.focused && !down && !up {
                            active = Some(index);
                            if selected != Some(command.id) {
                                reveal = Some(index);
                            }
                        }
                        if state.enabled && row.clicked {
                            intents.command = Some(command.id);
                        }
                    }
                },
            );
            if let Some(index) = reveal {
                let top = index as f32 * ROW_HEIGHT;
                let bottom = top + ROW_HEIGHT;
                let offset = ui.scroll(&scroll).settled;
                let visible = (list_height - 2.0 * LIST_PADDING).max(0.0);
                if top < offset {
                    ui.set_scroll_offset(&scroll, top);
                } else if bottom > offset + visible {
                    ui.set_scroll_offset(&scroll, bottom - visible);
                }
            }
            intents.selected = active
                .and_then(|index| rows().nth(index))
                .map(|(command, _)| command.id);
            if ui.key_pressed(KeyCode::Enter, Mods::NONE) && intents.command.is_none() {
                intents.command = intents.selected;
            }
            intents.open =
                intents.command.is_none() && !ui.key_pressed(KeyCode::Escape, Mods::NONE);
        },
    );
    intents
}

fn result_labels(ui: &mut Ui, title: &str, name: &str, tail: &str, enabled: bool, width: f32) {
    let measure = |ui: &mut Ui, label: &str| {
        ui.text_service()
            .measure(label, DETAIL, None)
            .map_or(0.0, |m| m.width.ceil())
    };
    // Preserve a readable title column when the effective chord list or hint is long.
    let detail_budget = (width - 16.0 - 24.0 - 96.0).max(0.0);
    let id_width = measure(ui, name).min(detail_budget);
    let tail_width = measure(ui, tail).min(220.0).min(detail_budget - id_width);
    label(
        ui,
        title,
        BODY.w(Size::Grow).ellipsis(),
        if enabled { INK } else { QUIET },
    );
    label(ui, name, DETAIL.w_px(id_width).ellipsis(), TAG);
    label(ui, tail, DETAIL.w_px(tail_width).ellipsis(), QUIET);
}

fn label(ui: &mut Ui, text: &str, style: Label, ink: Rgba) {
    let node = ui.label(text, style.pad(0.0));
    ui.style(&node, &States::of(Look::NONE.text(ink)), State::default());
}

fn unavailable(command: &Command) -> &'static str {
    match command.name {
        "viewer.commandPalette" => "Already open",
        "history.undo" => "No edit to undo",
        "history.redo" => "No edit to redo",
        "history.undoSession" => "No live agent edits",
        "item.trim" | "item.trimEnd" => "Put the playhead inside a clip",
        "item.split" => "Put the playhead inside an item",
        "item.delete" | "item.deleteLeavingGap" | "item.duplicate" => "Select timeline items",
        "overlay.replaceSpine" => "Select a media overlay",
        _ => "Unavailable here",
    }
}

fn matches(command: &Command, state: CommandState, query: &str) -> bool {
    let fields = [
        state.title.unwrap_or(command.title),
        command.title,
        command.name,
        command.group,
        if command.group == "Transport" {
            "Playback"
        } else {
            ""
        },
    ];
    query
        .split_ascii_whitespace()
        .all(|word| fields.iter().any(|text| contains(text, word)))
}

fn contains(text: &str, query: &str) -> bool {
    query.is_empty()
        || text
            .as_bytes()
            .windows(query.len())
            .any(|window| window.eq_ignore_ascii_case(query.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{self, BACK, CROP, PLAY, REDO};
    use pxui::{Role, UiDesc, testing};

    fn frames(build: &mut dyn FnMut(&mut Ui)) -> Ui {
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        install(&mut ui).unwrap();
        ui.set_a11y(true);
        for _ in 0..3 {
            testing::frame(&mut ui, &[], build);
        }
        ui
    }

    #[test]
    fn palette_filters_and_invokes_the_enabled_command_on_enter() {
        let invoked = std::cell::Cell::new(None);
        let keys = commands::Keys::new(None);
        let mut states = commands::states(true, true, (true, true));
        let play = catalog()
            .iter()
            .position(|command| command.id == PLAY)
            .unwrap();
        states[play].enabled = false;
        let query = std::cell::Cell::new("TransPort");
        let initial = std::cell::Cell::new(true);
        let mut draw = |ui: &mut Ui| {
            let intents = paint(
                ui,
                query.get(),
                (&states, keys.bindings()),
                None,
                initial.replace(false),
            );
            invoked.set(intents.command);
        };
        let mut ui = frames(&mut draw);
        assert!(testing::find(&ui, Role::TextField, "Search commands").is_some());
        assert!(testing::find(&ui, Role::Button, "Pause").is_some());
        assert!(testing::find(&ui, Role::Button, "Undo").is_none());
        testing::key(&mut ui, KeyCode::Tab, &mut draw);
        testing::key(&mut ui, KeyCode::Enter, &mut draw);
        assert_eq!(invoked.get(), Some(BACK));
        query.set("history.REDO");
        testing::frame(&mut ui, &[], &mut draw);
        assert!(testing::find(&ui, Role::Button, "Previous Frame").is_none());
        testing::key(&mut ui, KeyCode::Enter, &mut draw);
        assert_eq!(invoked.get(), Some(REDO));
    }

    #[test]
    fn palette_uses_catalog_commands_and_effective_binding_labels() {
        let states = commands::states(true, false, (true, true));
        let bindings = [BindingDescription {
            id: CROP,
            group: "Edit",
            title: "Crop",
            keys: "F7".into(),
        }];
        let invoked = std::cell::Cell::new(None);
        let initial = std::cell::Cell::new(true);
        let mut draw = |ui: &mut Ui| {
            let result = paint(
                ui,
                "clip.CROP",
                (&states, &bindings),
                None,
                initial.replace(false),
            );
            invoked.set(result.command);
        };
        let mut ui = frames(&mut draw);
        assert!(testing::find(&ui, Role::Button, "Crop").is_some());
        assert!(testing::find(&ui, Role::Label, "F7").is_some());
        testing::key(&mut ui, KeyCode::Enter, &mut draw);
        assert_eq!(invoked.get(), Some(CROP));
    }
    #[test]
    fn palette_matches_measured_dialog_geometry_and_stays_warm() {
        use pxui::FrameIn;

        let mut states = commands::states(true, false, (true, true));
        states[CROP.0 as usize].enabled = false;
        let bindings = [
            BindingDescription {
                id: PLAY,
                group: "Transport",
                title: "Play / Pause",
                keys: "Space · Cmd+Alt+Shift+PageDown · Ctrl+Shift+F12".into(),
            },
            BindingDescription {
                id: CROP,
                group: "Edit",
                title: "Crop",
                keys: "F7".into(),
            },
        ];
        mem_sys::set_peer(pxui::library_counter);
        for (width, height, dialog_width, results_height, row_width) in [
            (960.0, 640.0, 620.0, 384.0, 595.0),
            (800.0, 540.0, 620.0, 324.0, 595.0),
            (480.0, 420.0, 448.0, 252.0, 423.0),
        ] {
            let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
            ui.set_a11y(true);
            let input = FrameIn::new(width, height);
            let mut ordinary = |ui: &mut Ui| {
                ui.search_field(
                    "ordinary",
                    "",
                    &SearchField::DEFAULT.name("Ordinary search"),
                );
            };
            ui.frame(&input, &mut ordinary).unwrap();
            let original = testing::find(&ui, Role::TextField, "Ordinary search").unwrap();
            install(&mut ui).unwrap();
            let first = std::cell::Cell::new(true);
            let mut build = |ui: &mut Ui| {
                let overlay = pxui::Overlay::DEFAULT
                    .w_px(dialog_width)
                    .name("Command palette");
                assert!(
                    ui.modal("palette", &overlay, |ui| {
                        let result =
                            paint(ui, "", (&states, &bindings), None, first.replace(false));
                        assert_eq!(result.selected, Some(PLAY));
                        assert!(result.command.is_none());
                    })
                    .open
                );
            };
            for _ in 0..4 {
                ui.frame(&input, &mut build).unwrap();
            }
            assert!(ui.text_input_focused());
            let dialog = testing::find(&ui, Role::Dialog, "Command palette").unwrap();
            let list = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::ScrollArea))
                .unwrap()
                .rect;
            let field = testing::find(&ui, Role::TextField, "Search commands").unwrap();
            let play = testing::find(&ui, Role::Button, "Play").unwrap();
            let tag = testing::find(&ui, Role::Label, "transport.play").unwrap();
            let chord = testing::find(&ui, Role::Label, &bindings[0].keys).unwrap();
            assert_eq!(dialog.w, dialog_width);
            assert_eq!(dialog.h, SEARCH_HEIGHT + results_height + 2.0);
            assert_eq!(list.h, results_height);
            assert_eq!(list.y, dialog.y + SEARCH_HEIGHT + 1.0);
            assert_eq!(field.x, dialog.x + 11.0);
            assert_eq!(field.h, SEARCH_HEIGHT - 1.0);
            assert_eq!(
                play.w, row_width,
                "dialog {dialog:?}, list {list:?}, field {field:?}, row {play:?}"
            );
            assert_eq!(play.h, ROW_HEIGHT);
            assert_eq!(play.x, list.x + LIST_PADDING);
            assert_eq!(play.y, list.y + LIST_PADDING);
            assert!(
                tag.x + tag.w + 11.0 <= chord.x,
                "{tag:?} overlaps {chord:?}"
            );
            assert!(chord.x + chord.w <= play.x + play.w - 7.0);
            ui.frame(&input, &mut build).unwrap();
            assert!(
                ui.layers()
                    .flat_map(|layer| layer.instances)
                    .any(|instance| {
                        instance.rect == [play.x, play.y, play.x + play.w, play.y + play.h]
                            && instance.color == [42, 42, 45, 255]
                    }),
                "selected fill missing at {play:?}"
            );
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    ui.frame(&input, &mut build).unwrap();
                });
            ui.frame(&input, &mut ordinary).unwrap();
            assert_eq!(
                testing::find(&ui, Role::TextField, "Ordinary search"),
                Some(original)
            );
        }
    }

    #[test]
    fn palette_types_navigates_and_clicks_only_currently_eligible_commands() {
        use pxui::{Event, Str};
        use std::cell::{Cell, RefCell};

        let keys = commands::Keys::new(None);
        let mut initial = commands::states(true, false, (true, true));
        initial[PLAY.0 as usize].enabled = false;
        initial[commands::FORWARD.0 as usize].enabled = false;
        let states = Cell::new(initial);
        let opened = Cell::new(true);
        let query = RefCell::new(String::new());
        let selected = Cell::new(None);
        let invoked = Cell::new(None);
        let mut build = |ui: &mut Ui| {
            let result = paint(
                ui,
                &query.borrow(),
                (&states.get(), keys.bindings()),
                selected.get(),
                opened.replace(false),
            );
            if let Some(next) = result.query {
                *query.borrow_mut() = next;
            }
            selected.set(result.selected);
            invoked.set(result.command);
        };
        let mut ui = frames(&mut build);
        assert!(ui.text_input_focused());
        let input = "playback";
        testing::frame(
            &mut ui,
            &[Event {
                kind: Event::TEXT_INPUT,
                text: Str {
                    ptr: input.as_ptr(),
                    len: input.len(),
                },
                ..Event::default()
            }],
            &mut build,
        );
        assert_eq!(query.borrow().as_str(), "playback");
        assert_eq!(selected.get(), Some(BACK));
        testing::key(&mut ui, KeyCode::ArrowDown, &mut build);
        assert_eq!(selected.get(), Some(commands::START));
        testing::key(&mut ui, KeyCode::ArrowUp, &mut build);
        assert_eq!(selected.get(), Some(BACK));

        let button = testing::find(&ui, Role::Button, "Play").unwrap();
        let x = f64::from(button.x + button.w * 0.5);
        let y = f64::from(button.y + button.h * 0.5);
        testing::click(&mut ui, x, y, &mut build);
        assert_eq!(invoked.get(), None);
        testing::key(&mut ui, KeyCode::Enter, &mut build);
        assert_eq!(invoked.get(), Some(BACK));

        let mut next = states.get();
        next[PLAY.0 as usize].enabled = true;
        states.set(next);
        testing::frame(&mut ui, &[], &mut build);
        testing::click(&mut ui, x, y, &mut build);
        assert_eq!(invoked.get(), Some(PLAY));
    }
}
