use super::playbar;
use pxui::{
    Button, Fill, Icon, IconButton, Label, Node, Paint, Rect, Stroke, ThemeHandle, Ui,
    geom::{Edges, Rgba},
    theme::ColorToken,
    ui::{Align, Along, Axis, Justify, Key, Look, Size, States},
};

pub(super) const PANE_HEIGHT: f32 = 30.0;

const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const STRONG_LINE: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const BG: Rgba = Rgba::rgb(9, 9, 10);
const HOVER: Rgba = Rgba::rgb(34, 34, 36);
const SELECTED: Rgba = Rgba::rgb(42, 42, 45);
const VIDEO: Rgba = Rgba::rgb(236, 230, 206);
const COMMAND: States = States::of(Look::NONE.text(MUTED).border(LINE, 1.0))
    .hover(Look::NONE.text(INK).fill(HOVER).border(STRONG_LINE, 1.0))
    .pressed(Look::NONE.text(INK).fill(SELECTED).border(STRONG_LINE, 1.0));
const TRANSPORT: States = States::of(Look::NONE.text(MUTED))
    .hover(Look::NONE.text(INK).fill(HOVER))
    .pressed(Look::NONE.text(INK).fill(SELECTED))
    .disabled(Look::NONE.text(Rgba::rgb(71, 71, 74)));
const PLAY: States = States::of(Look::NONE.text(BG).fill(INK))
    .hover(Look::NONE.text(BG).fill(VIDEO))
    .pressed(Look::NONE.text(BG).fill(VIDEO))
    .disabled(Look::NONE.text(Rgba::rgb(71, 71, 74)).fill(INK));

const INK_TOKEN: ColorToken = ColorToken(0);
const MUTED_TOKEN: ColorToken = ColorToken(1);
const QUIET_TOKEN: ColorToken = ColorToken(2);
const TEXT: Label = Label::MONO.size(11.0).line_height(15.4);
const META: Label = TEXT.color(QUIET_TOKEN);
const POSITION: Label = TEXT.size(15.0).line_height(21.0).color(INK_TOKEN);
const LABEL_ADVANCE: f32 = 6.6; // IBM Plex Mono at 11 points.
// Top padding drops the contents half a point for PXUI's pixel snapping.
const PANE_PADDING: Edges = Edges::new(8.0, 1.0, 9.0, 0.0);
const TRANSPORT_PADDING: Edges = Edges::new(8.0, 1.0, 8.0, 0.0);

thread_local! {
    static TEXT_THEME: ThemeHandle = {
        let mut theme = ThemeHandle::default();
        theme.set_color(INK_TOKEN, INK);
        theme.set_color(MUTED_TOKEN, MUTED);
        theme.set_color(QUIET_TOKEN, QUIET);
        theme
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PaneAction {
    Media,
    Effects,
    Crop,
    Captions,
    Zoom,
}

#[derive(Default)]
pub(super) struct Triggers([i64; PaneAction::Zoom as usize + 1]);

impl Triggers {
    pub(super) fn x(&self, ui: &Ui, action: PaneAction) -> f32 {
        ui.box_of_key(Key::Resolved(self.0[action as usize]))
            .map_or(4.0, |button| ui.rect(&button).x)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransportAction {
    Start,
    Back,
    Play,
    Forward,
    End,
}

pub(super) fn pane(
    ui: &mut Ui,
    aspect: &str,
    fitted: bool,
    zoom_percent: u32,
    narrow: bool,
) -> (Option<PaneAction>, Triggers) {
    let mut action = None;
    let mut triggers = Triggers::default();
    bar(ui, PANE_HEIGHT, &mut |ui| {
        ui.row(
            Node {
                w: Size::Grow,
                h: Size::Px(PANE_HEIGHT - 1.0),
                align: Align::Center,
                padding: PANE_PADDING,
                gap: 4.0,
                ..Node::DEFAULT
            },
            |ui| {
                for (name, item, gap, icon) in [
                    ("Media", PaneAction::Media, 6.0, 0),
                    ("Effects", PaneAction::Effects, 6.0, 1),
                    ("Crop", PaneAction::Crop, 5.0, 2),
                    ("Captions", PaneAction::Captions, 5.0, 3),
                ] {
                    let width = if narrow {
                        32.0
                    } else {
                        32.0 + gap + name.len() as f32 * LABEL_ADVANCE
                    };
                    let command = Button::sized(width, 22.0)
                        .look(&COMMAND)
                        .label(TEXT)
                        .icon(Icon::draw(pane_icon, icon))
                        .icon_size(14.0)
                        .gap(gap)
                        .name(name);
                    let button = ui.button(name, if narrow { "" } else { name }, &command);
                    triggers.0[item as usize] = button.handle.box_key;
                    if button.clicked {
                        action = Some(item);
                    }
                }
                ui.node(
                    "pane-space",
                    Node {
                        w: Size::Grow,
                        ..Node::DEFAULT
                    },
                );
                ui.row(Node::DEFAULT.w(Size::Fit).pad_x(4.0), |ui| {
                    ui.label("Program", META);
                });
                ui.row(
                    Node {
                        w: Size::Fit,
                        h: Size::Px(18.0),
                        padding: Edges::new(6.0, 0.0, 6.0, 0.0),
                        align: Align::Center,
                        look: Look::NONE.border(STRONG_LINE, 1.0),
                        ..Node::DEFAULT
                    },
                    |ui| {
                        ui.label(aspect, TEXT.size(10.0).line_height(16.0).color(MUTED_TOKEN));
                    },
                );
                ui.node("zoom-space", Node::DEFAULT.w_px(0.0));
                let prefix = if fitted { "Fit · " } else { "" };
                let chars =
                    prefix.chars().count() as u32 + zoom_percent.checked_ilog10().unwrap_or(0) + 2;
                let width = (chars as f32 * LABEL_ADVANCE + 18.0).max(74.0);
                let zoom = Button::sized(width, 22.0)
                    .look(&COMMAND)
                    .axis(Axis::Row)
                    .content_align(Along::Start)
                    .pad(Edges::new(9.0, 0.0, 9.0, 0.0))
                    .name("Viewer zoom");
                let button = ui.button_content("viewer-zoom", &zoom, |ui, state| {
                    let ink = if COMMAND.pick(state).text == INK {
                        INK_TOKEN
                    } else {
                        MUTED_TOKEN
                    };
                    ui.label_digits(format_args!("{prefix}{zoom_percent}%"), TEXT.color(ink));
                });
                triggers.0[PaneAction::Zoom as usize] = button.handle.box_key;
                if button.clicked {
                    action = Some(PaneAction::Zoom);
                }
            },
        );
        rule(ui, "pane-rule");
    });
    (action, triggers)
}

pub(super) fn transport(ui: &mut Ui, view: playbar::View, narrow: bool) -> Option<TransportAction> {
    let mut action = None;
    bar(ui, 34.0, &mut |ui| {
        rule(ui, "transport-rule");
        ui.row(
            Node {
                w: Size::Grow,
                h: Size::Px(33.0),
                align: Align::Center,
                padding: TRANSPORT_PADDING,
                gap: 8.0,
                ..Node::DEFAULT
            },
            |ui| {
                let side = Node {
                    align: Align::Center,
                    w: Size::Grow,
                    ..Node::DEFAULT
                };
                ui.row(Node { gap: 6.0, ..side }, |ui| {
                    let [min, sec, frame] = playbar::clock(view.at, view.fps);
                    let at =
                        ui.label_digits(format_args!("{min:02}:{sec:02}:{frame:02}"), POSITION);
                    ui.a11y_name(&at, "Current timecode");

                    // PXUI uses the bottom edge as the baseline for digits labels.
                    ui.row(
                        Node::DEFAULT
                            .w(Size::Fit)
                            .center()
                            .gap(6.0)
                            .padding(Edges::new(0.0, 4.0, 0.0, 0.0)),
                        |ui| {
                            ui.label("/", META);
                            let [min, sec, frame] = playbar::clock(view.end, view.fps);
                            let end =
                                ui.label_digits(format_args!("{min:02}:{sec:02}:{frame:02}"), META);
                            ui.a11y_name(&end, "Sequence duration");
                        },
                    );
                });
                ui.row(
                    Node {
                        w: Size::Fit,
                        align: Align::Center,
                        gap: 1.0,
                        ..Node::DEFAULT
                    },
                    |ui| {
                        let playing = view.play.title == Some("Pause");
                        for (name, item, icon) in [
                            ("Go to beginning", TransportAction::Start, 0),
                            ("Previous frame", TransportAction::Back, 1),
                            (
                                if playing { "Pause" } else { "Play" },
                                TransportAction::Play,
                                if playing { 3 } else { 2 },
                            ),
                            ("Next frame", TransportAction::Forward, 4),
                            ("Go to end", TransportAction::End, 5),
                        ] {
                            let main = item == TransportAction::Play;
                            if main {
                                ui.node("play-before", Node::DEFAULT.w_px(3.0));
                            }
                            let button = if main {
                                IconButton::sized(34.0, 24.0).look(&PLAY)
                            } else {
                                IconButton::sized(26.0, 26.0).look(&TRANSPORT)
                            };
                            let button = button.disabled(
                                view.end <= 0 || view.dragging || (main && !view.play.enabled),
                            );
                            if ui
                                .icon_button(
                                    item as u64,
                                    Icon::draw(transport_icon, icon),
                                    name,
                                    &button,
                                )
                                .clicked
                            {
                                action = Some(item);
                            }
                            if main {
                                ui.node("play-after", Node::DEFAULT.w_px(3.0));
                            }
                        }
                    },
                );
                if !narrow {
                    ui.row(
                        Node {
                            justify: Justify::End,
                            ..side
                        },
                        |ui| {
                            if matches!(view.rate, 0 | 1) {
                                ui.label_fmt(format_args!("{} fps", view.fps), META);
                            } else {
                                let direction = if view.rate < 0 { "◀ " } else { "" };
                                ui.label_fmt(
                                    format_args!(
                                        "{direction}{}× · {} fps",
                                        view.rate.unsigned_abs(),
                                        view.fps
                                    ),
                                    META,
                                );
                            }
                        },
                    );
                }
            },
        );
    });
    action
}

fn bar(ui: &mut Ui, height: f32, build: &mut dyn FnMut(&mut Ui)) {
    TEXT_THEME.with(|theme| {
        ui.themed(theme, &mut |ui| {
            ui.column(
                Node::DEFAULT
                    .w(Size::Grow)
                    .h_px(height)
                    .look(Look::NONE.fill(PANEL)),
                &mut *build,
            );
        });
    });
}

fn rule(ui: &mut Ui, key: &'static str) {
    ui.node(
        key,
        Node {
            w: Size::Grow,
            h: Size::Px(1.0),
            look: Look::NONE.fill(LINE),
            ..Node::DEFAULT
        },
    );
}

fn transport_icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, word: u32) {
    let side = rect.w.min(rect.h).min(15.0);
    let s = side / 24.0;
    let x = rect.x + (rect.w - side) / 2.0;
    let y = rect.y + (rect.h - side) / 2.0;
    let point = |a: f32, b: f32| (x + a * s, y + b * s);
    let fill = Fill::new(ink);
    match word {
        0 | 1 | 4 | 5 => {
            let (base, tip) = match word {
                0 => (18.5, 9.5),
                1 => (14.0, 5.5),
                4 => (10.0, 18.5),
                _ => (5.5, 14.5),
            };
            paint.triangle(point(base, 6.5), point(base, 17.5), point(tip, 12.0), fill);
            if matches!(word, 0 | 5) {
                let stop = if word == 0 { 5.5 } else { 18.5 };
                paint.line(
                    point(stop, 6.0),
                    point(stop, 18.0),
                    Stroke::new(ink).width(1.6 * s),
                );
            }
        }
        2 => paint.triangle(point(8.0, 5.5), point(19.0, 12.0), point(8.0, 18.5), fill),
        _ => {
            for center in [8.0, 16.0] {
                paint.fill(
                    Rect::new(x + (center - 0.8) * s, y + 5.5 * s, 1.6 * s, 13.0 * s),
                    fill.fractional(),
                );
            }
        }
    }
}

fn pane_icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, word: u32) {
    let side = rect.w.min(rect.h).min(14.0);
    let s = side / 24.0;
    let x = rect.x + (rect.w - side) / 2.0;
    let y = rect.y + (rect.h - side) / 2.0;
    let stroke = Stroke::new(ink).width(1.6 * s);
    let point = |a: (f32, f32)| (x + a.0 * s, y + a.1 * s);
    let line = |paint: &mut Paint<'_>, a: (f32, f32), b: (f32, f32)| {
        paint.line(point(a), point(b), stroke);
    };
    match word {
        0 => {
            paint.stroke(
                Rect::new(x + 3.0 * s, y + 7.0 * s, 14.0 * s, 12.0 * s),
                stroke,
            );
            line(paint, (7.0, 4.0), (19.5, 4.0));
            paint.curve(
                point((19.5, 4.0)),
                point((21.0, 4.0)),
                point((21.0, 5.5)),
                stroke,
            );
            line(paint, (21.0, 5.5), (21.0, 15.0));
            line(paint, (8.5, 10.5), (13.0, 13.0));
            line(paint, (13.0, 13.0), (8.5, 15.5));
            line(paint, (8.5, 15.5), (8.5, 10.5));
        }
        1 => {
            line(paint, (4.0, 20.0), (14.5, 9.5));
            line(paint, (12.5, 7.5), (16.5, 11.5));
            for (cx, cy, radius) in [(18.0, 4.75, 1.75), (7.5, 4.75, 1.25), (19.0, 14.5, 1.5)] {
                line(paint, (cx - radius, cy), (cx + radius, cy));
                line(paint, (cx, cy - radius), (cx, cy + radius));
            }
        }
        2 => {
            line(paint, (7.0, 3.0), (7.0, 17.0));
            line(paint, (7.0, 17.0), (21.0, 17.0));
            line(paint, (3.0, 7.0), (17.0, 7.0));
            line(paint, (17.0, 7.0), (17.0, 21.0));
        }
        _ => {
            paint.stroke(
                Rect::new(x + 3.0 * s, y + 5.0 * s, 18.0 * s, 14.0 * s),
                stroke,
            );
            for right in [10.6, 17.0] {
                let left = right - 3.7;
                paint.curve(
                    point((right, 10.2)),
                    point((left, 8.2)),
                    point((left, 12.0)),
                    stroke,
                );
                paint.curve(
                    point((left, 12.0)),
                    point((left, 15.8)),
                    point((right, 13.8)),
                    stroke,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{FontData, FrameIn, Role, UiDesc, draw::kind, testing};
    use splice_kit::CommandState;
    use std::cell::Cell;

    #[test]
    fn transport_start_and_end_emit_accessible_actions() {
        let action = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            if let Some(picked) = transport(
                ui,
                playbar::View {
                    at: 192,
                    end: 660,
                    fps: 30,
                    play: CommandState::default(),
                    rate: 0,
                    dragging: false,
                },
                false,
            ) {
                action.set(Some(picked));
            }
        };
        let mut ui = pxui::doc::frames(1, &mut draw);
        let play = testing::find(&ui, Role::Button, "Play").unwrap();
        assert!((play.x + play.w * 0.5 - 160.0).abs() <= 1.0, "{play:?}");
        for (name, expected) in [
            ("Go to beginning", TransportAction::Start),
            ("Go to end", TransportAction::End),
        ] {
            let button = testing::find(&ui, Role::Button, name).unwrap();
            testing::click(
                &mut ui,
                f64::from(button.x + button.w / 2.0),
                f64::from(button.y + button.h / 2.0),
                &mut draw,
            );
            assert_eq!(action.take(), Some(expected));
        }
        assert_eq!(playbar::clock(192, 30), [0, 6, 12]);
        assert_eq!(playbar::clock(3602, 60), [1, 0, 2]);
    }

    #[test]
    fn desktop_chrome_matches_reference_positions_and_readout_colors() {
        let fonts = [FontData(include_bytes!(
            "../assets/fonts/IBMPlexMono-Regular.ttf"
        ))];
        let mut ui = Ui::new(&UiDesc::DEFAULT.fonts(&fonts)).unwrap();
        ui.set_a11y(true);
        ui.frame(&FrameIn::new(960.0, 640.0), &mut |ui| {
            ui.column(Node::DEFAULT.w_px(692.0), |ui| {
                ui.node("title", Node::DEFAULT.h_px(34.0));
                pane(ui, "9:16", true, 8, false);
                ui.node("stage", Node::DEFAULT.h_px(177.0));
                transport(
                    ui,
                    playbar::View {
                        at: 192,
                        end: 660,
                        fps: 30,
                        play: CommandState::default(),
                        rate: 0,
                        dragging: false,
                    },
                    false,
                );
            });
        })
        .unwrap();
        assert_eq!(
            testing::find(&ui, Role::Button, "Viewer zoom"),
            Some(Rect::new(609.0, 38.0, 74.0, 22.0)),
        );
        assert_eq!(
            testing::find(&ui, Role::Button, "Play"),
            Some(Rect::new(329.0, 247.0, 34.0, 24.0)),
        );
        for (name, value) in [
            ("Current timecode", "00:06:12"),
            ("Sequence duration", "00:22:00"),
        ] {
            let label = ui
                .a11y_nodes()
                .iter()
                .find(|label| ui.a11y_name_of(label) == name)
                .unwrap();
            assert_eq!(ui.a11y_value_of(label), Some(value));
        }
        let layer = ui.layer(0).unwrap();
        for (left, right, color) in [
            (8.0, 80.0, [244, 243, 238, 255]),
            (99.0, 152.0, [130, 130, 125, 255]),
        ] {
            assert!(layer.instances.iter().any(|i| {
                i.kind == kind::MASK
                    && i.rect[1] >= 254.0
                    && (left..right).contains(&i.rect[0])
                    && i.color == color
            }));
        }
    }

    #[test]
    fn changing_clock_allocates_nothing_after_warm_up() {
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = Ui::new(&UiDesc::DEFAULT).unwrap();
        let mut tick = || {
            for at in 180..200 {
                testing::frame(&mut ui, &[], &mut |ui| {
                    pane(ui, "9:16", true, (at - 179) as u32, true);
                    transport(
                        ui,
                        playbar::View {
                            at,
                            end: 660,
                            fps: 30,
                            play: CommandState::default(),
                            rate: 0,
                            dragging: false,
                        },
                        false,
                    );
                });
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut tick);
    }
}
