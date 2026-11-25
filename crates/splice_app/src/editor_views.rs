use super::{commands::BindingDescription, editor_titlebar::View};
use pxui::{
    Button, Label, Node, Pressable, Role, Stroke, TextSpan, Ui,
    draw::flags::rect::{BOTTOM, LEFT},
    geom::{Edges, Rgba},
    ui::{Align, Along, Bar, Flags, Look, Size, State, States},
};
use splice_core::commands::COMMANDS;
use splice_mcp::tool_catalog;
use splice_model::Project;

mod engineering;
mod plan;
mod registry;
pub(super) use registry::Parity;

const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const BG: Rgba = Rgba::rgb(9, 9, 10);
const SURFACE: Rgba = Rgba::rgb(11, 11, 12);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const SAGE: Rgba = Rgba::rgb(211, 228, 205);
const MOTION: Rgba = Rgba::rgb(191, 224, 234);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const NAV_LOOK: States = States::of(Look::NONE.text(MUTED))
    .hover(Look::NONE.fill(Rgba::rgb(34, 34, 36)).text(INK))
    .checked(Look::NONE.fill(Rgba::rgb(42, 42, 45)).text(INK));
const SPEC_NAV: States = NAV_LOOK
    .checked(
        Look::NONE
            .fill(Rgba::rgb(42, 42, 45))
            .text(INK)
            .border(ACCENT, 2.0)
            .sides(LEFT),
    )
    .focus(Look::NONE.text(INK).border(INK, 1.0));
const NAV_TEXT: Label = Label::MONO.size(11.0).line_height(15.4);
const SECTION_CAPTION: Label = Label::MONO
    .size(10.0)
    .line_height(14.0)
    .caps(true)
    .tracking(0.06)
    .w(Size::Grow)
    .wrap();
const DOC_BODY: Label = Label::MONO
    .size(12.0)
    .line_height(19.44)
    .w(Size::Grow)
    .wrap();
const DOC_LEDE: Label = DOC_BODY.size(13.0).line_height(21.06);
const TEXT: Label = Label::MONO.size(12.0).w(Size::Grow).wrap();
const CAPTION: Label = Label::MONO.size(10.0).w(Size::Grow).wrap();
const NAME: Label = Label::BODY.size(18.0).weight(300).w(Size::Grow).wrap();
const HEADING: Label = Label::BODY
    .font(1)
    .size(28.0)
    .line_height(39.2)
    .role(Role::Heading)
    .weight(300)
    .w(Size::Grow)
    .wrap();
const SUBHEADING: Label = Label::BODY
    .font(1)
    .size(19.0)
    .line_height(26.6)
    .role(Role::Heading)
    .weight(300)
    .w(Size::Grow)
    .wrap();
const TOOLS_STARTS: &[(usize, &str)] = &[
    (0, "Perception"),
    (9, "Commands"),
    (10, "Session and history"),
    (14, "Editor"),
    (15, "Jobs"),
    (18, "Recording"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Selection {
    tool: usize,
    section: usize,
    scroll: f32,
    restoring: bool,
    parity_requested: bool,
    parity_details: bool,
    external_link: Option<&'static str>,
}

impl Selection {
    pub(super) fn restore_scroll(&mut self) -> bool {
        self.restoring = self.scroll > 0.0;
        self.restoring
    }

    pub(super) fn take_parity_request(&mut self) -> bool {
        std::mem::take(&mut self.parity_requested)
    }

    pub(super) fn resolve(mut self) -> Self {
        if let Some(url) = self.external_link.take() {
            plan::open(url);
        }
        self
    }
}

#[cfg(test)]
fn paint(
    ui: &mut Ui,
    project: Option<&Project>,
    view: View,
    selection: Selection,
    bindings: &[BindingDescription],
) -> Option<Selection> {
    paint_with_parity(ui, project, view, selection, bindings, &Parity::default())
}

pub(super) fn paint_with_parity(
    ui: &mut Ui,
    project: Option<&Project>,
    view: View,
    selection: Selection,
    bindings: &[BindingDescription],
    parity: &Parity,
) -> Option<Selection> {
    match view {
        View::Editor => None,
        View::Agent => {
            agent(ui, project, selection.tool).map(|tool| Selection { tool, ..selection })
        }
        View::Spec => spec_with_parity(ui, selection, (bindings, parity)),
    }
}

fn agent(ui: &mut Ui, project: Option<&Project>, selected: usize) -> Option<usize> {
    let tools = tool_catalog();
    let tool = tools.get(selected).or_else(|| tools.first())?;
    let mut picked = None;
    ui.row(Node::DEFAULT.w(Size::Grow).h(Size::Grow), |ui| {
        ui.column(panel(180.0), |ui| {
            pane_heading(ui, &mut |ui| {
                ui.label_fmt(format_args!("{} MCP tools", tools.len()), CAPTION.size(11.0));
            });
            ui.scroll_view("tools", Node::DEFAULT.w(Size::Grow).h(Size::Grow), |ui| {
                ui.column(Node::DEFAULT.w(Size::Grow).pad(4.0), |ui| {
                    for (index, row) in tools.iter().enumerate() {
                        if let Some((_, group)) = TOOLS_STARTS.iter().find(|(start, _)| *start == index) {
                            group_title(ui, group);
                        }
                        if nav(ui, index, row.name, selected == index, 30.0) {
                            picked = Some(index);
                        }
                    }
                });
            });
        });
        vertical_rule(ui, "agent-rail-divider");
        ui.column(Node::DEFAULT.w(Size::Grow).h(Size::Grow).look(Look::NONE.fill(BG)), |ui| {
            pane_heading(ui, &mut |ui| {
                ui.label(tool.name, NAME);
                if let Some(project) = project {
                    ui.label_fmt(format_args!("r{}", project.rev), Label::MONO.size(11.0));
                }
            });
            ui.scroll_view("tool-detail", Node::DEFAULT.w(Size::Grow).h(Size::Grow), |ui| {
                ui.column(Node::DEFAULT.w(Size::Grow).pad(12.0).gap(8.0), |ui| {
                    ui.label(tool.description, TEXT);
                    ui.label("Request · tools/call arguments (JSON)", CAPTION);
                    ui.column(
                        Node::DEFAULT.w(Size::Grow).pad(8.0).gap(4.0)
                            .look(Look::NONE.fill(SURFACE).border(LINE, 1.0)),
                        |ui| {
                            if tool.input.is_empty() {
                                ui.label("{}", TEXT);
                            }
                            for &(field, kind) in tool.input {
                                let suffix = if tool.required.contains(&field) { " · required" } else { "" };
                                ui.label_fmt(format_args!("{field}{suffix}"), TEXT);
                                ui.label(if kind.starts_with('{') { "JSON value" } else { kind }, CAPTION);
                            }
                        },
                    );
                    ui.label("Result fields", CAPTION);
                    for &(field, _) in tool.output {
                        ui.label(field, TEXT);
                    }
                    ui.label("Session activity", NAME);
                    ui.label("Inspect committed activity with history.list. Use history.undo to revert a batch or a session.", TEXT);
                });
            });
        });
        if let Some(project) = project {
            vertical_rule(ui, "agent-project-divider");
            ui.column(panel(180.0), |ui| {
                pane_heading(ui, &mut |ui| {
                    ui.label("Current project", CAPTION.size(11.0));
                });
                ui.column(Node::DEFAULT.w(Size::Grow).pad(8.0).gap(4.0), |ui| {
                    ui.label_fmt(format_args!("Revision r{}", project.rev), TEXT);
                    ui.label_fmt(format_args!("{} · {} fps", project.aspect.label(), project.fps), TEXT);
                    ui.label_fmt(format_args!("{} clips", project.clips.order().len()), TEXT);
                    ui.label_fmt(format_args!("{} overlays", project.items.order().len()), TEXT);
                    ui.label_fmt(format_args!("{} markers", project.markers.len()), TEXT);
                });
            });
        }
    });
    picked
}

#[cfg(test)]
fn spec(ui: &mut Ui, selection: Selection, bindings: &[BindingDescription]) -> Option<Selection> {
    spec_with_parity(ui, selection, (bindings, &Parity::default()))
}

fn spec_with_parity(
    ui: &mut Ui,
    selection: Selection,
    context: (&[BindingDescription], &Parity),
) -> Option<Selection> {
    let selected = selection.section;
    let mut next = selection;
    let narrow = ui.view_size().width <= 820.0;
    let mut picked = None;
    let mut position = None;
    let frame = Node::DEFAULT.w(Size::Grow).h(Size::Grow);
    if narrow {
        ui.column(frame, |ui| {
            picked = spec_nav(ui, selected, true);
            row_rule(ui);
            position = spec_document(ui, narrow, context, &mut next, &mut picked);
        });
    } else {
        ui.row(frame, |ui| {
            picked = spec_nav(ui, selected, false);
            vertical_rule(ui, "spec-divider");
            position = spec_document(ui, narrow, context, &mut next, &mut picked);
        });
    }
    let (active, scroll) = position.unwrap_or((None, selection.scroll));
    next.section = picked.or(active).unwrap_or(selected);
    next.scroll = scroll;
    (next != selection).then_some(next)
}

fn spec_nav(ui: &mut Ui, selected: usize, narrow: bool) -> Option<usize> {
    let scroll = if narrow {
        Node::column()
            .w(Size::Grow)
            .h_px(31.4)
            .flags(Flags::SCROLL_X | Flags::CLIP | Flags::BLOCKING)
            .look(Look::NONE.fill(PANEL))
    } else {
        panel(219.0)
            .flags(Flags::SCROLL | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
            .bar(Bar::new(Rgba::rgb(56, 56, 59), 6.0).gap(2.0))
    };
    let scroll = ui.node("spec-nav", scroll.role(Role::ScrollArea));
    ui.a11y_name(&scroll, "Spec sections");
    let mut picked = None;
    ui.within(&scroll, &mut |ui| {
        let rail = if narrow {
            Node::row().w(Size::Fit).pad(4.0)
        } else {
            Node::column()
                .w(Size::Grow)
                .padding(Edges::new(0.0, 8.0, 0.0, 32.0))
        };
        let build = |ui: &mut Ui| {
            let mut previous_group = "";
            for (index, &(anchor, group, name, _)) in SECTIONS.iter().enumerate() {
                if !narrow && group != previous_group {
                    previous_group = group;
                    ui.column(
                        Node::column()
                            .w(Size::Grow)
                            .padding(Edges::new(12.0, 12.0, 12.0, 4.0)),
                        |ui| {
                            doc_label(ui, group, SECTION_CAPTION, QUIET);
                        },
                    );
                }
                let inset = if narrow { 8.0 } else { 12.0 };
                let width = if narrow {
                    Size::Px(name.len() as f32 * 6.6 + 2.0 * inset)
                } else {
                    Size::Grow
                };
                let button = Button::DEFAULT
                    .size(width, Size::Px(23.4))
                    .centered()
                    .look(&SPEC_NAV)
                    .selected(selected == index)
                    .label(NAV_TEXT)
                    .content_align(Along::Start)
                    .pad(Edges::new(inset, 0.0, inset, 0.0));
                if ui.button(anchor, name, &button).clicked {
                    picked = Some(index);
                }
            }
        };
        if narrow {
            ui.row(rail, build);
        } else {
            ui.column(rail, build);
        }
    });
    picked
}

fn spec_document(
    ui: &mut Ui,
    narrow: bool,
    (bindings, parity): (&[BindingDescription], &Parity),
    selection: &mut Selection,
    picked: &mut Option<usize>,
) -> Option<(Option<usize>, f32)> {
    let mut sections = [None; SECTIONS.len()];
    let doc = ui.node(
        "spec-document",
        Node::column()
            .w(Size::Grow)
            .h(Size::Grow)
            .flags(Flags::SCROLL | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
            .bar(Bar::new(Rgba::rgb(56, 56, 59), 6.0).gap(2.0))
            .role(Role::ScrollArea)
            .look(Look::NONE.fill(BG)),
    );
    ui.a11y_name(&doc, "Spec document");
    ui.within(&doc, &mut |ui| {
        let padding = if narrow {
            Edges::new(16.0, 12.0, 16.0, 80.0)
        } else {
            Edges::new(32.0, 20.0, 32.0, 120.0)
        };
        ui.column(Node::DEFAULT.w(Size::Grow).padding(padding), |ui| {
            for (index, &(anchor, _, name, explanation)) in SECTIONS.iter().enumerate() {
                let section = ui.node(anchor, Node::column().w(Size::Grow).max_w(980.0));
                sections[index] = Some(section);
                ui.within(&section, &mut |ui| {
                    if index != 0 {
                        row_rule(ui);
                    }
                    let body = Node::DEFAULT
                        .w(Size::Grow)
                        .padding(Edges::new(
                            0.0,
                            if index == 0 { 0.0 } else { 20.0 },
                            0.0,
                            20.0,
                        ))
                        .gap(8.0);
                    ui.column(body, |ui| {
                        if plan::document(ui, anchor, narrow, picked, &mut selection.external_link)
                        {
                            return;
                        }
                        if engineering::document(ui, anchor, narrow, picked) {
                            return;
                        }
                        if product::document(ui, anchor, narrow, picked) {
                            return;
                        }
                        let title = match anchor {
                            "s-overview" => "Splice: the main spec",
                            "s-media" => "Media on macOS (no ffmpeg)",
                            _ => name,
                        };
                        if matches!(anchor, "s-effects" | "s-captions" | "s-motion") {
                            model_heading(ui, title);
                        } else {
                            doc_label(ui, title, HEADING, INK);
                        }
                        if anchor == "s-overview" {
                            overview(ui, narrow, explanation, picked);
                            return;
                        }
                        match anchor {
                            "s-decisions" => {
                                let width = spec_table_width(ui, narrow);
                                ui.view_of("decisions", &width.to_bits(), |ui| {
                                    decisions(ui, width)
                                });
                                return;
                            }
                            "s-features" => {
                                feature_inventory(ui, narrow, picked);
                                return;
                            }
                            "s-lanes" => {
                                model::timeline(ui, spec_table_width(ui, narrow), picked);
                                return;
                            }
                            "s-effects" => {
                                model::effects(ui, spec_table_width(ui, narrow), picked);
                                return;
                            }
                            "s-framing" => {
                                framing_model(ui, spec_table_width(ui, narrow), picked);
                                return;
                            }
                            "s-captions" => {
                                caption_model(ui, spec_table_width(ui, narrow), narrow, picked);
                                return;
                            }
                            "s-motion" => {
                                motion_model(ui, spec_table_width(ui, narrow), narrow, picked);
                                return;
                            }
                            "s-registry" | "s-mcp" | "s-parity" => {
                                let width = spec_table_width(ui, narrow);
                                ui.column(Node::column().w(Size::Grow), |ui| match anchor {
                                    "s-registry" => registry::command_registry(ui, width, bindings),
                                    "s-mcp" => registry::mcp(ui, width, picked),
                                    _ => registry::parity(
                                        ui,
                                        width,
                                        parity,
                                        &mut selection.parity_requested,
                                        &mut selection.parity_details,
                                    ),
                                });
                                return;
                            }
                            _ => {}
                        }
                        ui.column(Node::column().w(Size::Grow).max_w(561.6), |ui| {
                            doc_label(ui, explanation, DOC_BODY, MUTED);
                        });
                    });
                });
            }
        });
    });

    let viewport = ui.rect(&doc);
    if viewport.h <= 0.0 {
        return None;
    }
    let scroll = ui.scroll(&doc);
    if selection.restoring && scroll.settled != selection.scroll {
        ui.set_scroll_offset(&doc, selection.scroll);
        return None;
    }
    if let Some(index) = *picked
        && let Some(section) = sections[index]
    {
        let target = (ui.rect(&section).y - viewport.y + scroll.painted).max(0.0);
        ui.set_scroll_offset(&doc, target);
        ui.repaint_after(0.0);
        return Some((Some(index), target));
    }

    let active = sections
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, &section)| {
            let bounds = ui.rect(&section?);
            (bounds.h > 0.0 && bounds.y < viewport.y + 120.0).then_some(index)
        });
    Some((active, scroll.settled))
}

fn doc_label(ui: &mut Ui, text: &str, style: Label, color: Rgba) {
    let label = ui.label(text, style);
    ui.style(
        &label,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}

fn overview(ui: &mut Ui, narrow: bool, lede: &str, picked: &mut Option<usize>) {
    let width = spec_table_width(ui, narrow);
    let card_width = if narrow { width } else { (width - 12.0) * 0.5 };
    let line_width = (card_width - 46.2).max(1.0);
    ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
        ui.column(Node::column().w(Size::Grow).max_w(624.0), |ui| {
            doc_label(ui, lede, DOC_LEDE, INK);
        });
        ui.column(Node::column().w(Size::Grow).max_w(561.6), |ui| {
        overview_spans(
            ui,
            "overview-body",
            "Editor and Agent · MCP work on the same project. The editor sends commands to the same registry the terminal and MCP use. The command map, effective keymap, tool list and parity check below read those live registries so they stay tied to the native editor. The MCP surface section also lists the error codes agents use to recover from failed calls.",
            width.min(561.6),
            &[TextSpan::new(0, 6).weight(500).color(INK), TextSpan::new(11, 23).weight(500).color(INK)],
        );
    });
    let grid = Node::column().w(Size::Grow).gap(12.0);
    let card = Node::column()
        .w(Size::Grow)
        .padding(Edges::new(13.0, 9.0, 13.0, 9.0))
        .gap(6.0)
        .look(Look::NONE.fill(PANEL).border(LINE, 1.0));
    let list = Node::column().w(Size::Grow);
    let bullet = Node::row()
        .w(Size::Grow)
        .gap(8.0)
        .padding(Edges::new(5.0, 0.0, 0.0, 0.0));
    let cards = |ui: &mut Ui| {
        ui.column(card, |ui| {
            doc_label(ui, "Principles", SECTION_CAPTION.line_height(14.0), QUIET);
            ui.column(list, |ui| {
                ui.row(bullet, |ui| {
                    doc_label(ui, "•", DOC_BODY.w(Size::Fit), MUTED);
                    overview_spans(
                        ui,
                        "overview-agent-first",
                        "Agent-first. The core and its MCP surface are the product. The UI and agents edit through edit.apply with the same registry.",
                        line_width,
                        &[TextSpan::new(90, 100).color(MOTION)],
                    );
                });
                for text in [
                    "Overlays can follow source frames. Caption cues can follow transcript words.",
                    "Recorder components and effects are additive and can be controlled at their own scope.",
                    "Agents commit batches live. Each batch is one undo step, and a whole session can be reverted together.",
                    "Headless core first. Editing capabilities are registry commands or MCP tools.",
                    "Integer sequence frames and stable prefixed IDs are shared by the viewer, CLI and MCP.",
                    "Native macOS frameworks handle capture, decoding, composition and export.",
                    "The core tests warmed edit, evaluation and playback paths with allocation counters.",
                ] {
                    ui.row(bullet, |ui| {
                        doc_label(ui, "•", DOC_BODY.w(Size::Fit), MUTED);
                        doc_label(ui, text, DOC_BODY, MUTED);
                    });
                }
            });
        });
        ui.column(card, |ui| overview_status(ui, line_width, picked, bullet));
    };
    if narrow {
        ui.column(grid, cards);
    } else {
        ui.row(grid.align(Align::Stretch), cards);
    }
    });
}

fn overview_spans(ui: &mut Ui, key: &'static str, text: &str, width: f32, spans: &[TextSpan]) {
    let lines = ui
        .text_service()
        .measure_spans(text, DOC_BODY, Some(width), spans)
        .expect("valid overview spans")
        .lines;
    let label = ui.node(
        key,
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(lines as f32 * 19.44)
            .role(Role::Label)
            .look(Look::NONE.text(MUTED)),
    );
    ui.text_spans(&label, text, DOC_BODY, spans)
        .expect("valid overview spans");
}

fn overview_status(ui: &mut Ui, line_width: f32, picked: &mut Option<usize>, bullet: Node) {
    let title = ui.label_fmt(
        format_args!("Status · {}", env!("CARGO_PKG_VERSION")),
        SECTION_CAPTION.line_height(14.0),
    );
    ui.style(
        &title,
        &States::of(Look::NONE.text(QUIET)),
        State::default(),
    );

    ui.column(Node::column().w(Size::Grow), |ui| {
        for (summary, link, tail, badge) in [
            (
                "Delivered:",
                Some(("s-m1", "M1 · Splice 0.1")),
                "Headless editing and native preview",
                ("built", SAGE),
            ),
            ("Timeline model and MCP parity:", None, "", ("live", SAGE)),
            (
                "Rust core, macOS-first, native media:",
                None,
                "",
                ("built", SAGE),
            ),
            (
                "Engineering rules (rays style):",
                None,
                "",
                ("enforced", MOTION),
            ),
            (
                "PXUI renders the editor and handles input and accessibility. splice_kit provides commands and undo.",
                Some(("s-ui", "UI framework: PXUI")),
                "",
                ("in use", SAGE),
            ),
            (
                "Captions and motion graphics:",
                Some(("s-position", "Agent-first and gaps")),
                "",
                ("in use", MOTION),
            ),
        ] {
            ui.row(bullet, |ui| {
                doc_label(ui, "•", DOC_BODY.w(Size::Fit), MUTED);
                ui.row(
                    Node::row()
                        .w(Size::Grow)
                        .gap(4.0)
                        .row_gap(1.4)
                        .wrap()
                        .align(Align::Center),
                    |ui| {
                        status_text(ui, summary, line_width);
                        if let Some((anchor, label)) = link {
                            let control = Button::sized(label.chars().count() as f32 * 7.2, 19.44)
                                .label(DOC_BODY.w(Size::Fit))
                                .look(&STATUS_LINK);
                            if ui.button(anchor, label, &control).clicked {
                                *picked = SECTIONS.iter().position(|section| section.0 == anchor);
                            }
                        }
                        if !tail.is_empty() {
                            status_text(ui, tail, line_width);
                        }
                        ui.row(
                            Node::row()
                                .w(Size::Fit)
                                .h_px(18.0)
                                .pad_x(6.0)
                                .align(Align::Center)
                                .look(Look::NONE.border(badge.1, 1.0)),
                            |ui| {
                                doc_label(
                                    ui,
                                    badge.0,
                                    Label::MONO.size(10.0).line_height(16.0),
                                    badge.1,
                                );
                            },
                        );
                    },
                );
            });
        }
    });
}

fn status_text(ui: &mut Ui, text: &str, line_width: f32) {
    // A fixed flow item wraps whole words before the badge can move to the next line.
    let width = (text.chars().count() as f32 * 7.2 + 0.1).min(line_width);
    doc_label(ui, text, DOC_BODY.w_px(width), MUTED);
}

const STATUS_LINK: States = States::of(Look::NONE.text(MOTION).border(MOTION, 1.0).sides(BOTTOM))
    .hover(Look::NONE.text(INK).border(INK, 1.0).sides(BOTTOM))
    .focus(Look::NONE.text(INK).border(INK, 1.0));

fn spec_link(ui: &mut Ui, anchor: &'static str, picked: &mut Option<usize>) {
    let Some((index, &(_, _, name, _))) = SECTIONS
        .iter()
        .enumerate()
        .find(|(_, section)| section.0 == anchor)
    else {
        return;
    };
    let link = Button::DEFAULT
        .size(Size::Fit, Size::Px(22.0))
        .look(&NAV_LOOK)
        .label(NAV_TEXT);
    if ui.button(anchor, name, &link).clicked {
        *picked = Some(index);
    }
}

const TABLE_LINE: f32 = 15.4;
const TABLE_TEXT: Label = NAV_TEXT
    .w(Size::Grow)
    .line_height(TABLE_LINE)
    .wrap()
    .role(Role::Cell);
const TABLE_HEAD: Label = SECTION_CAPTION.tracking(0.05).role(Role::ColumnHeader);
const TABLE_ROW: Node = Node::row()
    .w(Size::Grow)
    .align(Align::Start)
    .role(Role::Row);
const TABLE_EDGE: Rgba = Rgba::rgb(56, 56, 59);

fn spec_table(
    ui: &mut Ui,
    name: &'static str,
    columns: &[(&str, Size)],
    draw: &mut dyn FnMut(&mut Ui),
) {
    ui.column(Node::column().w(Size::Grow).pad_y(8.0), |ui| {
        let table = ui.node("spec-table", Node::column().w(Size::Grow).role(Role::Table));
        ui.a11y_name(&table, name);
        ui.within(&table, &mut |ui| {
            ui.row(
                TABLE_ROW
                    .h_px(26.5)
                    .padding(Edges::new(0.0, 6.0, 0.0, 6.0))
                    .look(Look::NONE.fill(BG).border(TABLE_EDGE, 1.0).sides(BOTTOM)),
                |ui| {
                    for &(title, width) in columns {
                        table_cell(ui, width, &mut |ui| doc_label(ui, title, TABLE_HEAD, QUIET));
                    }
                },
            );
            draw(ui);
        });
    });
}

fn table_cell(ui: &mut Ui, width: Size, draw: &mut dyn FnMut(&mut Ui)) {
    ui.column(Node::column().w(width).pad_x(8.0), draw);
}

fn table_row(ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui)) {
    ui.row(TABLE_ROW.padding(Edges::new(0.0, 6.5, 0.0, 5.5)), draw);
    row_rule(ui);
}

fn table_tag(ui: &mut Ui, text: &str, ink: Rgba, edge: Rgba, dashed: bool) {
    let box_ = Node::row()
        .w(Size::Fit)
        .h_px(18.0)
        .align(Align::Center)
        .pad_x(6.0);
    let badge = ui.row(
        if dashed {
            box_
        } else {
            box_.look(Look::NONE.border(edge, 1.0))
        },
        |ui| {
            doc_label(
                ui,
                text,
                TABLE_TEXT.w(Size::Fit).size(10.0).line_height(16.0),
                ink,
            )
        },
    );
    if dashed {
        ui.canvas_over(&badge, move |paint, rect| {
            paint.dashed_rect(rect, Stroke::new(edge));
        });
    }
}

fn spec_table_width(ui: &Ui, narrow: bool) -> f32 {
    (ui.view_size().width - if narrow { 32.0 } else { 284.0 }).clamp(1.0, 980.0)
}

fn decisions(ui: &mut Ui, width: f32) {
    let text_width = (width - 29.2 - 76.0 - 16.0).max(1.0);
    let columns = [
        ("#", Size::Px(29.2)),
        ("Decision", Size::Grow),
        ("State", Size::Px(76.0)),
    ];
    spec_table(ui, "Decisions", &columns, &mut |ui| {
        for &(number, text, state, spans) in DECISIONS {
            table_row(ui, &mut |ui| {
                table_cell(ui, columns[0].1, &mut |ui| {
                    doc_label(ui, number, TABLE_TEXT, INK)
                });
                table_cell(ui, Size::Grow, &mut |ui| {
                    // Inline spans attach to a placed node. Use the same shaper to size its lines.
                    let lines = ui
                        .text_service()
                        .measure_spans(text, TABLE_TEXT, Some(text_width), spans)
                        .expect("valid decision spans")
                        .lines;
                    let label = ui.node(
                        "decision",
                        Node::DEFAULT
                            .w(Size::Grow)
                            .h_px(lines as f32 * TABLE_LINE)
                            .role(Role::Cell)
                            .look(Look::NONE.text(MUTED)),
                    );
                    ui.text_spans(&label, text, TABLE_TEXT, spans)
                        .expect("valid decision spans");
                });
                table_cell(ui, columns[2].1, &mut |ui| {
                    let ink = if state == "locked" { SAGE } else { MOTION };
                    table_tag(ui, state, ink, ink, false);
                });
            });
        }
    });
}

fn feature_inventory(ui: &mut Ui, narrow: bool, picked: &mut Option<usize>) {
    let width = spec_table_width(ui, narrow);
    ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
        feature_lede(ui, width.min(561.6), picked);
        ui.view_of("feature-inventory", &width.to_bits(), |ui| {
            feature_table(ui, width)
        });
        ui.row(
            Node::row()
                .w(Size::Grow)
                .gap(10.0)
                .wrap()
                .align(Align::Center),
            |ui| {
                let label = ui.label_fmt(
                    format_args!(
                        "Native registry · {} commands · {} MCP tools",
                        COMMANDS.len(),
                        tool_catalog().len()
                    ),
                    DOC_BODY.w(Size::Fit),
                );
                ui.style(
                    &label,
                    &States::of(Look::NONE.text(MUTED)),
                    State::default(),
                );
                for anchor in ["s-registry", "s-mcp"] {
                    spec_link(ui, anchor, picked);
                }
            },
        );
    });
}

fn feature_lede(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow).max_w(width), |ui| {
        doc_label(
            ui,
            "Everything the product does at v1. Proto is the v13 browser.",
            DOC_BODY,
            MUTED,
        );
        ui.row(
            Node::row().w(Size::Grow).wrap().align(Align::Center),
            |ui| {
                const LEAD: &str = "Core is the Rust milestone that owns it (see";
                doc_label(
                    ui,
                    LEAD,
                    DOC_BODY.w_px((LEAD.len() as f32 * 7.2 + 7.2).min(width)),
                    MUTED,
                );
                let link = ui.pressable(
                    "s-roadmap",
                    Node::row()
                        .w_px(50.4)
                        .h_px(19.44)
                        .align(Align::Center)
                        .role(Role::Link),
                    &Pressable::DEFAULT.look(&STATUS_LINK),
                    |ui, _| {
                        doc_label(
                            ui,
                            "Roadmap",
                            DOC_BODY.w(Size::Fit).role(Role::Unknown),
                            MOTION,
                        )
                    },
                );
                ui.a11y_name(&link.handle, "Roadmap");
                if link.clicked {
                    *picked = SECTIONS.iter().position(|section| section.0 == "s-roadmap");
                }
                doc_label(ui, ").", DOC_BODY.w(Size::Fit), MUTED);
            },
        );
    });
}

fn feature_table(ui: &mut Ui, width: f32) {
    // The v13 table's auto layout keeps Proto at its longest tag width. Area
    // and Core receive about 7.4% and 2.5% of the added width, respectively.
    let columns = [
        ("Area", Size::Px((53.64 + width * 0.0744).max(75.4))),
        ("Feature", Size::Grow),
        ("Proto", Size::Px(142.0)),
        ("Core", Size::Px((32.4 + width * 0.02525).max(40.0))),
    ];
    spec_table(ui, "Feature inventory", &columns, &mut |ui| {
        for &(area, feature, prototype, milestone) in FEATURES {
            table_row(ui, &mut |ui| {
                table_cell(ui, columns[0].1, &mut |ui| {
                    doc_label(ui, area, TABLE_TEXT, INK)
                });
                table_cell(ui, Size::Grow, &mut |ui| {
                    doc_label(ui, feature, TABLE_TEXT, MUTED)
                });
                table_cell(ui, columns[2].1, &mut |ui| {
                    let built = prototype.starts_with('✓');
                    let ink = if built { SAGE } else { MUTED };
                    let edge = if built { SAGE } else { TABLE_EDGE };
                    table_tag(ui, prototype, ink, edge, prototype == "planned");
                });
                table_cell(ui, columns[3].1, &mut |ui| {
                    doc_label(ui, milestone, TABLE_TEXT, MUTED)
                });
            });
        }
    });
}

const DECISIONS: &[(&str, &str, &str, &[TextSpan])] = &[
    (
        "1",
        "Anchored overlay lanes. The magnetic Video lane is the spine. B-roll, picture-in-picture, images, titles and motion live on named lanes above it, each item anchored to a source frame of a spine clip. A pinned anchor ignores edits (watermarks, music beds). Overlap bumps an item to a free lane; nothing overwrites.",
        "locked",
        &[TextSpan::new(0, 23).weight(500).color(INK)],
    ),
    (
        "2",
        "One command registry drives everything. Menus, shortcuts, gestures, inspector fields, the command palette, CLI and MCP all call the same registry. A CI check keeps it 1:1.",
        "locked",
        &[TextSpan::new(0, 39).weight(500).color(INK)],
    ),
    (
        "3",
        "A small MCP surface. The v13 prototype has 19 tools covering perception, session and history, view, jobs and recording. The atomic write tool, edit.apply, accepts any registry command.",
        "locked",
        &[
            TextSpan::new(0, 20).weight(500).color(INK),
            TextSpan::new(143, 153).color(MOTION),
        ],
    ),
    (
        "4",
        "Agents edit live. Each edit.apply batch commits as one undo step labelled Agent: …. Undo All Agent Edits (⌥⌘Z, banner button, history.undo {scope:\"session\"}) reverts the session in one step and replays later human edits on top. history.undo {scope:\"to\", revision} restores any earlier state as a new revision.",
        "locked",
        &[
            TextSpan::new(0, 17).weight(500).color(INK),
            TextSpan::new(23, 33).color(MOTION),
            TextSpan::new(74, 84).color(MOTION),
            TextSpan::new(132, 162).color(MOTION),
            TextSpan::new(234, 269).color(MOTION),
        ],
    ),
    (
        "5",
        "Integer frames at the sequence rate, everywhere. Seconds are never accepted.",
        "locked",
        &[TextSpan::new(0, 14).weight(500).color(INK)],
    ),
    (
        "6",
        "Deleting a parent clip deletes its anchored overlays, with an undo notice. overlays:\"keep\" leaves a gap clip that carries them.",
        "locked",
        &[
            TextSpan::new(0, 52).weight(500).color(INK),
            TextSpan::new(75, 90).color(MOTION),
        ],
    ),
    (
        "7",
        "Overlay lanes are unlimited, auto-created, and persist until removed, so lane IDs never renumber.",
        "locked",
        &[TextSpan::new(0, 27).weight(500).color(INK)],
    ),
    (
        "8",
        "Keystroke privacy. Native takes.events redacts text under the project policy.",
        "locked",
        &[
            TextSpan::new(0, 18).weight(500).color(INK),
            TextSpan::new(26, 38).color(MOTION),
        ],
    ),
    (
        "9",
        "Rust core, macOS first. The core is a Rust library with integration tests. Its command surface is exposed 1:1 over MCP.",
        "locked",
        &[TextSpan::new(0, 23).weight(500).color(INK)],
    ),
    (
        "10",
        "Native media frameworks, no vendored codecs. AVFoundation, VideoToolbox, CoreMedia/CoreVideo, Metal, CoreAudio, ScreenCaptureKit and Speech, as in Recorder. No ffmpeg.",
        "locked",
        &[TextSpan::new(0, 44).weight(500).color(INK)],
    ),
    (
        "11",
        "Rays engineering style. std + cc only, hand-written *_sys bindings, gate-enforced shape rules, a counting allocator with zero-alloc assertions, build-time budgets, and cargo t with hard timeouts.",
        "proposed",
        &[
            TextSpan::new(0, 23).weight(500).color(INK),
            TextSpan::new(24, 27).color(MOTION),
            TextSpan::new(30, 32).color(MOTION),
            TextSpan::new(52, 57).color(MOTION),
            TextSpan::new(168, 175).color(MOTION),
        ],
    ),
    (
        "13",
        "Recorder features are additive layers. Components (what was recorded), effects (treatments at clip, lane or project scope, resolved per parameter), time-ranged items and built-in properties. No global recorder settings.",
        "proposed",
        &[TextSpan::new(0, 38).weight(500).color(INK)],
    ),
    (
        "14",
        "Crop is a built-in per-clip property in normalized source coordinates, with fit or fill; padding applies after crop.",
        "proposed",
        &[TextSpan::new(0, 36).weight(500).color(INK)],
    ),
    (
        "15",
        "Crossfades are overlaps. The v13 design fades overlay items. Native uses transition.add between clips.",
        "proposed",
        &[
            TextSpan::new(0, 24).weight(500).color(INK),
            TextSpan::new(73, 87).color(MOTION),
        ],
    ),
    (
        "12",
        "UI framework: PXUI (rust-port, the LocalWave Unification Plan), pinned as a submodule. Whatever generic piece PXUI lacks (command system, responder chain, undo manager, gesture phases, later video textures and the native menu bar) is built in this repo as splice_kit, in PXUI's shape and rules, and moves into PXUI once PXUI is mature. PXUI itself is not changed during M1.",
        "locked",
        &[
            TextSpan::new(0, 18).weight(500).color(INK),
            TextSpan::new(256, 266).color(MOTION),
        ],
    ),
    (
        "16",
        "Agent-first positioning. The edge is the quality of the contract, not having MCP (Resolve 21.1 has one): complete coverage, transactions with dry run and session undo, recorded screen data, permissions and audit.",
        "proposed",
        &[TextSpan::new(0, 24).weight(500).color(INK)],
    ),
    (
        "17",
        "Captions are word-bound cues on Captions lanes. Timing is derived from transcript words, text edits are per word and never change the transcript, and the look is the Caption Style effect (project › lane › cue).",
        "proposed",
        &[TextSpan::new(0, 28).weight(500).color(INK)],
    ),
    (
        "18",
        "Motion graphics are parameters at stable paths with keyframes, links and provenance. Templates are parameter schemas, and text animators are data. Scope is capped at 2D.",
        "proposed",
        &[TextSpan::new(0, 46).weight(500).color(INK)],
    ),
    (
        "19",
        "No approvals, no drafts. Agents edit, export and record without asking. Safety comes from history: an fsynced journal, undo of the last batch, a session or back to any revision, and an audit trail. Export never overwrites unless asked.",
        "locked",
        &[TextSpan::new(0, 24).weight(500).color(INK)],
    ),
    (
        "20",
        "Headless core first (M1). Core + MCP + CLI + export, with a viewer that is only a frame and a play bar. splice mcp never opens UI.",
        "locked",
        &[
            TextSpan::new(0, 25).weight(500).color(INK),
            TextSpan::new(104, 114).color(MOTION),
        ],
    ),
    (
        "21",
        "The product is called Splice. Crates splice_*, CLI splice, projects .splice, bundle com.olegpustovit.splice.",
        "locked",
        &[
            TextSpan::new(0, 29).weight(500).color(INK),
            TextSpan::new(37, 45).color(MOTION),
            TextSpan::new(51, 57).color(MOTION),
            TextSpan::new(68, 75).color(MOTION),
            TextSpan::new(84, 107).color(MOTION),
        ],
    ),
    (
        "22",
        "Journal-first project package. Name.splice/ with a hash-chained, fsynced journal.ops as the source of truth and periodic project.json snapshots. Canonical JSON, so hashes are stable.",
        "locked",
        &[
            TextSpan::new(0, 30).weight(500).color(INK),
            TextSpan::new(31, 43).color(MOTION),
            TextSpan::new(73, 84).color(MOTION),
            TextSpan::new(121, 133).color(MOTION),
        ],
    ),
    (
        "23",
        "Agents improve Splice. Every failed call, agent undo and friction note is logged; the build loop turns each cluster into a test and a core fix.",
        "locked",
        &[
            TextSpan::new(0, 22).weight(500).color(INK),
            TextSpan::new(57, 65).color(MOTION),
        ],
    ),
];

// The Proto column describes the v13 browser fixture. The live catalogs below show native support.
const FEATURES: &[(&str, &str, &str, &str)] = &[
    (
        "Timeline",
        "Magnetic spine with ripple on/off, reorder by drag",
        "✓",
        "M1",
    ),
    (
        "Timeline",
        "Split, blade (sticky and momentary ⌥), trim to playhead, edge trims",
        "✓",
        "M1",
    ),
    (
        "Timeline",
        "Snapping to edges, playhead, markers, clicks and keystrokes; ⌘ bypass",
        "✓",
        "M4",
    ),
    (
        "Timeline",
        "Zoom around pointer or playhead, fit, zoom to selection, overview strip",
        "✓",
        "M4",
    ),
    (
        "Timeline",
        "Copy, cut (moves), paste, duplicate, nudge, marquee and lane selection",
        "✓",
        "M1 / M4",
    ),
    (
        "Timeline",
        "Speed 0.25–16× with ripple; speed ramps",
        "✓ / ramps planned",
        "M1 / M6",
    ),
    (
        "Timeline",
        "Fades by handle, menu or inspector; color tags; names",
        "✓",
        "M1",
    ),
    ("Timeline", "Join, ungroup, restore cuts", "planned", "M6"),
    (
        "Effects",
        "Window (padding, background, corners, shadow, inset), Cursor, Click Zoom, Camera Layout, Keystroke Chips",
        "✓",
        "M1 / M3",
    ),
    (
        "Effects",
        "Clip, lane and project scopes; per-parameter overrides; turn off here; promote to lane or project",
        "✓",
        "M1",
    ),
    (
        "Effects",
        "Effects browser with drag and drop; inspector cards with source scopes and reset dots; fx badges by shape",
        "✓",
        "M4",
    ),
    (
        "Effects",
        "Paste Effects (merge), Remove Effects",
        "✓",
        "M1",
    ),
    (
        "Captions",
        "Generate from the transcript (1–8 words, fillers optional); cues follow their words through cuts and speed changes",
        "✓",
        "M6",
    ),
    (
        "Captions",
        "Per-word text, emphasis, emoji and hide; retype a cue; split and merge",
        "✓",
        "M6",
    ),
    (
        "Captions",
        "Caption Style effect: 6 presets (bold, karaoke, pop, box, clean, typewriter), colours, stroke, position, platform safe zones",
        "✓",
        "M6",
    ),
    (
        "Captions",
        "SRT / VTT / JSON through transcript.get; Copy as SRT",
        "✓",
        "M6",
    ),
    (
        "Captions",
        "Translate, import SRT, speaker lanes, burn-in and embedded caption tracks, 20+ presets",
        "planned",
        "M6",
    ),
    (
        "Motion",
        "Parameter paths, keyframes with ease curves, templates, brand kit, text animators",
        "planned",
        "M8",
    ),
    (
        "Effects",
        "Motion blur, blur regions, spotlight, device frame, background blur, click sound, presets, time-ranged effects",
        "planned",
        "M6",
    ),
    (
        "Framing",
        "Per-clip crop overlay with aspect locks, fit or fill; 9:16, 4:5, 1:1, 4:3, 16:9 canvas",
        "✓",
        "M1 / M4",
    ),
    ("Framing", "Animated crop (reframe keys)", "planned", "M6"),
    (
        "Viewer",
        "Fit, pinch and ⌘-scroll zoom around the pointer, pan, 25–400%, active-pane ⌘± and ⇧Z",
        "✓",
        "M4",
    ),
    (
        "Lanes and anchoring",
        "Graphics, Overlay n…1 and Motion lanes; unlimited, auto-created, stable IDs",
        "✓",
        "M1",
    ),
    (
        "Lanes and anchoring",
        "Anchors in parent source time; pinned anchors with PIN badge",
        "✓",
        "M1",
    ),
    (
        "Lanes and anchoring",
        "Auto-bump on overlap; explicit lane gives LANE_CONFLICT",
        "✓",
        "M1",
    ),
    (
        "Lanes and anchoring",
        "Connector lines from overlay to anchor",
        "✓",
        "M4",
    ),
    (
        "Lanes and anchoring",
        "Grave-key move without overlays",
        "✓",
        "M4",
    ),
    (
        "Lanes and anchoring",
        "Lift to Overlay, Replace Spine Clip, Delete Keep Overlays (gap clip)",
        "✓",
        "M1",
    ),
    (
        "Lanes and anchoring",
        "Lane create, rename (double-click), delete; reorder",
        "✓ / reorder planned",
        "M1",
    ),
    (
        "Lanes and anchoring",
        "Cascade delete notice with Undo",
        "✓",
        "M4",
    ),
    (
        "Overlays",
        "Media overlays: placement full, card or corners; behind presenter; muted b-roll",
        "✓",
        "M1 / M3",
    ),
    (
        "Overlays",
        "Titles with fades; scale animations with keyframes",
        "✓",
        "M1 / M3",
    ),
    (
        "Overlays",
        "Masks (cover, blur, highlight), zoom settings, captions, style",
        "planned",
        "M6",
    ),
    (
        "Takes",
        "Components on one source clock: screen, camera, mic, system audio, cursor, keystrokes",
        "✓",
        "M1 / M3",
    ),
    (
        "Takes",
        "Per-clip component visibility, volume, cursor size and rings, keystroke chips",
        "✓",
        "M1",
    ),
    ("Takes", "Hide single keystrokes; chip size", "✓", "M1"),
    ("Takes", "Audio link, unlink and sync", "planned", "M6"),
    (
        "Markers",
        "Add, rename, move by drag or frame, delete; ruler markers",
        "✓",
        "M1",
    ),
    (
        "Transcript",
        "Word IDs with frames; word anchors with hash guard; cut words and fillers",
        "✓ (synthetic)",
        "M6",
    ),
    (
        "Transcript",
        "Pause eraser, intents (pace, fit duration, keep shots, auto zoom)",
        "planned",
        "M6",
    ),
    (
        "Agent",
        "19-tool prototype MCP surface; edit.apply with refs, dry run, idempotency, ifRevision",
        "✓ (simulated)",
        "M2",
    ),
    (
        "Agent",
        "Live sessions, AGENT badges, activity panel, undo batch, undo all (⌥⌘Z) with human replay",
        "✓",
        "M2",
    ),
    (
        "Agent",
        "Access level off, read or live per project",
        "✓",
        "M2",
    ),
    (
        "Agent",
        "Export with no approval (never overwrites); recording jobs; jobs with progress and cancel",
        "✓ (simulated)",
        "M1",
    ),
    (
        "Agent",
        "Command palette over the whole registry (⌘K)",
        "✓",
        "M4",
    ),
    (
        "Preview",
        "Program monitor with compositing order, fades, behind presenter",
        "✓ (canvas)",
        "M3",
    ),
    (
        "Preview",
        "Transport: J/K/L shuttle, frame step, edit-point jumps",
        "✓",
        "M3 / M4",
    ),
    (
        "Preview",
        "Zero-copy VideoToolbox → Metal playback; audio output",
        "native only",
        "M3",
    ),
    (
        "Project",
        "Canvas 9:16, 1:1, 16:9; 30 fps integer frames",
        "✓",
        "M1",
    ),
    (
        "Project",
        "Package with journal, crash recovery by replay",
        "native only",
        "M1",
    ),
    (
        "Export",
        "H.264, HEVC and ProRes via VideoToolbox; GIF; pixel parity with preview",
        "native only",
        "M5",
    ),
    (
        "Import",
        "Local video import (prototype); Recorder takes with events",
        "✓ / bridge planned",
        "M3 / M7",
    ),
];

mod model;
mod product;

struct ModelPoint {
    text: &'static str,
    spans: &'static [TextSpan],
    link: Option<(&'static str, &'static str)>,
}

fn model_heading(ui: &mut Ui, title: &str) {
    ui.row(
        Node::row()
            .w(Size::Grow)
            .gap(8.0)
            .align(Align::Start)
            .wrap(),
        |ui| {
            doc_label(ui, title, HEADING.w(Size::Fit), INK);
            ui.column(
                Node::column()
                    .w(Size::Fit)
                    .padding(Edges::new(0.0, 17.0, 0.0, 0.0)),
                |ui| {
                    ui.row(
                        Node::row()
                            .w(Size::Fit)
                            .h_px(18.0)
                            .pad_x(6.0)
                            .align(Align::Center)
                            .look(Look::NONE.border(MOTION, 1.0)),
                        |ui| {
                            doc_label(
                                ui,
                                "v13 proposed",
                                CAPTION.w(Size::Fit).line_height(16.0),
                                MOTION,
                            );
                        },
                    );
                },
            );
        },
    );
}

fn model_link(ui: &mut Ui, anchor: &'static str, name: &'static str, picked: &mut Option<usize>) {
    let link = ui.pressable(
        anchor,
        Node::row()
            .w(Size::Fit)
            .h_px(19.44)
            .align(Align::Center)
            .role(Role::Link),
        &Pressable::DEFAULT.look(&STATUS_LINK),
        |ui, _| doc_label(ui, name, DOC_BODY.w(Size::Fit).role(Role::Unknown), MOTION),
    );
    ui.a11y_name(&link.handle, name);
    if link.clicked {
        *picked = SECTIONS.iter().position(|section| section.0 == anchor);
    }
}

fn model_list(ui: &mut Ui, width: f32, points: &[ModelPoint], picked: &mut Option<usize>) {
    let width = (width - 20.0).clamp(1.0, 561.6);
    ui.column(Node::column().w(Size::Grow).role(Role::List), |ui| {
        for point in points {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .align(Align::Start)
                    .role(Role::ListItem),
                |ui| {
                    doc_label(ui, "•", DOC_BODY.w_px(20.0).role(Role::Unknown), MUTED);
                    ui.column(Node::column().w_px(width), |ui| {
                        let style = DOC_BODY.role(Role::Paragraph);
                        let lines = ui
                            .text_service()
                            .measure_spans(point.text, style, Some(width), point.spans)
                            .expect("valid model document spans")
                            .lines;
                        let text = ui.node(
                            "model-point",
                            Node::DEFAULT
                                .w(Size::Grow)
                                .h_px(lines as f32 * 19.44)
                                .role(Role::Paragraph)
                                .look(Look::NONE.text(MUTED)),
                        );
                        ui.text_spans(&text, point.text, style, point.spans)
                            .expect("valid model document spans");
                        if let Some((anchor, name)) = point.link {
                            model_link(ui, anchor, name, picked);
                        }
                    });
                },
            );
        }
    });
}

fn model_subsection(
    ui: &mut Ui,
    title: &str,
    width: f32,
    points: &[ModelPoint],
    picked: &mut Option<usize>,
) {
    ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
        doc_label(ui, title, SUBHEADING, INK);
        model_list(ui, width, points, picked);
    });
}

fn model_cards(
    ui: &mut Ui,
    width: f32,
    narrow: bool,
    cards: [(&'static str, &[ModelPoint]); 2],
    picked: &mut Option<usize>,
) {
    let card_width = if narrow { width } else { (width - 12.0) * 0.5 };
    let grid = Node::column().w(Size::Grow).gap(12.0);
    let draw = |ui: &mut Ui| {
        for (heading, points) in cards {
            let card = ui.column(
                Node::column()
                    .w(Size::Grow)
                    .padding(Edges::new(13.0, 9.0, 13.0, 15.0))
                    .gap(6.0)
                    .role(Role::Group)
                    .look(Look::NONE.fill(PANEL).border(LINE, 1.0)),
                |ui| {
                    doc_label(
                        ui,
                        heading,
                        SECTION_CAPTION.line_height(13.0).role(Role::Heading),
                        QUIET,
                    );
                    model_list(ui, card_width - 26.0, points, picked);
                },
            );
            ui.a11y_name(&card, heading);
        }
    };
    if narrow {
        ui.column(grid, draw);
    } else {
        ui.row(grid.align(Align::Stretch), draw);
    }
}

fn framing_model(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(
        Node::column()
            .w(Size::Grow)
            .padding(Edges::new(0.0, 12.0, 0.0, 6.0))
            .gap(20.0),
        |ui| {
            model_subsection(ui, "Crop", width, FRAMING_CROP, picked);
            model_subsection(ui, "Viewer zoom", width, FRAMING_ZOOM, picked);
        },
    );
}

fn caption_model(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow).padding(Edges::new(0.0, 0.0, 0.0, 6.0)).gap(20.0), |ui| {
        ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
            ui.column(Node::column().w(Size::Grow).max_w(624.0), |ui| {
                doc_label(ui, "Captions come from the recorded audio’s transcript, are edited word by word by humans and agents through the same commands, and are drawn by a visual layer with TikTok-style presets. Cues can follow transcript words or hold free text imported from SRT or WebVTT.", DOC_LEDE, INK);
            });
            model_cards(ui, width, narrow, [("Model", CAPTION_MODEL), ("Look: the Caption Style effect", CAPTION_STYLE)], picked);
        });
        caption_commands(ui, width);
        model_subsection(ui, "From speech to word timing on macOS", width, CAPTION_SPEECH, picked);
    });
}

fn caption_commands(ui: &mut Ui, width: f32) {
    ui.column(Node::column().w(Size::Grow).gap(8.0), |ui| {
        ui.column(Node::column().w(Size::Grow).gap(4.0), |ui| {
            doc_label(ui, "Commands", SUBHEADING, INK);
            // Intrinsic source widths plus their shares of the extra width in the v13 table.
            let columns = [
                ("Command", Size::Px((112.72 + width * 0.0448).max(120.0))),
                ("What it does", Size::Grow),
                ("UI", Size::Px((-14.28 + width * 0.3024).max(72.0))),
            ];
            spec_table(ui, "Caption commands", &columns, &mut |ui| {
                for &(command, detail, surface) in CAPTION_COMMAND_ROWS {
                    table_row(ui, &mut |ui| {
                        table_cell(ui, columns[0].1, &mut |ui| {
                            doc_label(ui, command, TABLE_TEXT.size(10.45), MOTION);
                        });
                        table_cell(ui, Size::Grow, &mut |ui| {
                            doc_label(ui, detail, TABLE_TEXT, MUTED)
                        });
                        table_cell(ui, columns[2].1, &mut |ui| {
                            doc_label(ui, surface, TABLE_TEXT, MUTED)
                        });
                    });
                }
            });
        });
        let code = ui.column(
            Node::column()
                .w(Size::Grow)
                .padding(Edges::new(13.0, 9.0, 13.0, 9.0))
                .look(Look::NONE.fill(SURFACE).border(LINE, 1.0))
                .role(Role::Group),
            |ui| {
                doc_label(
                    ui,
                    CAPTION_BATCH,
                    NAV_TEXT.w(Size::Grow).line_height(17.6).wrap_glyph(),
                    INK,
                );
            },
        );
        ui.a11y_name(&code, "Caption edit.apply example");
    });
}

fn motion_model(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow).padding(Edges::new(0.0, 0.0, 0.0, 6.0)).gap(6.0), |ui| {
        ui.column(Node::column().w(Size::Grow).max_w(624.0), |ui| {
            doc_label(ui, "Motion graphics have to be as addressable as edits, so an agent can set a curve as precisely as a human drags one. Scope is 2D: layers, text animators, shapes, masks, point tracking and deterministic particles. Parameters have stable addresses in the edit registry.", DOC_LEDE, INK);
        });
        model_cards(ui, width, narrow, [("Principles", MOTION_PRINCIPLES), ("Commands", MOTION_COMMANDS)], picked);
        ui.column(Node::column().w(Size::Grow).max_w(561.6), |ui| {
            doc_label(ui, "V13 order of work: stable parameter paths and keyframes → templates and brand kit → text animators → masks and point tracking → shapes, blend modes and motion blur. The command registry reflects the native support today.", DOC_BODY, MUTED);
        });
    });
}

const FRAMING_CROP: &[ModelPoint] = &[
    ModelPoint {
        text: "Per clip, in source terms. A crop is a rectangle normalized to the clip's native source frame ({x,y,w,h} in 0–1), so it survives proxies and resolution changes. Spine clips and media overlays both have one. Generated graphics have no fixed frame and adapt to their box instead (WRONG_KIND).",
        spans: &[
            TextSpan::new(0, 26).weight(500).color(INK),
            TextSpan::new(95, 104).color(MOTION),
            TextSpan::new(280, 290).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Fit or fill. Fit letterboxes the crop inside the space the Window leaves; fill covers it and trims the excess around the crop's centre.",
        spans: &[TextSpan::new(0, 12).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Crop overlay (⇧C, the ⌗ Crop button, Canvas inspector or the clip menu): the full source frame with the rectangle, eight handles, rule-of-thirds and a dimmed outside. Aspect locks: Free, Output, 16:9, 4:3, 1:1, 4:5, 9:16; ⇧ while dragging keeps the current ratio. Arrows nudge 1 px (⇧ 10 px). ↩ applies one clip.crop step; Esc cancels. This keeps Recorder's crop sheet semantics (apply / cancel, source pixels) inside the viewer, using the source frame and pixel controls.",
        spans: &[
            TextSpan::new(0, 12).weight(500).color(INK),
            TextSpan::new(317, 326).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Inspector ▸ Framing shows the crop in source pixels with Crop…, Reset and Fit / Fill.",
        spans: &[],
        link: None,
    },
];

const FRAMING_ZOOM: &[ModelPoint] = &[
    ModelPoint {
        text: "Fit by default. Pinch on the trackpad or ⌘-scroll zooms around the pointer; scroll or drag pans when zoomed; double-click toggles Fit and 100% (output pixels).",
        spans: &[TextSpan::new(0, 3).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "The zoom button (Fit · 22%) opens Fit, Zoom In, Zoom Out and 25–400%.",
        spans: &[],
        link: None,
    },
    ModelPoint {
        text: "⌘= / ⌘− / ⇧Z act on the active pane: the viewer or timeline, whichever you last pointed at, as in Final Cut Pro.",
        spans: &[TextSpan::new(0, 43).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Viewer-only framing. Use ui.show {viewer:{zoom:\"fit\"}} or ui.show {viewer:{zoom:1}} for 100%. The Metal preview transform draws at display resolution and leaves the composition and undo history intact.",
        spans: &[
            TextSpan::new(0, 20).weight(500).color(INK),
            TextSpan::new(25, 54).color(MOTION),
            TextSpan::new(58, 83).color(MOTION),
        ],
        link: None,
    },
];

const CAPTION_MODEL: &[ModelPoint] = &[
    ModelPoint {
        text: "A Captions lane (kind:\"captions\") sits above Graphics. A project can have several, for example a translation or a second speaker.",
        spans: &[
            TextSpan::new(2, 15).weight(500).color(INK),
            TextSpan::new(17, 32).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "A caption cue (ovl_…, kind:\"caption\") stores word IDs in words and per-word overrides in edits. Imported cues can also carry free text.",
        spans: &[
            TextSpan::new(2, 13).weight(500).color(INK),
            TextSpan::new(15, 22).color(MOTION),
            TextSpan::new(24, 38).color(MOTION),
            TextSpan::new(59, 64).color(MOTION),
            TextSpan::new(91, 96).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Timing is derived. After every batch the core places each cue from where its words sit on the timeline. Cut, speed up or move the take, and the captions follow. A cue whose words are all cut is removed as a reported side effect, and undo restores it.",
        spans: &[TextSpan::new(0, 18).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Display edits never touch the transcript. The transcript hash, word anchors and cut.words keep working after a caption is retyped.",
        spans: &[
            TextSpan::new(0, 41).weight(500).color(INK),
            TextSpan::new(80, 89).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Word-bound cues follow their words. Split or merge regroups them. Free-text cues can be moved and trimmed; word edits return WRONG_KIND.",
        spans: &[
            TextSpan::new(0, 35).weight(500).color(INK),
            TextSpan::new(125, 135).color(MOTION),
        ],
        link: None,
    },
];

const CAPTION_STYLE: &[ModelPoint] = &[
    ModelPoint {
        text: "The captions effect resolves parameters across project › Captions lane › cue. Style the track on its lane and override individual cues as with Window and Cursor.",
        spans: &[TextSpan::new(4, 12).color(MOTION)],
        link: Some(("s-effects", "Effects")),
    },
    ModelPoint {
        text: "Parameters include preset (bold, karaoke, pop, box, clean, typewriter or a named look), size, position, color, active (highlight), stroke, uppercase and safe (tiktok, reels, shorts, none).",
        spans: &[
            TextSpan::new(19, 25).color(MOTION),
            TextSpan::new(88, 92).color(MOTION),
            TextSpan::new(94, 102).color(MOTION),
            TextSpan::new(104, 109).color(MOTION),
            TextSpan::new(111, 117).color(MOTION),
            TextSpan::new(131, 137).color(MOTION),
            TextSpan::new(139, 148).color(MOTION),
            TextSpan::new(153, 157).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Per-word emphasis takes the highlight colour and +12% scale; emoji follow the word.",
        spans: &[],
        link: None,
    },
    ModelPoint {
        text: "Captions usually paint above titles. The viewer shows the platform safe zone when a cue or its lane is selected.",
        spans: &[],
        link: None,
    },
];

const CAPTION_SPEECH: &[ModelPoint] = &[
    ModelPoint {
        text: "SpeechAnalyzer runs on device on macOS 26. Native transcription reads audioTimeRange from final results and stores word timing in source frames.",
        spans: &[
            TextSpan::new(0, 14).weight(500).color(INK),
            TextSpan::new(70, 84).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Word alignment. Accurate word edges support captions, word anchors and cut.words. A separate forced-alignment pass was part of the v13 design.",
        spans: &[
            TextSpan::new(0, 15).weight(500).color(INK),
            TextSpan::new(71, 80).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Provider design. V13 proposed choosing alternate speech providers. The native path uses the system transcriber and keeps the captured words and their timing.",
        spans: &[TextSpan::new(0, 16).weight(500).color(INK)],
        link: Some(("s-media", "Media on macOS")),
    },
    ModelPoint {
        text: "The transcript is part of the project and versioned by its hash. Send the hash from transcript.get with a word-based edit so TRANSCRIPT_CHANGED can report a stale transcript.",
        spans: &[
            TextSpan::new(84, 98).color(MOTION),
            TextSpan::new(125, 143).color(MOTION),
        ],
        link: None,
    },
];

const MOTION_PRINCIPLES: &[ModelPoint] = &[
    ModelPoint {
        text: "Parameters have stable paths. Pair a target with a type.param path, such as captions.size on a cue, window.padding on a clip or transform.scale on an overlay.",
        spans: &[
            TextSpan::new(0, 29).weight(500).color(INK),
            TextSpan::new(37, 43).color(MOTION),
            TextSpan::new(51, 61).color(MOTION),
            TextSpan::new(76, 89).color(MOTION),
            TextSpan::new(100, 114).color(MOTION),
            TextSpan::new(128, 143).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Keyframes have {at, value, ease}. Item frames are relative to the owner’s start. Lane and project frames are absolute. Eases include hold, linear, named curves and cubic Bézier control points.",
        spans: &[
            TextSpan::new(0, 9).weight(500).color(INK),
            TextSpan::new(15, 32).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Provenance follows scope. Inspect the project, lane and item overrides to trace the value. items.inspect shows the item’s parameters and keys; effect cards show inherited settings.",
        spans: &[
            TextSpan::new(0, 25).weight(500).color(INK),
            TextSpan::new(91, 104).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Templates combine fields and layers. Built-in template names address groups such as a lower third or callout. The project’s style provides fonts and colours, and an instance can change its exposed text fields.",
        spans: &[TextSpan::new(0, 36).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Text animators are data. Native titles accept an entrance by character, word or line through title.set {in:{preset,unit,duration,stagger}}. Caption presets use the timing of transcript words for their active state.",
        spans: &[
            TextSpan::new(0, 24).weight(500).color(INK),
            TextSpan::new(93, 138).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Links before expressions. V13 proposed B = f(A) as a small, deterministic, serializable expression for linking parameters.",
        spans: &[
            TextSpan::new(0, 25).weight(500).color(INK),
            TextSpan::new(39, 47).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Semantic arguments sit next to literal ones. Use zoom.add {focus:\"cursor\"} or intent.autoZoom to use the take’s recorded activity for zoom.",
        spans: &[
            TextSpan::new(0, 18).weight(500).color(INK),
            TextSpan::new(49, 74).color(MOTION),
            TextSpan::new(78, 93).color(MOTION),
        ],
        link: None,
    },
];

const MOTION_COMMANDS: &[ModelPoint] = &[
    ModelPoint {
        text: "param.set {target,path,value} sets a number. param.keys {target,path,keys,mode} replaces or merges keys. Both run in edit.apply and are undoable.",
        spans: &[
            TextSpan::new(0, 29).color(MOTION),
            TextSpan::new(45, 79).color(MOTION),
            TextSpan::new(117, 127).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "param.link was a v13 proposal for linking values.",
        spans: &[TextSpan::new(0, 10).color(MOTION)],
        link: None,
    },
    ModelPoint {
        text: "template.add {name,at,fields}, template.set {item,fields}, and style.set for project fonts, colours and accents.",
        spans: &[
            TextSpan::new(0, 29).color(MOTION),
            TextSpan::new(31, 57).color(MOTION),
            TextSpan::new(63, 72).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "title.set {item,in:{preset,unit,duration,stagger}} animates title units.",
        spans: &[TextSpan::new(0, 50).color(MOTION)],
        link: None,
    },
    ModelPoint {
        text: "Numeric effect fields listed as keyable use the same paths. motion.add creates a scale item, and zoom.add creates a camera segment.",
        spans: &[
            TextSpan::new(60, 70).color(MOTION),
            TextSpan::new(97, 105).color(MOTION),
        ],
        link: Some(("s-registry", "Command registry")),
    },
    ModelPoint {
        text: "The Motion inspector edits ◆ Start and ◆ End on a linear scale item. V13 proposed a curve editor and an item keyframe row.",
        spans: &[],
        link: None,
    },
];

const CAPTION_COMMAND_ROWS: &[(&str, &str, &str)] = &[
    (
        "captions.generate",
        "Groups visible words at maxWords, clip boundaries, pauses over 12 frames (8 frames after two words), or over 22 characters. It replaces cues for those words and can set the lane preset.",
        "Captions ▸ Generate · Words per Caption · Include Filler Words · Captions lane inspector 1w–6w",
    ),
    (
        "caption.set",
        "text retypes the cue in word order. Extra text joins the last word, and missing words are hidden. words applies text, emph, emoji or hidden to a word ID. A null edit restores that word.",
        "Inspector text and word chips · Emphasis · Double-click a cue",
    ),
    (
        "caption.split / caption.merge",
        "Regroups transcript words into cues. Timing stays derived from the words.",
        "C or Blade · Word chip ▸ Split before · Merge with next · Merge Captions",
    ),
    (
        "effect.set {type:\"captions\"}",
        "Set the caption preset or safe zone at project, lane or cue scope. Pass target and type, or the effect ID.",
        "Preset tiles · Caption Style · Captions ▸ Style",
    ),
    (
        "transcript.get {captions}",
        "Reads json with cue and per-word timing, or srt or vtt. Times come from the current cut. Hidden words remain in JSON with a flag and are omitted from subtitle text.",
        "Captions ▸ Copy as SRT",
    ),
    (
        "captions.translate / captions.import",
        "Translate to a new lane with original timing. Import SRT or WebVTT and align cues to existing words where possible. Cues without words remain free text.",
        "Caption commands in edit.apply",
    ),
];

const CAPTION_BATCH: &str = r##"// edit.apply arguments for the sample session at r0
{"projectId":"prj_1","sessionId":"ses_1","ifRevision":"r0","ops":[
  {"cmd":"effect.set","target":"lan_cap","type":"captions","params":{"preset":"pop","active":"#7cf2c4","safe":"reels"}},
  {"cmd":"caption.set","item":"ovl_cap2","words":{"w:screen:5":{"emph":true,"emoji":"🎯"}}},
  {"cmd":"caption.set","item":"ovl_cap4","text":"open the palette"}
]}"##;

fn panel(width: f32) -> Node {
    Node::DEFAULT
        .w_px(width)
        .h(Size::Grow)
        .look(Look::NONE.fill(PANEL))
}

fn pane_heading(ui: &mut Ui, build: &mut dyn FnMut(&mut Ui)) {
    ui.row(
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(29.0)
            .center()
            .pad_x(8.0)
            .look(Look::NONE.fill(PANEL)),
        build,
    );
    row_rule(ui);
}

fn row_rule(ui: &mut Ui) {
    ui.node(
        "horizontal-rule",
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(1.0)
            .look(Look::NONE.fill(LINE)),
    );
}

fn vertical_rule(ui: &mut Ui, key: &'static str) {
    ui.node(
        key,
        Node::DEFAULT
            .w_px(1.0)
            .h(Size::Grow)
            .look(Look::NONE.fill(LINE)),
    );
}

fn group_title(ui: &mut Ui, group: &str) {
    ui.column(
        Node::DEFAULT
            .w(Size::Grow)
            .padding(Edges::new(8.0, 10.0, 8.0, 4.0)),
        |ui| {
            ui.label(group, CAPTION);
        },
    );
}

fn nav(ui: &mut Ui, index: usize, name: &str, selected: bool, height: f32) -> bool {
    let button = Button::DEFAULT
        .size(Size::Grow, Size::Px(height))
        .look(&NAV_LOOK)
        .selected(selected)
        .label(Label::MONO.size(11.0))
        .content_align(Along::Start)
        .pad(Edges::new(6.0, 0.0, 6.0, 0.0));
    let mut picked = false;
    ui.row(Node::DEFAULT.w(Size::Grow).h_px(height), |ui| {
        ui.node(
            "active-indicator",
            Node::DEFAULT
                .w_px(2.0)
                .h_px(height)
                .look(Look::NONE.fill(if selected { ACCENT } else { PANEL })),
        );
        picked = ui.button(index as u64, name, &button).clicked;
    });
    picked
}

const SECTIONS: &[(&str, &str, &str, &str)] = &[
    (
        "s-overview",
        "Product",
        "Overview",
        "Splice is an agent-first macOS video editor for screen recordings, talking-head takes and short social cuts. A Rust core owns the timeline and exposes commands over MCP. The UI gives humans direct manipulation of the same project while agents inspect, edit and verify it through the core. Edits and their undo history are stored in the project package.",
    ),
    ("s-decisions", "Product", "Decisions", ""),
    ("s-features", "Product", "Feature inventory", ""),
    ("s-position", "Product", "Agent-first and gaps", ""),
    ("s-lanes", "Model", "Timeline model", ""),
    ("s-effects", "Model", "Effects and Recorder layers", ""),
    ("s-framing", "Model", "Framing and viewer", ""),
    ("s-captions", "Model", "Captions", ""),
    ("s-motion", "Model", "Motion graphics", ""),
    (
        "s-registry",
        "Model",
        "Command registry",
        "The native editor, terminal and agents use these wire command names.",
    ),
    (
        "s-mcp",
        "Agents",
        "MCP surface",
        "This catalog is generated from the current default server profile.",
    ),
    (
        "s-parity",
        "Agents",
        "Parity check",
        "Editing semantics are shared by the native surfaces through the core apply path. Rendering also has preview and export comparisons.",
    ),
    ("s-improve", "Agents", "Self-improvement", ""),
    (
        "s-arch",
        "Engineering",
        "Architecture",
        "Core state, evaluation and MCP are portable. Native viewer, capture and export are macOS clients of the same project.",
    ),
    (
        "s-ui",
        "Engineering",
        "UI framework: PXUI",
        "PXUI drives the macOS editor’s rendering, input and accessibility. The frame returns editing intents for the project writer.",
    ),
    (
        "s-icons",
        "Engineering",
        "Icon system",
        "Icons identify transport, timeline, editing and viewer actions. Native controls carry accessible text names.",
    ),
    (
        "s-media",
        "Engineering",
        "Media on macOS",
        "ScreenCaptureKit captures the source. AVFoundation and the native GPU compose previews and exports.",
    ),
    (
        "s-rust",
        "Engineering",
        "Rust rules",
        "The Rust core keeps dependencies bounded and uses one project writer. Platform calls live behind safe Rust APIs.",
    ),
    (
        "s-alloc",
        "Engineering",
        "Zero allocation",
        "Warmed edit, evaluation and playback paths are measured with allocation counters.",
    ),
    (
        "s-build",
        "Engineering",
        "Build speed",
        "The repository uses a shared build target and focused workspace checks.",
    ),
    (
        "s-tests",
        "Engineering",
        "Testing",
        "Focused regressions and integration checks run with bounded test deadlines.",
    ),
    (
        "s-perf",
        "Engineering",
        "Performance budgets",
        "The repository tests warmed allocation behavior and measures core apply and rendering on its target machines.",
    ),
    (
        "s-m1",
        "Plan",
        "M1: headless core",
        "The first milestone delivered the timeline engine, MCP surface, file format and native preview.",
    ),
    (
        "s-roadmap",
        "Plan",
        "Roadmap",
        "PLAN.md tracks acceptance checks and completed work.",
    ),
    (
        "s-open",
        "Plan",
        "Open questions",
        "The living design spec records open design questions and their available evidence.",
    ),
    (
        "s-research",
        "Reference",
        "Research",
        "Project decisions, prototype behavior and native outcomes are recorded in the living design spec.",
    ),
    (
        "s-sources",
        "Reference",
        "Sources",
        "Repository references include the live command registry, MCP contract, file format and prototype core.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::testing;
    use std::cell::Cell;

    #[test]
    fn compact_product_tables_fit_and_the_roadmap_link_navigates() {
        use pxui::{Event, FrameIn};

        assert_eq!(DECISIONS.len(), 23);
        assert_eq!(FEATURES.len(), 52);
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let input = FrameIn::new(480.0, 640.0);
        let mut selection = Selection::default();
        let mut draw = |ui: &mut Ui, events: &[Event]| {
            let mut selected = None;
            ui.frame(&input.events(events), &mut |ui| {
                selected = paint(ui, None, View::Spec, selection, &[]).or(selected);
            })
            .unwrap();
            if let Some(next) = selected {
                selection = next;
            }
        };
        draw(&mut ui, &[]);
        draw(&mut ui, &[]);
        for name in ["Decisions", "Feature inventory"] {
            let table = testing::find(&ui, Role::Table, name).unwrap();
            assert_eq!((table.x, table.w), (16.0, 448.0));
        }
        let lead = testing::find(&ui, Role::Cell, DECISIONS[0].1).unwrap();
        assert!(lead.h > 90.0, "{lead:?}");

        for (role, name) in [(Role::Button, "Feature inventory"), (Role::Link, "Roadmap")] {
            let rect = testing::find(&ui, role, name).unwrap();
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                let event = Event::pointer(kind, 0, (rect.x + 5.0).into(), (rect.y + 5.0).into());
                draw(&mut ui, &[event]);
            }
            draw(&mut ui, &[]);
        }
        let heading = testing::find(&ui, Role::Heading, "Roadmap").unwrap();
        assert!((heading.y - 53.4).abs() < 0.1, "{heading:?}");
    }

    #[test]
    fn caption_model_keeps_the_reference_card_breakpoint_and_links() {
        for width in [960.0, 800.0, 480.0] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let input = pxui::FrameIn::new(width, 640.0);
            ui.frame(&input, &mut |ui: &mut Ui| {
                paint(ui, None, View::Spec, Selection::default(), &[]);
            })
            .unwrap();
            let model = testing::find(&ui, Role::Group, "Model").unwrap();
            let style = testing::find(&ui, Role::Group, "Look: the Caption Style effect").unwrap();
            if width > 820.0 {
                assert_eq!(model.y, style.y);
                assert!((style.x - model.x - model.w - 12.0).abs() < 0.1);
            } else {
                assert_eq!(model.x, style.x);
                assert!((style.y - model.y - model.h - 12.0).abs() < 0.1);
            }
            assert!(testing::find(&ui, Role::Link, "Effects").is_some());
            assert!(testing::find(&ui, Role::Table, "Caption commands").is_some());
        }
    }

    #[test]
    fn compact_spec_nav_keeps_link_widths_and_scrolls_sideways() {
        use pxui::Event;

        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let mut draw = |ui: &mut Ui| {
            paint(ui, None, View::Spec, Selection::default(), &[]);
        };
        testing::frame(&mut ui, &[], &mut draw);
        testing::frame(&mut ui, &[], &mut draw);
        let overview = testing::find(&ui, Role::Button, "Overview").unwrap();
        let decisions = testing::find(&ui, Role::Button, "Decisions").unwrap();
        assert!((overview.w - 68.8).abs() < 0.1, "{overview:?}");
        assert!(
            (decisions.x - overview.x - overview.w).abs() < 0.1,
            "{decisions:?}"
        );
        let title = testing::find(&ui, Role::Heading, "Splice: the main spec").unwrap();
        testing::frame(
            &mut ui,
            &[
                Event::pointer(Event::POINTER_MOVED, 0, 120.0, 16.0),
                Event::pointer(Event::SCROLLED, 0, -60.0, 0.0),
            ],
            &mut draw,
        );
        testing::frame(&mut ui, &[], &mut draw);
        let shifted = testing::find(&ui, Role::Button, "Decisions").unwrap();
        assert!(shifted.x < decisions.x, "{decisions:?} -> {shifted:?}");
        assert_eq!(
            testing::find(&ui, Role::Heading, "Splice: the main spec"),
            Some(title)
        );
    }

    #[test]
    fn overview_status_link_jumps_to_the_product_anchor() {
        use pxui::{Event, FrameIn};

        let intent = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            if let Some(selection) = paint(ui, None, View::Spec, Selection::default(), &[]) {
                intent.set(Some(selection));
            }
        };
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let input = FrameIn::new(960.0, 900.0);
        ui.frame(&input, &mut draw).unwrap();
        let link = ui
            .a11y_nodes()
            .iter()
            .find(|n| {
                n.is_role(Role::Button)
                    && ui.a11y_name_of(n) == "Agent-first and gaps"
                    && n.rect.x > 220.0
            })
            .unwrap()
            .rect;
        let [x, y] = [
            f64::from(link.x + link.w / 2.0),
            f64::from(link.y + link.h / 2.0),
        ];
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            ui.frame(&input.events(&[Event::pointer(kind, 0, x, y)]), &mut draw)
                .unwrap();
        }
        let selection = intent.get().unwrap();
        assert_eq!(SECTIONS[selection.section].0, "s-position");
        assert!(selection.scroll > 0.0);
        ui.frame(&input, &mut |ui| {
            paint(ui, None, View::Spec, selection, &[]);
        })
        .unwrap();
        let heading = testing::find(
            &ui,
            Role::Heading,
            "Agent-first: where Splice wins and where it falls short",
        )
        .unwrap();
        assert!((heading.y - 21.0).abs() <= 1.0, "{heading:?}");
    }

    #[test]
    fn spec_navigation_returns_selection_for_the_next_frame() {
        let picked = Cell::new(None);
        let selected = Cell::new(Selection::default());
        let mut frame = |ui: &mut Ui| {
            // PXUI may call the build again without events to settle layout.
            if let Some(selection) = paint(ui, None, View::Spec, selected.get(), &[]) {
                picked.set(Some(selection));
            }
        };
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        for _ in 0..2 {
            testing::frame(&mut ui, &[], &mut frame);
        }
        let rect = testing::find(&ui, Role::Button, "Decisions").unwrap();
        testing::click(
            &mut ui,
            f64::from(rect.x + rect.w * 0.5),
            f64::from(rect.y + rect.h * 0.5),
            &mut frame,
        );
        let selection = picked.get().unwrap();
        assert_eq!(selection.section, 1);
        assert!(selection.scroll > 0.0);
        selected.set(selection);
        testing::frame(&mut ui, &[], &mut frame);
        let heading = testing::find(&ui, Role::Heading, "Decisions").unwrap();
        assert!(heading.y >= 34.0 && heading.y <= 65.0, "{heading:?}");

        let mut restored = selection;
        assert!(restored.restore_scroll());
        let mut capture = crate::viewer_ui().unwrap();
        capture.set_a11y(true);
        for _ in 0..3 {
            testing::frame(&mut capture, &[], &mut |ui| {
                paint(ui, None, View::Spec, restored, &[]);
            });
        }
        assert_eq!(
            testing::find(&capture, Role::Heading, "Decisions"),
            Some(heading)
        );
    }
}
