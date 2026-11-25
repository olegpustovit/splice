use super::commands::{self, BindingDescription};
use pxui::{
    Button, Label, Node, Overlay, TextAlign, Ui,
    geom::Rgba,
    ui::{Align, Look, Role, Size, State, States},
};

const TITLE: &str = "Keyboard shortcuts";
const GROUPS: [&str; 6] = ["PLAYBACK", "TOOLS", "EDIT", "LANES", "ADD", "VIEW"];
const HEADER: f32 = 46.0;
const PAD: f32 = 12.0;
const COLUMN_GAP: f32 = 20.0;
const MIN_COLUMN: f32 = 200.0;

const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const STRONG_LINE: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const TAG_INK: Rgba = Rgba::rgb(191, 224, 234);
const CLOSE_BORDER: Look = Look::NONE.fill(CONTROL).border(LINE, 1.0).text(INK);
const CLOSE_LOOK: States = States::of(CLOSE_BORDER).hover(CLOSE_BORDER.border(STRONG_LINE, 1.0));

const BODY: Label = Label::BODY.size(11.0);
const CHORD: Label = Label::MONO.size(11.0);
const TAG: Label = Label::MONO.size(10.0);

pub(super) fn paint(
    ui: &mut Ui,
    bindings: &[BindingDescription],
    [width, height]: [f32; 2],
) -> bool {
    let width = (width - 32.0).clamp(0.0, 720.0);
    let height = (height * 0.8).max(0.0);
    // PXUI adds its 1pt modal stroke around the body, plus our header rule.
    let body = (height - 3.0).max(0.0);
    let head = HEADER.min(body);
    let grid_height = body - head;
    let pad = PAD.min(width * 0.1);
    let inner = width - 2.0 * pad;
    let close_width = 56.0_f32.min(inner);
    let columns = (((inner + COLUMN_GAP) / (MIN_COLUMN + COLUMN_GAP)) as usize).clamp(1, 3);
    let column_width = (inner - (columns - 1) as f32 * COLUMN_GAP) / columns as f32;

    let mut groups = [0; GROUPS.len()];
    let mut count = 0;
    for group in 0..GROUPS.len() {
        if bindings
            .iter()
            .any(|row| !row.keys.is_empty() && group_of(row) == group)
        {
            groups[count] = group;
            count += 1;
        }
    }

    let mut closed = false;
    let overlay = Overlay::DEFAULT.w_px(width).name(TITLE);
    let modal = ui.modal("keyboard-shortcuts", &overlay, |ui| {
        ui.column(
            Node::column()
                .w(Size::Grow)
                .look(Look::NONE.fill(CONTROL).border(STRONG_LINE, 1.0)),
            |ui| {
                ui.row(
                    Node::row()
                        .w(Size::Grow)
                        .h_px(head)
                        .pad_x(pad)
                        .gap(8.0)
                        .center(),
                    |ui| {
                        text(
                            ui,
                            TITLE,
                            Label::BODY
                                .font(1)
                                .weight(300)
                                .size(20.0)
                                .w_px((inner - close_width - 8.0).max(0.0))
                                .ellipsis()
                                .role(Role::Heading),
                            INK,
                        );
                        let close = ui.button(
                            "close-shortcuts",
                            "Close",
                            &Button::sized(close_width, 26.0_f32.min(head)).look(&CLOSE_LOOK),
                        );
                        ui.focus_default(&close.handle);
                        closed |= close.clicked;
                    },
                );
                line(ui, "shortcut-heading-rule");
                if grid_height > 0.0 && count > 0 {
                    ui.scroll_view(
                        "effective-bindings",
                        Node::DEFAULT.w(Size::Grow).max_h(grid_height),
                        |ui| {
                            ui.column(Node::column().w(Size::Grow).pad(pad).gap(PAD), |ui| {
                                for band in groups[..count].chunks(columns) {
                                    ui.row(
                                        Node::row()
                                            .w(Size::Grow)
                                            .gap(COLUMN_GAP)
                                            .align(Align::Start),
                                        |ui| {
                                            for &group in band {
                                                ui.id(group as u64, |ui| {
                                                    ui.column(
                                                        Node::column().w_px(column_width),
                                                        |ui| {
                                                            text(
                                                                ui,
                                                                GROUPS[group],
                                                                TAG.h_px(18.0).role(Role::Heading),
                                                                QUIET,
                                                            );
                                                            for binding in
                                                                bindings.iter().filter(|row| {
                                                                    !row.keys.is_empty()
                                                                        && group_of(row) == group
                                                                })
                                                            {
                                                                ui.id(
                                                                    u64::from(binding.id.0),
                                                                    |ui| {
                                                                        row(
                                                                            ui,
                                                                            binding,
                                                                            column_width,
                                                                        )
                                                                    },
                                                                );
                                                            }
                                                        },
                                                    );
                                                });
                                            }
                                        },
                                    );
                                }
                            });
                        },
                    );
                }
            },
        );
    });
    closed || !modal.open
}

fn group_of(binding: &BindingDescription) -> usize {
    match binding.group {
        "Transport" => 0,
        "Tools" => 1,
        "Editor" if binding.id == commands::PALETTE => 1,
        "Edit" | "History" => 2,
        "Lanes" => 3,
        "Add" => 4,
        _ => 5,
    }
}

fn route(binding: &BindingDescription) -> &'static str {
    let Some(command) = commands::catalog().get(binding.id.0 as usize) else {
        return "";
    };
    match command.name {
        "transport.stepBack"
        | "transport.step"
        | "transport.stepBackSecond"
        | "transport.stepSecond"
        | "transport.start"
        | "transport.end"
        | "transport.previousEdit"
        | "transport.nextEdit"
        | "view.zoomIn"
        | "view.zoomOut"
        | "timeline.zoomIn"
        | "timeline.zoomOut"
        | "timeline.zoomSelection"
        | "timeline.selectLaneItems"
        | "timeline.escape"
        | "view.fit" => "ui.show",
        "timeline.nudgeBack"
        | "timeline.nudgeForward"
        | "timeline.nudgeBackTen"
        | "timeline.nudgeForwardTen" => "item.move",
        "item.laneUp" | "item.laneDown" => "item.lane",
        "item.trimEnd" => "item.trim",
        "item.deleteLeavingGap" => "item.delete",
        "ui.cut" | "ui.copy" | "ui.paste" => "ui.clipboard",
        "history.undoSession" => "history.undo",
        name => name,
    }
}

fn row(ui: &mut Ui, binding: &BindingDescription, width: f32) {
    let measure = |ui: &mut Ui, label: &str, style: Label| {
        ui.text_service()
            .measure(label, style, None)
            .map_or(width, |m| m.width.ceil())
    };
    let chord_width = measure(ui, &binding.keys, CHORD).min(width * 0.45);
    let gap = 8.0_f32.min(width * 0.1);
    let left = (width - chord_width - gap).max(0.0);
    let title_width = measure(ui, binding.title, BODY);
    let route = route(binding);
    let tag_width = measure(ui, route, TAG);

    ui.row(
        Node::row()
            .w_px(width)
            .gap(gap)
            .pad_y(3.0)
            .align(Align::Start),
        |ui| {
            if !route.is_empty() && title_width + 4.0 + tag_width <= left {
                ui.row(Node::row().w_px(left).gap(4.0).align(Align::Center), |ui| {
                    text(ui, binding.title, BODY.w_px(title_width), INK);
                    text(ui, route, TAG.w_px(tag_width), TAG_INK);
                });
            } else {
                ui.column(Node::column().w_px(left), |ui| {
                    text(ui, binding.title, BODY.w_px(left).wrap_glyph(), INK);
                    if !route.is_empty() {
                        text(ui, route, TAG.w_px(left).wrap_glyph(), TAG_INK);
                    }
                });
            }
            text(
                ui,
                &binding.keys,
                CHORD.w_px(chord_width).wrap_glyph().align(TextAlign::End),
                MUTED,
            );
        },
    );
    line(ui, "shortcut-row-rule");
}

fn line(ui: &mut Ui, key: &'static str) {
    ui.node(
        key,
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(1.0)
            .look(Look::NONE.fill(LINE)),
    );
}

fn text(ui: &mut Ui, value: &str, style: Label, color: Rgba) {
    let label = ui.label(value, style.pad(0.0));
    ui.style(
        &label,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Event, FontData, FrameIn, KeyCode, Rect, UiDesc, testing};

    fn headless() -> Ui {
        let fonts = [
            FontData(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
            FontData(include_bytes!(
                "../assets/fonts/BarlowSemiCondensed-Light.ttf"
            )),
        ];
        let mut ui = Ui::new(&UiDesc::DEFAULT.fonts(&fonts)).unwrap();
        ui.set_a11y(true);
        ui
    }

    fn bounds(ui: &Ui, role: Role, name: &str) -> Rect {
        testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {role:?} {name}"))
    }

    #[test]
    fn shortcut_grid_fits_and_borrows_effective_custom_labels() {
        let path =
            std::env::temp_dir().join(format!("splice-shortcut-sheet-{}.keys", std::process::id()));
        std::fs::write(
            &path,
            "Ctrl+Alt+Shift+Cmd+PageDown = transport.play\n\
             Ctrl+Alt+Shift+Cmd+PageUp = transport.play\n\
             Ctrl+Alt+Shift+Cmd+F11 = transport.play\n\
             Ctrl+Alt+Shift+Cmd+F12 = transport.play\n\
             Backspace = history.undo\n",
        )
        .unwrap();
        let keys = commands::Keys::new(Some(&path));
        std::fs::remove_file(path).unwrap();
        let bindings = keys.bindings();
        let play = bindings
            .iter()
            .find(|row| row.id == commands::PLAY)
            .unwrap();
        assert!(play.keys.contains("PageDown") && play.keys.starts_with("Space"));
        assert!(
            bindings
                .iter()
                .find(|row| row.id == commands::DELETE)
                .unwrap()
                .keys
                .len()
                < 5
        );
        assert!(
            bindings
                .iter()
                .find(|row| row.id == commands::UNDO)
                .unwrap()
                .keys
                .len()
                > 5
        );
        mem_sys::set_peer(pxui::library_counter);
        for (width, sheet_width, columns) in [
            (960.0, 720.0, 3),
            (800.0, 720.0, 3),
            (480.0, 448.0, 2),
            (400.0, 368.0, 1),
        ] {
            let height = 940.0;
            let mut ui = headless();
            let input = FrameIn::new(width, height);
            let mut build = |ui: &mut Ui| assert!(!paint(ui, bindings, [width, height]));
            for _ in 0..4 {
                ui.frame(&input, &mut build).unwrap();
            }
            let dialog = bounds(&ui, Role::Dialog, TITLE);
            assert!((dialog.w - sheet_width).abs() < 1.0, "{dialog:?}");
            assert!(dialog.h <= height * 0.8 + 1.0, "{dialog:?}");
            let header = bounds(&ui, Role::Heading, TITLE);
            let close = bounds(&ui, Role::Button, "Close");
            assert!(close.y < dialog.y + HEADER && header.x + header.w <= close.x);
            let margin = dialog.x + dialog.w - (close.x + close.w);
            assert!((margin - PAD).abs() <= 2.0, "{dialog:?} {close:?}");
            let first = bounds(&ui, Role::Heading, "PLAYBACK");
            let tools = bounds(&ui, Role::Heading, "TOOLS");
            let edit = bounds(&ui, Role::Heading, "EDIT");
            if columns == 1 {
                assert_eq!(tools.x, first.x);
                assert!(tools.y > first.y && edit.y > tools.y);
            } else {
                assert!(tools.x > first.x && (tools.y - first.y).abs() < 1.0);
                if columns == 2 {
                    assert_eq!(edit.x, first.x);
                    assert!(edit.y > tools.y);
                } else {
                    assert!(edit.x > tools.x && (edit.y - tools.y).abs() < 1.0);
                }
            }
            for group in GROUPS {
                assert_eq!(ui.a11y_nodes().iter().filter(|node|
                    node.is_role(Role::Heading) && ui.a11y_name_of(node) == group).count(), 1);
            }
            let title = bounds(&ui, Role::Label, play.title);
            let chord = bounds(&ui, Role::Label, &play.keys);
            let tag = bounds(&ui, Role::Label, "transport.play");
            assert!(
                title.x + title.w <= chord.x && tag.x + tag.w <= chord.x,
                "title {title:?}, tag {tag:?}, chord {chord:?}"
            );
            assert!(chord.x + chord.w <= dialog.x + dialog.w - 10.0);
            assert!(chord.h > 14.0, "{chord:?}");
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    ui.frame(&input, &mut build).unwrap();
                });
        }
    }

    #[test]
    fn shortcut_dialog_closes_from_button_keyboard_and_escape() {
        let keys = commands::Keys::new(None);
        for action in [None, Some(KeyCode::Enter), Some(KeyCode::Escape)] {
            let mut closed = false;
            let build = &mut |ui: &mut Ui| closed |= paint(ui, keys.bindings(), [320.0, 240.0]);
            let mut ui = headless();
            for _ in 0..3 {
                testing::frame(&mut ui, &[], build);
            }
            let scroll = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::ScrollArea))
                .unwrap();
            assert!(scroll.rect.h <= 240.0 * 0.8 - HEADER);
            match action {
                Some(key) => testing::key(&mut ui, key, build),
                None => {
                    let button = bounds(&ui, Role::Button, "Close");
                    let x = f64::from(button.x + button.w * 0.5);
                    let y = f64::from(button.y + button.h * 0.5);
                    testing::frame(
                        &mut ui,
                        &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
                        build,
                    );
                    testing::frame(
                        &mut ui,
                        &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)],
                        build,
                    );
                }
            }
            assert!(closed, "{action:?}");
        }
    }
}
