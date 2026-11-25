use super::editor_session;
use pxui::{
    Button, Fill, Icon, IconButton, Label, Node, Paint, Rect, Stroke, Ui,
    geom::Rgba,
    ui::{Justify, Look, Size, States},
};

const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const STRONG: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const TABS: [(&str, View, f32, u32); 3] = [
    ("Editor", View::Editor, 87.2, 0),
    ("Agent · MCP", View::Agent, 123.2, 1),
    ("Spec", View::Spec, 72.8, 2),
];
const TOOL_WIDTH: f32 = 26.0;
const ROW_HEIGHT: f32 = 33.0;
const COMPACT_TAB_TEXT_HEIGHT: f32 = 16.8;
const ROW_GAP: f32 = 2.0;
const WRAPPED_COMPACT_PAD: f32 = 3.0;
const TAB_INSET: f32 = 12.0;
const NAME_INSET: f32 = 13.0;
const NAME: Label = super::EDITOR_HEADING.size(16.0).h_px(19.0);

const TAB: States = States::of(Look::NONE.text(QUIET))
    .hover(Look::NONE.text(INK))
    .pressed(Look::NONE.text(INK))
    .checked(Look::NONE.text(INK));
const TOOL: States = States::of(Look::NONE.text(MUTED))
    .hover(Look::NONE.text(INK).fill(Rgba::rgb(34, 34, 36)))
    .pressed(Look::NONE.text(INK).fill(Rgba::rgb(42, 42, 45)));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum View {
    #[default]
    Editor,
    Agent,
    Spec,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    View(View),
    CommandPalette,
    Shortcuts,
    Session(editor_session::Action),
}

pub(super) struct Layout {
    compact: bool,
    icons_only: bool,
    wrapped: bool,
    pad: f32,
    name_max: f32,
    status_width: f32,
    revision_width: f32,
}

impl Layout {
    pub(super) fn new(
        width: f32,
        revision: Option<u64>,
        session: Option<&editor_session::State>,
    ) -> Self {
        let compact = width <= 820.0;
        let icons_only = width <= 480.0;
        let pad = if icons_only { 8.0 } else { 12.0 };
        let controls_gap = if icons_only { 2.0 } else { 8.0 };
        let available = width - 2.0 * pad;
        let brand = if icons_only { 20.0 } else { 70.0 };
        let mut left = brand + pad;
        for (_, _, full_width, _) in TABS {
            left += tab_width(full_width, compact, icons_only);
        }
        if !compact {
            left += TAB_INSET;
        }
        let revision_width = revision.filter(|_| !compact).map_or(0.0, |rev| {
            let glyphs = rev.checked_ilog10().unwrap_or(0) + 2;
            glyphs as f32 * editor_session::LABEL_ADVANCE
        });
        let (banner, status) = session.map_or((0.0, 0.0), |state| state.banner_widths(!compact));
        let fixed_right = 2.0 * TOOL_WIDTH
            + controls_gap
            + if revision_width > 0.0 {
                revision_width + controls_gap
            } else {
                0.0
            }
            + if banner > 0.0 {
                banner + controls_gap
            } else {
                0.0
            };
        let status_width = status.min((available - fixed_right).max(0.0));
        let right = fixed_right + status_width;
        let wrapped = left + pad + right > available;
        let name_max = if compact {
            0.0
        } else {
            let left_available = if wrapped {
                available
            } else {
                available - pad - right
            };
            (left_available - left - pad - NAME_INSET).clamp(0.0, 200.0 - NAME_INSET)
        };
        Self {
            compact,
            icons_only,
            wrapped,
            pad,
            name_max,
            status_width,
            revision_width,
        }
    }

    fn row_height(&self) -> f32 {
        if self.wrapped && self.compact {
            20.0
        } else {
            ROW_HEIGHT
        }
    }

    fn vertical_pad(&self) -> f32 {
        if self.wrapped && self.compact {
            WRAPPED_COMPACT_PAD
        } else {
            0.0
        }
    }

    pub(super) fn height(&self) -> f32 {
        let second = if self.wrapped {
            ROW_GAP + TOOL_WIDTH
        } else {
            0.0
        };
        self.row_height() + second + 2.0 * self.vertical_pad() + 1.0
    }
}

fn tab_width(full_width: f32, compact: bool, icons_only: bool) -> f32 {
    if icons_only {
        28.0
    } else if compact {
        full_width - 8.0
    } else {
        full_width
    }
}

pub(super) fn paint(
    ui: &mut Ui,
    name: Option<&str>,
    revision: Option<u64>,
    view: View,
    layout: &Layout,
    session: Option<&editor_session::State>,
) -> Option<Action> {
    let mut action = None;
    let row_height = layout.row_height();
    let vertical_pad = layout.vertical_pad();
    let hit_height = row_height - 1.0;
    let icon_size = if layout.icons_only { 16.0 } else { 14.0 };
    let mut tab_label = Label::MONO.size(12.0);
    let underline_y = if layout.compact {
        tab_label = tab_label.line_height(COMPACT_TAB_TEXT_HEIGHT);
        let content_height = if layout.icons_only {
            icon_size
        } else {
            COMPACT_TAB_TEXT_HEIGHT
        };
        (hit_height + content_height) * 0.5
    } else {
        hit_height
    };
    let tab_icon = if layout.compact && !layout.icons_only {
        compact_labeled_icon
    } else {
        icon
    };
    let bar = Node::DEFAULT.w(Size::Grow).look(Look::NONE.fill(PANEL));
    let doc = name.filter(|_| layout.name_max >= 18.0).map(|name| {
        let measured = ui
            .text_service()
            .width(name, NAME, None)
            .unwrap_or(layout.name_max);
        (name, measured.min(layout.name_max))
    });
    ui.column(bar.h_px(layout.height()), |ui| {
        if vertical_pad > 0.0 {
            ui.node("titlebar-top-pad", Node::DEFAULT.h_px(vertical_pad));
        }
        ui.row(
            bar.h_px(row_height)
                .center()
                .pad_x(layout.pad)
                .justify(Justify::SpaceBetween),
            |ui| {
                ui.row(Node::row().w(Size::Fit).center().gap(layout.pad), |ui| {
                    ui.row(Node::row().w(Size::Fit).center().gap(8.0), |ui| {
                        let logo = ui.node("splice-mark", Node::DEFAULT.w_px(20.0).h_px(20.0));
                        ui.canvas(&logo, |paint, rect| {
                            let fill = Fill::new(INK);
                            let x = rect.x + 1.67;
                            let y = rect.y + 1.67;
                            let size = 16.67;
                            let r = 4.58;
                            paint.fill(Rect::new(x + r, y, size - 2.0 * r, size), fill);
                            paint.fill(Rect::new(x, y + r, size, size - 2.0 * r), fill);
                            for dx in [r, size - r] {
                                for dy in [r, size - r] {
                                    paint.circle((x + dx, y + dy), r, Some(fill), None);
                                }
                            }
                            let stroke = Stroke::new(PANEL).width(1.75);
                            let p = |x: f32, y: f32| {
                                (rect.x + x * (5.0 / 6.0), rect.y + y * (5.0 / 6.0))
                            };
                            let pulse = [
                                (6.0, 12.5),
                                (8.6, 12.5),
                                (10.3, 7.5),
                                (13.7, 17.5),
                                (15.4, 12.5),
                                (18.0, 12.5),
                            ];
                            for pair in pulse.windows(2) {
                                paint.line(
                                    p(pair[0].0, pair[0].1),
                                    p(pair[1].0, pair[1].1),
                                    stroke,
                                );
                            }
                        });
                        if !layout.icons_only {
                            ui.label(
                                "Splice",
                                super::EDITOR_HEADING
                                    .size(18.0)
                                    .h_px(18.0)
                                    .w(Size::Px(42.0)),
                            );
                        }
                    });
                    if let Some((name, width)) = doc {
                        ui.row(
                            Node::row().w_px(width + NAME_INSET).center().gap(12.0),
                            |ui| {
                                ui.node(
                                    "document-divider",
                                    Node::DEFAULT
                                        .w_px(1.0)
                                        .h_px(19.0)
                                        .look(Look::NONE.fill(STRONG)),
                                );
                                ui.label(name, NAME.w_px(width).ellipsis());
                            },
                        );
                    }
                    ui.row(
                        Node::row()
                            .w(Size::Fit)
                            .h_px(row_height)
                            .padding(pxui::geom::Edges::new(
                                if layout.compact { 0.0 } else { TAB_INSET },
                                0.0,
                                0.0,
                                0.0,
                            )),
                        |ui| {
                            for (name, target, full_width, word) in TABS {
                                let active = view == target;
                                let width =
                                    tab_width(full_width, layout.compact, layout.icons_only);
                                ui.stack(Node::DEFAULT.w_px(width).h_px(row_height), |ui| {
                                    let tab = Button::sized(width, hit_height)
                                        .look(&TAB)
                                        .selected(active)
                                        .name(name)
                                        .label(tab_label)
                                        .icon(Icon::draw(
                                            tab_icon,
                                            word + if active { 8 } else { 0 },
                                        ))
                                        .icon_size(icon_size)
                                        .gap(6.0);
                                    if ui
                                        .button(
                                            name,
                                            if layout.icons_only { "" } else { name },
                                            &tab,
                                        )
                                        .clicked
                                    {
                                        action = Some(Action::View(target));
                                    }
                                    if active {
                                        ui.node(
                                            "tab-underline",
                                            Node::DEFAULT
                                                .at(0.0, underline_y)
                                                .w_px(width)
                                                .h_px(1.0)
                                                .look(Look::NONE.fill(ACCENT)),
                                        );
                                    }
                                });
                            }
                        },
                    );
                });
                if !layout.wrapped {
                    action = controls(ui, layout, revision, session).or(action);
                }
            },
        );
        if layout.wrapped {
            ui.node("titlebar-row-gap", Node::DEFAULT.h_px(ROW_GAP));
            ui.row(
                bar.h_px(TOOL_WIDTH)
                    .center()
                    .pad_x(layout.pad)
                    .justify(Justify::End),
                |ui| action = controls(ui, layout, revision, session).or(action),
            );
        }
        if vertical_pad > 0.0 {
            ui.node("titlebar-bottom-pad", Node::DEFAULT.h_px(vertical_pad));
        }
        ui.node(
            "titlebar-divider",
            Node::DEFAULT
                .w(Size::Grow)
                .h_px(1.0)
                .look(Look::NONE.fill(LINE)),
        );
    });
    action
}

fn controls(
    ui: &mut Ui,
    layout: &Layout,
    revision: Option<u64>,
    session: Option<&editor_session::State>,
) -> Option<Action> {
    let mut action = None;
    ui.row(
        Node::row()
            .w(Size::Fit)
            .center()
            .gap(if layout.icons_only { 2.0 } else { 8.0 }),
        |ui| {
            if let Some(session) = session
                && let Some(next) = editor_session::banner(ui, session, layout.status_width)
            {
                action = Some(Action::Session(next));
            }
            if let Some(revision) = revision.filter(|_| layout.revision_width > 0.0) {
                let label = ui.label_fmt(
                    format_args!("r{revision}"),
                    Label::MONO
                        .size(11.0)
                        .line_height(15.4)
                        .w_px(layout.revision_width),
                );
                ui.style(
                    &label,
                    &States::of(Look::NONE.text(QUIET)),
                    pxui::ui::State::default(),
                );
            }
            let tool = IconButton::sized(TOOL_WIDTH, TOOL_WIDTH).look(&TOOL);
            for (name, item, word) in [
                ("Command palette", Action::CommandPalette, 3),
                ("Keyboard shortcuts", Action::Shortcuts, 4),
            ] {
                if ui
                    .icon_button(name, Icon::draw(icon, word), name, &tool)
                    .clicked
                {
                    action = Some(item);
                }
            }
        },
    );
    action
}

fn compact_labeled_icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, word: u32) {
    // PXUI supplies a 20-point box to the icon drawing callback.
    let side = 14.0;
    let x = rect.x + (rect.w - side) * 0.5;
    let y = rect.y + (rect.h - side) * 0.5;
    icon(paint, Rect::new(x, y, side, side), ink, word);
}

fn icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, word: u32) {
    let side = rect
        .w
        .min(rect.h)
        .min(if word & 7 >= 3 { 15.0 } else { 16.0 });
    let s = side / 24.0;
    let x = rect.x + (rect.w - side) / 2.0;
    let y = rect.y + (rect.h - side) / 2.0;
    let ink = if word & 8 != 0 { ACCENT } else { ink };
    let stroke = Stroke::new(ink).width(1.6 * s);
    let point = |(a, b): (f32, f32)| (x + a * s, y + b * s);
    let path = |paint: &mut Paint<'_>, points: &[(f32, f32)]| {
        for pair in points.windows(2) {
            paint.line(point(pair[0]), point(pair[1]), stroke);
        }
    };
    let curve = |paint: &mut Paint<'_>, points: [(f32, f32); 4]| {
        paint.cubic(points.map(point), stroke);
    };
    match word & 7 {
        0 => {
            paint.frame(
                Rect::new(x + 3.0 * s, y + 3.5 * s, 18.0 * s, 10.5 * s),
                stroke,
            );
            path(paint, &[(3.0, 17.5), (11.0, 17.5)]);
            path(paint, &[(14.0, 17.5), (21.0, 17.5)]);
            path(paint, &[(3.0, 20.5), (15.0, 20.5)]);
        }
        1 => {
            curve(paint, [(11.0, 3.5), (11.5, 7.6), (13.3, 9.4), (17.5, 10.0)]);
            curve(
                paint,
                [(17.5, 10.0), (13.3, 10.6), (11.5, 12.4), (11.0, 16.5)],
            );
            curve(
                paint,
                [(11.0, 16.5), (10.5, 12.4), (8.7, 10.6), (4.5, 10.0)],
            );
            curve(paint, [(4.5, 10.0), (8.7, 9.4), (10.5, 7.6), (11.0, 3.5)]);
            path(paint, &[(18.5, 15.5), (18.5, 20.5)]);
            path(paint, &[(16.0, 18.0), (21.0, 18.0)]);
        }
        2 => {
            path(
                paint,
                &[
                    (6.0, 3.0),
                    (15.0, 3.0),
                    (19.0, 7.0),
                    (19.0, 21.0),
                    (6.0, 21.0),
                    (6.0, 3.0),
                ],
            );
            path(paint, &[(15.0, 3.0), (15.0, 7.0), (19.0, 7.0)]);
            for (y, end) in [(11.0, 16.0), (14.5, 16.0), (18.0, 13.0)] {
                path(paint, &[(9.0, y), (end, y)]);
            }
        }
        3 => {
            path(paint, &[(9.0, 6.0), (9.0, 18.0)]);
            path(paint, &[(15.0, 6.0), (15.0, 18.0)]);
            path(paint, &[(6.0, 9.0), (18.0, 9.0)]);
            path(paint, &[(6.0, 15.0), (18.0, 15.0)]);
            for (cx, cy, sx, sy) in [
                (6.0, 6.0, 1.0, 1.0),
                (18.0, 6.0, -1.0, 1.0),
                (6.0, 18.0, 1.0, -1.0),
                (18.0, 18.0, -1.0, -1.0),
            ] {
                curve(
                    paint,
                    [
                        (cx + sx * 3.0, cy),
                        (cx + sx * 3.0, cy - sy * 6.5),
                        (cx - sx * 6.5, cy - sy * 2.3),
                        (cx - sx * 2.12, cy + sy * 2.12),
                    ],
                );
                curve(
                    paint,
                    [
                        (cx - sx * 2.12, cy + sy * 2.12),
                        (cx - sx * 1.56, cy + sy * 2.69),
                        (cx - sx * 0.79, cy + sy * 3.0),
                        (cx, cy + sy * 3.0),
                    ],
                );
            }
        }
        _ => {
            paint.frame(
                Rect::new(x + 3.0 * s, y + 6.0 * s, 18.0 * s, 12.0 * s),
                stroke,
            );
            for x in [7.0, 10.3, 13.7, 17.0] {
                paint.circle(point((x, 10.0)), 0.8 * s, Some(Fill::new(ink)), None);
            }
            path(paint, &[(8.0, 14.0), (16.0, 14.0)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Role, testing};
    use std::cell::Cell;

    #[test]
    fn titlebar_controls_have_accessible_intents() {
        let picked = Cell::new(None);
        let mut frame = |ui: &mut Ui| {
            picked.set(paint(
                ui,
                Some("Field Notes / 01"),
                Some(1),
                View::Editor,
                &Layout::new(320.0, Some(1), None),
                None,
            ));
        };
        let mut ui = pxui::doc::frames(1, &mut frame);
        for (label, expected) in [
            ("Agent · MCP", Action::View(View::Agent)),
            ("Command palette", Action::CommandPalette),
            ("Keyboard shortcuts", Action::Shortcuts),
        ] {
            let rect = testing::find(&ui, Role::Button, label).unwrap();
            testing::click(
                &mut ui,
                f64::from(rect.x + rect.w * 0.5),
                f64::from(rect.y + rect.h * 0.5),
                &mut frame,
            );
            assert_eq!(picked.get(), Some(expected));
        }
    }

    #[test]
    fn compact_tabs_put_the_active_rule_below_the_icon_and_keep_the_hit_area() {
        let mut ui = super::super::viewer_ui().unwrap();
        ui.set_a11y(true);
        let layout = Layout::new(480.0, None, None);
        let input = pxui::FrameIn::new(480.0, 640.0);
        for _ in 0..2 {
            ui.frame(&input, &mut |ui| {
                paint(ui, None, None, View::Editor, &layout, None);
            })
            .unwrap();
        }

        let hit = testing::find(&ui, Role::Button, "Editor").unwrap();
        assert_eq!((hit.x, hit.y, hit.w, hit.h), (36.0, 0.0, 28.0, 32.0));
        assert!(ui.layers().flat_map(|layer| layer.instances).any(|shape| {
            shape.kind == pxui::draw::kind::RECT
                && shape.color == [ACCENT.r, ACCENT.g, ACCENT.b, 255]
                && shape.rect == [36.0, 24.0, 64.0, 25.0]
        }));
        assert_eq!(layout.height(), 34.0);
    }

    #[test]
    fn warmed_titlebar_paint_allocates_nothing() {
        mem_sys::set_peer(pxui::library_counter);
        let mut ui = super::super::viewer_ui().unwrap();
        let input = pxui::FrameIn::new(960.0, 640.0);
        let mut tick = || {
            ui.frame(&input, &mut |ui| {
                paint(
                    ui,
                    Some("Field Notes / 01"),
                    Some(1),
                    View::Editor,
                    &Layout::new(960.0, Some(1), None),
                    None,
                );
            })
            .unwrap();
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut tick);
    }
}
