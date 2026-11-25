use super::commands::{SHORTCUT_GROUPS, ShortcutRow};
use pxui::{
    Button, Label, Node, Overlay, TextAlign, Ui,
    geom::{Edges, Rgba},
    ui::{Align, Look, Role, Size, State, States},
};

const TITLE: &str = "Keyboard shortcuts";
const HEADER: f32 = 43.0;
const PAD: f32 = 12.0;
const COLUMN_GAP: f32 = 20.0;
const MIN_COLUMN: f32 = 200.0;
const ROW_LINE: f32 = 15.4;
const TAG_GAP: f32 = 6.6;

const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const STRONG_LINE: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const TAG_INK: Rgba = Rgba::rgb(191, 224, 234);
const CLOSE_BORDER: Look = Look::NONE.fill(CONTROL).border(LINE, 1.0).text(INK);
const CLOSE_LOOK: States = States::of(CLOSE_BORDER).hover(CLOSE_BORDER.border(STRONG_LINE, 1.0));

const BODY: Label = Label::BODY.size(11.0).line_height(ROW_LINE);
const CHORD: Label = Label::MONO.size(11.0).line_height(ROW_LINE);
const TAG: Label = Label::MONO.size(10.0);

pub(super) fn paint(ui: &mut Ui, shortcuts: &[ShortcutRow], [width, height]: [f32; 2]) -> bool {
    let width = (width - 32.0).clamp(0.0, 720.0);
    let height = (height * 0.8).max(0.0);
    // PXUI adds its 1pt modal stroke around the body, plus our header rule.
    let body = (height - 3.0).max(0.0);
    let head = HEADER.min(body);
    let grid_height = body - head;
    let pad = PAD.min(width * 0.1);
    let inner = width - 2.0 * pad;
    let close_width = 56.0_f32.min(inner);
    // The modal's stroke reserves one point on each side in the reference grid.
    let grid_inner = (inner - 2.0).max(0.0);
    let columns = (((grid_inner + COLUMN_GAP) / (MIN_COLUMN + COLUMN_GAP)) as usize).clamp(1, 3);
    let column_width = (grid_inner - (columns - 1) as f32 * COLUMN_GAP) / columns as f32;
    let grid_side = (pad - 1.0).max(0.0);

    let mut groups = SHORTCUT_GROUPS;
    let mut count = 0;
    for (group, name) in SHORTCUT_GROUPS {
        if shortcuts.iter().any(|row| row.group == group) {
            groups[count] = (group, name);
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
                    let grid = Node::column()
                        .w(Size::Grow)
                        .padding(Edges::new(grid_side, pad, grid_side, pad))
                        .gap(PAD);
                    ui.scroll_view(
                        "effective-bindings",
                        Node::DEFAULT.w(Size::Grow).max_h(grid_height),
                        |ui| {
                            ui.column(grid, |ui| {
                                for band in groups[..count].chunks(columns) {
                                    ui.row(
                                        Node::row()
                                            .w(Size::Grow)
                                            .gap(COLUMN_GAP)
                                            .align(Align::Start),
                                        |ui| {
                                            for &(group, name) in band {
                                                ui.id(group as u64, |ui| {
                                                    ui.column(
                                                        Node::column().w_px(column_width),
                                                        |ui| {
                                                            text(
                                                                ui,
                                                                name,
                                                                TAG.h_px(17.0).role(Role::Heading),
                                                                QUIET,
                                                            );
                                                            for (index, shortcut) in
                                                                shortcuts.iter().enumerate().filter(
                                                                    |(_, row)| row.group == group,
                                                                )
                                                            {
                                                                ui.id(index as u64, |ui| {
                                                                    row(ui, shortcut, column_width)
                                                                });
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

fn row(ui: &mut Ui, shortcut: &ShortcutRow, width: f32) {
    let chord_width = measure(ui, &shortcut.keys, CHORD).ceil().min(width * 0.45);
    let gap = 8.0_f32.min(width * 0.1);
    let left = (width - chord_width - gap).max(0.0);

    ui.row(
        Node::row()
            .w_px(width)
            .gap(gap)
            .pad_y(3.0)
            .align(Align::Start),
        |ui| {
            let label = ui.column(Node::column().w_px(left).role(Role::Label), |ui| {
                title_and_tag(ui, shortcut.title, shortcut.command, left);
            });
            ui.a11y_name(&label, shortcut.title);
            text(
                ui,
                &shortcut.keys,
                CHORD.w_px(chord_width).wrap_glyph().align(TextAlign::End),
                MUTED,
            );
        },
    );
    line(ui, "shortcut-row-rule");
}

fn title_and_tag(ui: &mut Ui, title: &str, tag: &str, width: f32) {
    let body = BODY.role(Role::Unknown);
    let tag_style = TAG.line_height(ROW_LINE);
    let mut start = 0;
    let mut end = 0;
    // PXUI spans share a font size. Keep the smaller command tag in its own label,
    // packing title words into measured lines and using any space left on the final line.
    for word in title.split_inclusive(' ') {
        let boundary = end;
        end += word.len();
        if boundary > start && measure(ui, title[start..end].trim_end(), body) > width {
            text(
                ui,
                title[start..boundary].trim_end(),
                body.w_px(width).wrap_glyph(),
                INK,
            );
            start = boundary;
        }
    }
    let tail = title[start..].trim_end();
    let title_width = measure(ui, tail, body);
    let tag_width = measure(ui, tag, tag_style);
    if title_width + TAG_GAP + tag_width <= width {
        ui.row(Node::row().w_px(width).gap(TAG_GAP), |ui| {
            text(ui, tail, body.w_px(title_width), INK);
            text(ui, tag, tag_style.w_px(tag_width), TAG_INK);
        });
    } else {
        text(ui, tail, body.w_px(width).wrap_glyph(), INK);
        text(ui, tag, tag_style.w_px(width).wrap_glyph(), TAG_INK);
    }
}

fn measure(ui: &mut Ui, label: &str, style: Label) -> f32 {
    ui.text_service()
        .width(label, style.pad(0.0), None)
        .unwrap_or(f32::INFINITY)
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
    use crate::commands;
    use input::Key;
    use pxui::{Event, FontData, FrameIn, Rect, UiDesc, testing};

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
    fn shortcut_rows_keep_words_and_flow_tags_at_authored_cadence() {
        let keys = commands::Keys::new(None);
        assert_eq!(keys.shortcuts().len(), 34);
        mem_sys::set_peer(pxui::library_counter);
        for (width, dialog_x, dialog_width, edit_x) in [
            (960.0, 120.0, 720.0, 609.0),
            (800.0, 40.0, 720.0, 529.0),
            (480.0, 16.0, 448.0, 29.0),
        ] {
            let mut ui = headless();
            let input = FrameIn::new(width, 640.0);
            let mut draw = |ui: &mut Ui| assert!(!paint(ui, keys.shortcuts(), [width, 640.0]));
            for _ in 0..4 {
                ui.frame(&input, &mut draw).unwrap();
            }
            let dialog = bounds(&ui, Role::Dialog, TITLE);
            assert_eq!(dialog, Rect::new(dialog_x, 64.0, dialog_width, 512.0));
            for row in keys.shortcuts() {
                bounds(&ui, Role::Label, row.title);
            }
            let title = |name| bounds(&ui, Role::Label, name);
            let tag = |owner: Rect, command| {
                ui.a11y_nodes()
                    .iter()
                    .find(|node| {
                        node.is_role(Role::Label)
                            && ui.a11y_name_of(node) == command
                            && node.rect.x >= owner.x
                            && node.rect.x + node.rect.w <= owner.x + owner.w + 0.01
                            && node.rect.y >= owner.y
                            && node.rect.y + node.rect.h <= owner.y + owner.h + 0.01
                    })
                    .unwrap_or_else(|| panic!("missing {command} in {owner:?}"))
                    .rect
            };
            let play = title("Play / pause");
            let reverse = title("Reverse · stop · forward (repeat for 2× 4×)");
            let step = title("Step frame / second");
            let trim = title("Trim start / end to playhead");
            let split = title("Split at playhead (a caption: caption.split)");
            let compact = width == 480.0;
            let step_tag = tag(step, "ui.show");
            let trim_tag = tag(trim, "item.trim");
            let reverse_tag = tag(reverse, "ui.play");
            println!(
                "SHORTCUTS {width} dialog={dialog:?} play={} reverse={} step={} step tag={:?} trim={} trim tag={:?} split={}",
                play.y, reverse.y, step.y, step_tag, trim.y, trim_tag, split.y
            );
            assert!((play.x - dialog_x - 13.0).abs() < 0.15);
            assert!((play.y - 142.0).abs() < 0.15);
            assert!((split.x - edit_x).abs() < 0.15);
            assert!((reverse.y - play.y - 22.4).abs() < 0.15);
            assert!((step.y - reverse.y - if compact { 53.2 } else { 37.8 }).abs() < 0.15);
            assert!((trim.y - split.y - if compact { 53.2 } else { 37.8 }).abs() < 0.15);
            assert!((step_tag.y - step.y - ROW_LINE).abs() < 0.15);
            assert!((trim_tag.y - trim.y - ROW_LINE).abs() < 0.15);
            if compact {
                let second = measure(&mut ui, "second", BODY);
                let playhead = measure(&mut ui, "playhead", BODY);
                assert!((step_tag.x - step.x - second - TAG_GAP).abs() < 0.15);
                assert!((trim_tag.x - trim.x - playhead - TAG_GAP).abs() < 0.15);
                assert!((reverse_tag.x - reverse.x).abs() < 0.15);
            } else {
                assert!(reverse_tag.x > reverse.x);
                assert!((reverse_tag.y - reverse.y - ROW_LINE).abs() < 0.15);
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    ui.frame(&input, &mut draw).unwrap();
                });
        }
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
             F5 = item.split\nBackspace = history.undo\n",
        )
        .unwrap();
        let keys = commands::Keys::new(Some(&path));
        std::fs::remove_file(path).unwrap();
        let shortcuts = keys.shortcuts();
        let play = shortcuts
            .iter()
            .find(|row| row.title == "Play / pause")
            .unwrap();
        assert!(play.keys.contains("PageDown") && play.keys.starts_with("Space"));
        mem_sys::set_peer(pxui::library_counter);
        for (width, sheet_width, columns, column_width) in [
            (960.0, 720.0, 3, 218.0),
            (800.0, 720.0, 3, 218.0),
            (480.0, 448.0, 2, 201.0),
            (400.0, 368.0, 1, 342.0),
        ] {
            let height = 940.0;
            let mut ui = headless();
            let input = FrameIn::new(width, height);
            let mut build = |ui: &mut Ui| assert!(!paint(ui, shortcuts, [width, height]));
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
            for (_, group) in SHORTCUT_GROUPS {
                assert_eq!(ui.a11y_nodes().iter().filter(|node|
                    node.is_role(Role::Heading) && ui.a11y_name_of(node) == group).count(), 1);
            }
            let title = bounds(&ui, Role::Label, play.title);
            let chord = bounds(&ui, Role::Label, &play.keys);
            let tag = bounds(&ui, Role::Label, "ui.play");
            assert!(
                title.x + title.w <= chord.x && tag.x + tag.w <= chord.x,
                "title {title:?}, tag {tag:?}, chord {chord:?}"
            );
            assert!((chord.x + chord.w - first.x - column_width).abs() < 0.15);
            assert!(chord.h > 14.0, "{chord:?}");
            bounds(&ui, Role::Label, "C · F5");
            bounds(&ui, Role::Label, "⌦ · ⇧⌫");
            bounds(&ui, Role::Label, "⌘Z · ⌫ ⇧⌘Z");
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    ui.frame(&input, &mut build).unwrap();
                });
        }
    }

    #[test]
    fn shortcut_dialog_closes_from_button_keyboard_and_escape() {
        let keys = commands::Keys::new(None);
        for (width, height) in [
            (960.0, 640.0),
            (800.0, 640.0),
            (480.0, 640.0),
            (320.0, 240.0),
        ] {
            for action in [None, Some(Key::Enter), Some(Key::Escape)] {
                let mut closed = false;
                let mut build =
                    |ui: &mut Ui| closed |= paint(ui, keys.shortcuts(), [width, height]);
                let mut frame = |ui: &mut Ui, events: &[Event]| {
                    ui.frame(&FrameIn::new(width, height).events(events), &mut build)
                        .unwrap();
                };
                let mut ui = headless();
                for _ in 0..3 {
                    frame(&mut ui, &[]);
                }
                let scroll = ui
                    .a11y_nodes()
                    .iter()
                    .find(|node| node.is_role(Role::ScrollArea))
                    .unwrap();
                assert!(scroll.rect.h <= height * 0.8 - HEADER);
                match action {
                    Some(key) => frame(
                        &mut ui,
                        &[
                            Event::key(Event::KEY_PRESSED, key.code()),
                            Event::key(Event::KEY_RELEASED, key.code()),
                        ],
                    ),
                    None => {
                        let button = bounds(&ui, Role::Button, "Close");
                        let x = f64::from(button.x + button.w * 0.5);
                        let y = f64::from(button.y + button.h * 0.5);
                        frame(&mut ui, &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]);
                        frame(&mut ui, &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)]);
                    }
                }
                assert!(closed, "{width} {action:?}");
            }
        }
    }
}
