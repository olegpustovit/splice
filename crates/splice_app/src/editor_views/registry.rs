use super::{
    ACCENT, BG, CAPTION, DOC_BODY, INK, LINE, MOTION, MUTED, NAV_LOOK, PANEL, QUIET, SAGE,
    SECTION_CAPTION, SUBHEADING, SURFACE, TABLE_EDGE, TABLE_TEXT, doc_label, model_link,
    spec_table, table_cell, table_row, table_tag,
};
use crate::commands::{self, BindingDescription};
use pxui::{
    Button, Label, Node, Role, TextSpan, Ui,
    geom::{Edges, Rgba},
    ui::{Align, Bar, Flags, Look, Size, State, States},
};
use splice_core::commands::{COMMANDS, Tier};
use splice_mcp::tool_catalog;
use std::sync::OnceLock;

mod check;
pub(crate) use check::Parity;
#[cfg(test)]
pub(crate) use check::load_sample;

const PROSE: f32 = 561.6;
const LIST_INSET: f32 = 20.0;
const DANGER: Rgba = Rgba::rgb(248, 113, 113);

fn inset(ui: &mut Ui, top: f32, bottom: f32, draw: &mut dyn FnMut(&mut Ui)) {
    ui.column(
        Node::column()
            .w(Size::Grow)
            .padding(Edges::new(0.0, top, 0.0, bottom)),
        draw,
    );
}

fn heading(ui: &mut Ui, title: &str) {
    inset(ui, 20.0, 6.0, &mut |ui| {
        doc_label(ui, title, SUBHEADING, INK)
    });
}

fn tinted(ui: &mut Ui, args: std::fmt::Arguments<'_>, label: Label, color: Rgba) {
    let handle = ui.label_fmt(args, label);
    ui.style(
        &handle,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}

struct Paragraph(&'static str, &'static str, &'static [&'static str]);

fn prose(ui: &mut Ui, width: f32, text: &Paragraph) {
    let width = width.min(PROSE).max(1.0);
    let style = DOC_BODY.w_px(width).role(Role::Paragraph);
    let Paragraph(text, lead, names) = *text;
    if lead.is_empty() && names.is_empty() {
        doc_label(ui, text, style, MUTED);
        return;
    }
    let mut spans = [TextSpan::default(); 12];
    let mut count = 0;
    if !lead.is_empty() {
        spans[count] = TextSpan::new(0, lead.len() as u32).color(INK).weight(500);
        count += 1;
    }
    let mut end = lead.len();
    for &name in names {
        let start = end
            + text[end..]
                .find(name)
                .expect("inline code in Spec paragraph");
        end = start + name.len();
        spans[count] = TextSpan::new(start as u32, end as u32).color(MOTION);
        count += 1;
    }
    let spans = &spans[..count];
    let lines = ui
        .text_service()
        .measure_spans(text, style, Some(width), spans)
        .expect("valid Spec paragraph")
        .lines;
    let handle = ui.node(
        "spec-live-paragraph",
        Node::DEFAULT
            .w_px(width)
            .h_px(lines as f32 * 19.44)
            .role(Role::Paragraph)
            .look(Look::NONE.text(MUTED)),
    );
    ui.text_spans(&handle, text, style, spans)
        .expect("valid Spec paragraph");
}

fn points(ui: &mut Ui, width: f32, entries: &[Paragraph], numbered: bool) {
    ui.column(Node::column().w(Size::Grow).role(Role::List), |ui| {
        for (index, paragraph) in entries.iter().enumerate() {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .align(Align::Start)
                    .role(Role::ListItem),
                |ui| {
                    let label = DOC_BODY.w_px(LIST_INSET).role(Role::Unknown);
                    if numbered {
                        tinted(ui, format_args!("{}.", index + 1), label, MUTED);
                    } else {
                        doc_label(ui, "•", label, MUTED);
                    }
                    prose(ui, width - LIST_INSET, paragraph);
                },
            );
        }
    });
}

fn horizontal(
    ui: &mut Ui,
    key: &'static str,
    look: Look,
    name: &'static str,
    build: &mut dyn FnMut(&mut Ui),
) {
    let area = ui.node(
        key,
        Node::column()
            .w(Size::Grow)
            .role(Role::ScrollArea)
            .look(look)
            .flags(Flags::SCROLL_X | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
            .bar(Bar::new(TABLE_EDGE, 6.0).gap(2.0)),
    );
    ui.a11y_name(&area, name);
    ui.within(&area, build);
}

fn code(ui: &mut Ui, name: &'static str, source: &str) {
    horizontal(
        ui,
        "spec-live-code",
        Look::NONE.fill(SURFACE).border(LINE, 1.0),
        name,
        &mut |ui| {
            ui.column(
                Node::column()
                    .w(Size::Fit)
                    .padding(Edges::new(13.0, 9.0, 13.0, 9.0)),
                |ui| {
                    for line in source.lines() {
                        doc_label(
                            ui,
                            line,
                            Label::MONO.size(11.0).line_height(17.6).w(Size::Fit),
                            INK,
                        );
                    }
                },
            );
        },
    );
}

fn cell(ui: &mut Ui, width: Size, text: &str, color: Rgba) {
    table_cell(ui, width, &mut |ui| doc_label(ui, text, TABLE_TEXT, color));
}

fn built() -> usize {
    COMMANDS
        .iter()
        .filter(|command| command.tier != Tier::Planned)
        .count()
}

pub(super) fn command_registry(ui: &mut Ui, width: f32, bindings: &[BindingDescription]) {
    ui.view_of("registry-command-map", &width.to_bits(), |ui| {
        code(ui, "Registry command descriptor", DESCRIPTOR);
        inset(ui, 8.0, 0.0, &mut |ui| points(ui, width, REGISTRY_RULES, false));
        inset(ui, 20.0, 6.0, &mut |ui| {
            ui.row(Node::row().w(Size::Grow).align(Align::Baseline).gap(8.0).wrap(), |ui| {
                doc_label(ui, "Command map", SUBHEADING.w(Size::Fit), INK);
                let built = built();
                tinted(
                    ui,
                    format_args!("{} commands · {} built · {} planned", COMMANDS.len(), built, COMMANDS.len() - built),
                    SUBHEADING.size(15.0).w(Size::Fit).role(Role::Unknown),
                    QUIET,
                );
            });
        });
        inset(ui, 6.0, 6.0, &mut |ui| {
            prose(ui, width, &Paragraph(
                "The running registry supplies each command's UI entry points, parameter names and risk. An asterisk marks a required parameter. Status shows the native release tier, with a dashed tag for a planned command.",
                "", &[],
            ));
        });
        let table_width = width.max(571.45);
        if table_width > width {
            horizontal(ui, "spec-command-scroll", Look::NONE, "Command map overflow", &mut |ui| {
                ui.column(Node::column().w_px(table_width), |ui| command_map(ui, table_width));
            });
        } else {
            command_map(ui, table_width);
        }
        heading(ui, "View commands");
        inset(ui, 6.0, 6.0, &mut |ui| prose(ui, width, &Paragraph(
            "These commands control the native window. For agents, ui.show sets the playhead, selection, range or zoom, and view.frames reads the rendered frames.",
            "", &["ui.show", "view.frames"],
        )));
        view_commands(ui);
    });

    heading(ui, "Keymap");
    inset(ui, 6.0, 6.0, &mut |ui| {
        prose(
            ui,
            width,
            &Paragraph(
                "The keyboard handler, shortcut dialog and this table use the effective bindings for this window, including overrides.",
                "",
                &[],
            ),
        )
    });
    keymap(ui, bindings, width);

    ui.view_of("registry-gestures", &width.to_bits(), |ui| {
        heading(ui, "Gestures");
        let gesture = Size::Px(width * 0.528);
        spec_table(
            ui,
            "Gestures",
            &[("Gesture", gesture), ("Dispatches", Size::Grow)],
            &mut |ui| {
                for &(name, dispatch) in GESTURES {
                    table_row(ui, &mut |ui| {
                        cell(ui, gesture, name, INK);
                        cell(ui, Size::Grow, dispatch, MOTION);
                    });
                }
            },
        );
    });
}

fn command_map(ui: &mut Ui, width: f32) {
    let command_width = 130.9 + (width - 571.45) * 0.029;
    let risk = 88.6;
    let status = 101.75;
    let surface = (width - command_width - risk - status) * 0.58;
    let widths = [
        Size::Px(command_width),
        Size::Px(surface),
        Size::Grow,
        Size::Px(risk),
        Size::Px(status),
    ];
    let columns = [
        ("Command", widths[0]),
        ("UI entry points", widths[1]),
        ("Params", widths[2]),
        ("Risk", widths[3]),
        ("Status", widths[4]),
    ];
    spec_table(ui, "Command map", &columns, &mut |ui| {
        let mut group = "";
        for command in COMMANDS {
            if group != command.group {
                group = command.group;
                table_row(ui, &mut |ui| {
                    table_cell(ui, Size::Grow, &mut |ui| {
                        inset(ui, 6.0, 0.0, &mut |ui| {
                            doc_label(ui, group, SECTION_CAPTION.role(Role::Heading), QUIET)
                        });
                    })
                });
            }
            table_row(ui, &mut |ui| {
                table_cell(ui, widths[0], &mut |ui| {
                    doc_label(ui, command.id, TABLE_TEXT, MOTION);
                    let style = CAPTION.line_height(15.4);
                    if command.shortcut.is_empty() {
                        doc_label(ui, command.title, style, QUIET);
                    } else {
                        tinted(
                            ui,
                            format_args!("{} · {}", command.title, command.shortcut),
                            style,
                            QUIET,
                        );
                    }
                });
                table_cell(ui, widths[1], &mut |ui| {
                    for entry in command.ui {
                        doc_label(ui, entry, TABLE_TEXT, MUTED);
                    }
                });
                table_cell(ui, widths[2], &mut |ui| {
                    if command.params.is_empty() {
                        doc_label(ui, "—", TABLE_TEXT, QUIET);
                    }
                    ui.row(Node::row().w(Size::Grow).gap(6.3).wrap(), |ui| {
                        for &(name, syntax) in command.params {
                            let required = if syntax.starts_with('!') { "*" } else { "" };
                            tinted(
                                ui,
                                format_args!("{name}{required}"),
                                TABLE_TEXT.w(Size::Fit),
                                MOTION,
                            );
                        }
                    });
                });
                cell(ui, widths[3], command.risk, MUTED);
                table_cell(ui, widths[4], &mut |ui| {
                    let planned = command.tier == Tier::Planned;
                    table_tag(
                        ui,
                        if planned {
                            "planned"
                        } else {
                            command.tier.cut()
                        },
                        MUTED,
                        TABLE_EDGE,
                        planned,
                    );
                });
            });
        }
    });
}

fn view_commands(ui: &mut Ui) {
    let id = Size::Px(192.0);
    spec_table(
        ui,
        "View commands",
        &[("ID", id), ("Meaning", Size::Grow)],
        &mut |ui| {
            if let Some(tool) = tool_catalog().iter().find(|tool| tool.name == "ui.show") {
                table_row(ui, &mut |ui| {
                    cell(ui, id, tool.name, MOTION);
                    cell(ui, Size::Grow, tool.description, MUTED);
                });
            }
            for command in commands::catalog().iter().filter(|command| {
                ["transport.", "timeline.", "ui.", "viewer."]
                    .iter()
                    .any(|prefix| command.name.starts_with(prefix))
                    && !command.name.starts_with("timeline.nudge")
                    && command.name != "ui.paste"
            }) {
                table_row(ui, &mut |ui| {
                    cell(ui, id, command.name, MOTION);
                    cell(ui, Size::Grow, command.title, MUTED);
                });
            }
        },
    );
}

fn keymap(ui: &mut Ui, bindings: &[BindingDescription], width: f32) {
    let key = Size::Px((width * 0.17).min(124.0));
    let command = Size::Px((width * 0.31).min(196.0));
    let columns = [("Keys", key), ("Action", Size::Grow), ("Command", command)];
    spec_table(ui, "Keymap", &columns, &mut |ui| {
        let catalog = commands::catalog();
        for binding in bindings.iter().filter(|binding| !binding.keys.is_empty()) {
            let id = catalog
                .iter()
                .find(|entry| entry.id == binding.id)
                .map_or("—", |entry| entry.name);
            table_row(ui, &mut |ui| {
                table_cell(ui, key, &mut |ui| {
                    let cell = ui.row(
                        Node::row().w(Size::Grow).role(Role::Cell).gap(3.0).wrap(),
                        |ui| {
                            for chord in binding.keys.split(" ? ") {
                                ui.row(
                                    Node::row()
                                        .w(Size::Fit)
                                        .min_w(16.0)
                                        .h_px(16.0)
                                        .center()
                                        .pad_x(3.0)
                                        .look(Look::NONE.fill(SURFACE).border(TABLE_EDGE, 1.0)),
                                    |ui| {
                                        doc_label(
                                            ui,
                                            chord,
                                            CAPTION
                                                .w(Size::Fit)
                                                .line_height(14.0)
                                                .role(Role::Unknown),
                                            MUTED,
                                        )
                                    },
                                );
                            }
                        },
                    );
                    ui.a11y_name(&cell, &binding.keys);
                });
                cell(ui, Size::Grow, binding.title, MUTED);
                cell(ui, command, id, MOTION);
            });
        }
    });
}

pub(super) fn mcp(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.view_of("native-mcp-surface", &width.to_bits(), |ui| {
        static INTRO: OnceLock<String> = OnceLock::new();
        let intro = INTRO.get_or_init(|| format!(
            "{} tools, listed in a fixed order so prompt caches stay warm. Timeline writes go through one tool, edit.apply, which accepts a batch of registry commands. Use commands.describe to find schemas and examples, then read timeline.snapshot to find explicit targets and their revision. After applying a batch, verify the cut with view.frames or view.check. Native results include typed fields and recoverable errors.",
            tool_catalog().len(),
        ));
        doc_label(ui, intro, DOC_BODY.w_px(width.min(PROSE)), MUTED);
        inset(ui, 6.0, 0.0, &mut |ui| tool_table(ui, width));
        for (title, list) in [("Transport and hosting", TRANSPORT), ("Addressing and time", ADDRESSING), ("Live editing, sessions and undo", SESSIONS)] {
            heading(ui, title);
            points(ui, width, list, false);
        }
    });
    inset(ui, 4.0, 0.0, &mut |ui| {
        model_link(ui, "s-improve", "Self-improvement loop", picked)
    });
    ui.view_of("native-mcp-example-errors", &width.to_bits(), |ui| {
        heading(ui, "Example: edit.apply dry run");
        inset(ui, 2.0, 0.0, &mut |ui| code(ui, "edit.apply dry-run arguments", MCP_DRY_RUN));
        inset(ui, 8.0, 0.0, &mut |ui| prose(ui, width, &Paragraph(
            "Open edit.apply in Agent · MCP. Use the projectId from project.info, the sessionId from session.start and the current ifRevision. On the sample cut the result contains a split, a new overlay and the removal of the selected title. The returned diff also shows derived changes.",
            "", &["edit.apply", "projectId", "project.info", "sessionId", "session.start", "ifRevision"],
        )));
        heading(ui, "Errors");
        inset(ui, 6.0, 6.0, &mut |ui| prose(ui, width, &Paragraph(
            "Each error contains a stable code, a message and a fix. Some include the current state (current), changes after a stale revision (changedSince), or accepted values (valid).",
            "", &["code", "message", "fix", "current", "changedSince", "valid"],
        )));
        errors(ui);
    });
}

fn tool_table(ui: &mut Ui, width: f32) {
    // v13 gives code its own font metrics. The bundled mono face needs tighter
    // spacing to match the longest ID's 97.7-point browser advance.
    const TOOL_ID: Label = TABLE_TEXT.size(10.45).tracking(-0.05);
    let mut name_width = 113.7_f32;
    for tool in tool_catalog() {
        let width = ui
            .text_service()
            .measure(tool.name, TOOL_ID, None)
            .expect("valid MCP identifier")
            .width;
        name_width = name_width.max(width + 16.05);
    }
    let name = Size::Px(name_width);
    // The hint column gets 3.93 percent of the measured table-width change.
    let hint = Size::Px((51.28 + (width - 448.0) * 0.0393).max(42.4));
    let columns = [("Tool", name), ("Hint", hint), ("Purpose", Size::Grow)];
    spec_table(ui, "MCP tools", &columns, &mut |ui| {
        for tool in tool_catalog() {
            let mode = match tool.name {
                "ui.show" => "view",
                _ if tool.read_only() => "read",
                _ => "edit",
            };
            table_row(ui, &mut |ui| {
                table_cell(ui, name, &mut |ui| {
                    doc_label(ui, tool.name, TOOL_ID, MOTION)
                });
                cell(ui, hint, mode, MUTED);
                cell(ui, Size::Grow, tool.description, MUTED);
            });
        }
    });
}

fn errors(ui: &mut Ui) {
    let code = Size::Px(135.0);
    let columns = [("Code", code), ("Meaning", Size::Grow)];
    spec_table(ui, "MCP errors", &columns, &mut |ui| {
        for &(id, meaning) in ERRORS {
            table_row(ui, &mut |ui| {
                cell(ui, code, id, MOTION);
                cell(ui, Size::Grow, meaning, MUTED);
            });
        }
    });
    doc_label(
        ui,
        "Additional native codes",
        SECTION_CAPTION.role(Role::Heading),
        QUIET,
    );
    let reference = include_str!("../../../../docs/MCP.md")
        .split("## Error codes")
        .nth(1)
        .and_then(|section| section.split("## Tests").next())
        .unwrap_or("");
    ui.row(Node::row().w(Size::Grow).gap(8.0).wrap(), |ui| {
        for (index, id) in reference.split('`').enumerate() {
            if index % 2 == 1
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                && !id.is_empty()
                && !ERRORS.iter().any(|row| row.0 == id)
                && reference.split('`').take(index).all(|seen| seen != id)
            {
                doc_label(ui, id, TABLE_TEXT.w(Size::Fit), MOTION);
            }
        }
    });
}

const RUN: States = States::of(Look::NONE.fill(INK).text(BG))
    .hover(Look::NONE.fill(SAGE).text(BG))
    .focus(Look::NONE.fill(INK).text(BG).border(ACCENT, 1.0))
    .disabled(Look::NONE.fill(PANEL).text(QUIET).border(LINE, 1.0));

pub(super) fn parity(
    ui: &mut Ui,
    width: f32,
    parity: &Parity,
    run: &mut bool,
    expanded: &mut bool,
) {
    ui.view_of("parity-spec-rules", &width.to_bits(), |ui| {
        prose(ui, width, &PARITY_LEAD);
        inset(ui, 6.0, 6.0, &mut |ui| {
            points(ui, width, PARITY_RULES, true)
        });
    });
    let running = matches!(parity, Parity::Running);
    ui.row(
        Node::row()
            .w(Size::Grow)
            .align(Align::Center)
            .gap(8.0)
            .wrap(),
        |ui| {
            let button = Button::sized(133.2, 24.0)
                .look(&RUN)
                .label(Label::MONO.size(12.0).line_height(16.8))
                .disabled(running);
            if ui.button("run-parity", "Run parity check", &button).clicked {
                *run = true;
                *expanded = false;
            }
            match parity {
                Parity::Idle => {}
                Parity::Running => doc_label(ui, "Checking the bundled sample…", CAPTION, QUIET),
                Parity::Complete(report) => {
                    if let Some(revision) = report.revision {
                        tinted(
                            ui,
                            format_args!(
                                "Checked sample r{revision} in {} ms",
                                report.milliseconds
                            ),
                            CAPTION.w(Size::Fit),
                            QUIET,
                        );
                    } else {
                        doc_label(
                            ui,
                            "Could not load the bundled sample",
                            CAPTION.w(Size::Fit),
                            DANGER,
                        );
                    }
                }
            }
        },
    );
    if let Parity::Complete(report) = parity {
        let passed = report.passed();
        let failures = report.examples.len() - passed + report.problems.len();
        ui.row(Node::row().w(Size::Grow).gap(8.0).wrap(), |ui| {
            metric(
                ui,
                120.0,
                format_args!("{}", if failures == 0 { "PASS" } else { "FAIL" }),
                format_args!("{failures} problem(s)"),
                if failures == 0 { INK } else { DANGER },
            );
            metric(
                ui,
                140.0,
                format_args!("{}", report.keymap + GESTURES.len()),
                format_args!("UI bindings checked"),
                INK,
            );
            metric(
                ui,
                120.0,
                format_args!("{passed}/{}", report.examples.len()),
                format_args!("Core dry runs"),
                INK,
            );
            let built = built();
            metric(
                ui,
                170.0,
                format_args!("{built}+{}", COMMANDS.len() - built),
                format_args!("built + planned commands"),
                INK,
            );
        });
        inset(ui, 14.0, 6.0, &mut |ui| {
            tinted(
                ui,
                format_args!(
                    "Bindings by surface · keys {} · gestures {} · MCP schemas {}",
                    report.keymap,
                    GESTURES.len(),
                    report.tools,
                ),
                DOC_BODY,
                QUIET,
            )
        });
        if failures != 0 {
            problems(ui, report);
        }
        let result_button = Button::DEFAULT
            .size(Size::Grow, Size::Px(20.0))
            .centered()
            .content_align(pxui::ui::Along::Start)
            .label(Label::MONO.size(12.0))
            .look(&NAV_LOOK)
            .selected(*expanded);
        let label = if *expanded {
            "▾  edit.apply fixture results"
        } else {
            "▸  edit.apply fixture results"
        };
        if ui
            .button(
                "parity-details",
                label,
                &result_button.name("edit.apply fixture results"),
            )
            .clicked
        {
            *expanded = !*expanded;
        }
        if *expanded {
            results(ui, width, report);
        }
    }
}

fn metric(
    ui: &mut Ui,
    width: f32,
    value: std::fmt::Arguments<'_>,
    caption: std::fmt::Arguments<'_>,
    ink: Rgba,
) {
    ui.column(
        Node::column()
            .w_px(width)
            .h_px(59.8)
            .padding(Edges::new(12.0, 6.0, 12.0, 6.0))
            .look(Look::NONE.fill(PANEL).border(LINE, 1.0)),
        |ui| {
            tinted(
                ui,
                value,
                Label::BODY.font(1).size(24.0).weight(300).line_height(33.6),
                ink,
            );
            tinted(ui, caption, CAPTION.line_height(14.0), QUIET);
        },
    );
}

fn problems(ui: &mut Ui, report: &check::Report) {
    let rule = Size::Px(48.0);
    let surface = Size::Px(166.0);
    let columns = [("Rule", rule), ("Where", surface), ("Problem", Size::Grow)];
    spec_table(ui, "Parity problems", &columns, &mut |ui| {
        for p in &report.problems {
            table_row(ui, &mut |ui| {
                cell(ui, rule, p.rule, INK);
                table_cell(ui, surface, &mut |ui| {
                    tinted(
                        ui,
                        format_args!("{} · {}", p.surface, p.name),
                        TABLE_TEXT,
                        MUTED,
                    );
                });
                cell(ui, Size::Grow, p.detail, MUTED);
            });
        }
        for result in &report.examples {
            if let Err((code, message)) = &result.result {
                table_row(ui, &mut |ui| {
                    cell(ui, rule, "2", INK);
                    table_cell(ui, surface, &mut |ui| {
                        tinted(
                            ui,
                            format_args!("edit.apply · {}", result.command),
                            TABLE_TEXT,
                            MUTED,
                        );
                    });
                    table_cell(ui, Size::Grow, &mut |ui| {
                        doc_label(ui, code, TABLE_TEXT, DANGER);
                        doc_label(ui, message, TABLE_TEXT, MUTED);
                    });
                });
            }
        }
    });
}

fn results(ui: &mut Ui, width: f32, report: &check::Report) {
    let id = Size::Px(width * 0.26);
    let effects = Size::Px((width * 0.16).max(84.0));
    let columns = [
        ("Command", id),
        ("Result", Size::Grow),
        ("Side effects", effects),
    ];
    spec_table(ui, "edit.apply fixture results", &columns, &mut |ui| {
        for row in &report.examples {
            table_row(ui, &mut |ui| {
                cell(ui, id, row.command, MOTION);
                match &row.result {
                    Ok((summary, count)) => {
                        cell(ui, Size::Grow, summary, MUTED);
                        table_cell(ui, effects, &mut |ui| {
                            tinted(ui, format_args!("{count}"), TABLE_TEXT, MUTED)
                        });
                    }
                    Err((code, message)) => table_cell(ui, Size::Grow, &mut |ui| {
                        doc_label(ui, code, TABLE_TEXT, DANGER);
                        doc_label(ui, message, TABLE_TEXT, MUTED);
                    }),
                }
            });
        }
    });
}

#[cfg(test)]
mod tests;

const DESCRIPTOR: &str = r#"Command {
  id:        "item.split"          // stable dotted wire name
  title:     "Split at Playhead"   // label shown in the editor
  shortcut:  "C"
  params:    schema                // explicit targets and frames
  risk:      edit | destructive
  ui:        menus[], inspector fields[], gestures[]
  example:   { … }                 // decoded and tested on the sample cut
}"#;

const REGISTRY_RULES: &[Paragraph] = &[
    Paragraph(
        "The UI resolves context into explicit arguments. Split at Playhead with a selection dispatches item.split {item:\"clp_take\", at:372}. The core never reads the selection, the playhead or a mode.",
        "The UI resolves context into explicit arguments.",
        &["item.split {item:\"clp_take\", at:372}"],
    ),
    Paragraph(
        "Modes are parameters. Magnetic becomes ripple:true|false. Snapping resolves to exact frames. The grave key sets carryOverlays:false when moving a spine clip.",
        "Modes are parameters.",
        &["ripple:true|false", "carryOverlays:false"],
    ),
    Paragraph(
        "One transaction per dispatch. A completed drag, a menu edit and an edit.apply batch each produce one undo step with a generated title.",
        "One transaction per dispatch.",
        &["edit.apply"],
    ),
    Paragraph(
        "Deterministic IDs. The engine mints IDs inside the transaction and records them with the operation. Replaying the journal uses the recorded IDs to reproduce the same edit.",
        "Deterministic IDs.",
        &[],
    ),
    Paragraph(
        "Refs. An operation can name its result (ref:\"$cut\"). Later operations in the batch can use \"$cut\" or \"$cut.right\".",
        "Refs.",
        &["ref:\"$cut\"", "\"$cut\"", "\"$cut.right\""],
    ),
];

const GESTURES: &[(&str, &str)] = &[
    (
        "Drag spine clip",
        "item.move {items, to, ripple, carryOverlays}",
    ),
    (
        "Drag overlay horizontally",
        "item.move (re-anchors at its new start)",
    ),
    ("Drag overlay vertically", "item.lane {items, lane}"),
    ("Drag edge grip", "item.trim {item, edge, delta, ripple}"),
    ("Drag fade handle", "item.set {items, fadeIn|fadeOut}"),
    ("Drag marker", "marker.set {marker, at}"),
    ("Blade click", "item.split {item, at}"),
    ("Click component eye", "component.set"),
    ("Click keystroke checkbox", "keys.hide"),
    ("Rename lane in Inspector", "lane.rename"),
    (
        "Drag crop frame or handles, then Apply",
        "clip.crop {items, rect, fit}",
    ),
    (
        "Pinch / ⌘-scroll / drag in viewer",
        "ui.show (corresponding view state)",
    ),
    (
        "Drag effect onto clip, overlay, lane header or viewer",
        "effect.add {target, type}",
    ),
    ("Click lane name", "ui.show (corresponding selection state)"),
    (
        "Drag ruler / playhead",
        "ui.show (corresponding playhead state)",
    ),
    ("Marquee", "ui.show (corresponding selection state)"),
    (
        "Wheel / pinch",
        "ui.show (corresponding timeline view state)",
    ),
];

const TRANSPORT: &[Paragraph] = &[
    Paragraph(
        "No MCP SDK. The native server reads newline-delimited JSON-RPC 2.0. It supports the 2025-11-25 initialization handshake and 2026-07-28 stateless requests with the protocol version in params._meta.",
        "No MCP SDK.",
        &["params._meta"],
    ),
    Paragraph(
        "Same binary, stdio shim. splice mcp speaks stdio to the client and forwards to a running viewer over a same-user Unix socket in the state directory. When no viewer has the project open, splice mcp can hold the writer lock and serve it headless.",
        "Same binary, stdio shim.",
        &["splice mcp", "splice mcp"],
    ),
    Paragraph(
        "Headless mode. The CLI and MCP integration tests load packages without a window. The core handles the same IDs, edits, revisions and undo as the editor.",
        "Headless mode.",
        &[],
    ),
    Paragraph(
        "Results. Tool responses have structuredContent that follows outputSchema, with a text mirror. Business errors carry isError:true and an actionable fix.",
        "Results.",
        &["structuredContent", "outputSchema", "isError:true", "fix"],
    ),
    Paragraph(
        "Long jobs. Export, transcription and recording can return a plain jobId. Read progress with jobs.status and request cancellation with jobs.cancel.",
        "Long jobs.",
        &["jobId", "jobs.status", "jobs.cancel"],
    ),
    Paragraph(
        "Notifications. Export clients can send a progress token to receive notifications/progress. Use history.list to read project revisions and their sources.",
        "Notifications.",
        &["notifications/progress", "history.list"],
    ),
];

const ADDRESSING: &[Paragraph] = &[
    Paragraph(
        "Stable prefixed IDs. clp_ clips, ovl_ overlays, lan_ lanes, mrk_ markers, med_ media, ses_ sessions, job_ jobs and components such as clp_take/mic.",
        "Stable prefixed IDs.",
        &[
            "clp_",
            "ovl_",
            "lan_",
            "mrk_",
            "med_",
            "ses_",
            "job_",
            "clp_take/mic",
        ],
    ),
    Paragraph(
        "Frames. Timeline coordinates use integer frames at the fps from project.info. sourceFrame is measured on the source clock. Invalid values return BAD_ARGS with a fix hint.",
        "Frames.",
        &["fps", "project.info", "sourceFrame", "BAD_ARGS"],
    ),
    Paragraph(
        "Anchor forms. Commands that take a frame anchor can resolve {marker:\"mrk_2\", offset}, {item:\"clp_take\", edge:\"start\", offset} or a word ID with transcriptHash. Use \"$ref\" for an item created earlier in the batch.",
        "Anchor forms.",
        &[
            "{marker:\"mrk_2\", offset}",
            "{item:\"clp_take\", edge:\"start\", offset}",
            "transcriptHash",
            "\"$ref\"",
        ],
    ),
    Paragraph(
        "Reads return IDs and human labels. timeline.snapshot includes resolved ranges, and detailed reads include the anchors and fields that determine them.",
        "Reads return IDs and human labels.",
        &["timeline.snapshot"],
    ),
];

const SESSIONS: &[Paragraph] = &[
    Paragraph(
        "Access per project. The human can set off, read or live access in the Canvas inspector. live is the default.",
        "Access per project.",
        &["off", "read", "live", "live"],
    ),
    Paragraph(
        "Sessions. session.start returns a server-minted sessionId and a checkpoint revision. Edits include that sessionId.",
        "Sessions.",
        &["session.start", "sessionId", "sessionId"],
    ),
    Paragraph(
        "Every batch commits live. The history step is named Agent: <summary>. The native activity panel lists batches and their Undo actions.",
        "Every batch commits live.",
        &["Agent: <summary>"],
    ),
    Paragraph(
        "Undo scopes. In history.undo use last for the last batch, batch with a revision for one batch, session for the whole session, or to with a revision to restore a prior state. Selective undo replays later edits and lists any that can no longer apply.",
        "Undo scopes.",
        &["history.undo", "last", "batch", "session", "to"],
    ),
    Paragraph(
        "Optimistic concurrency. Include ifRevision on an edit. STALE_REVISION returns the current revision and change context so the agent can re-read the project.",
        "Optimistic concurrency.",
        &["ifRevision", "STALE_REVISION"],
    ),
    Paragraph(
        "Dry run. dryRun:true returns changes, side effects and a diff on the current cut without committing.",
        "Dry run.",
        &["dryRun:true"],
    ),
    Paragraph(
        "Retries. A stable idempotencyKey identifies a batch if a client needs to retry it.",
        "Retries.",
        &["idempotencyKey"],
    ),
    Paragraph(
        "Export and recording. Export writes to <project>.splice/exports/ by default and requires overwrite:true to replace a file. record.start and record.stop control headless recording.",
        "Export and recording.",
        &[
            "<project>.splice/exports/",
            "overwrite:true",
            "record.start",
            "record.stop",
        ],
    ),
    Paragraph(
        "Friction. Attach a friction note to any tool call. Failed calls, friction notes and agent undo feed the self-improvement loop.",
        "Friction.",
        &["friction"],
    ),
];

const MCP_DRY_RUN: &str = r#"{
  "projectId": "prj_1", "sessionId": "ses_1", "ifRevision": "r0", "dryRun": true,
  "ops": [
    { "cmd": "item.split", "item": "clp_take", "at": 372, "ref": "$cut" },
    { "cmd": "overlay.add", "kind": "media", "media": "med_broll",
      "at": { "item": "$cut.right", "edge": "start" }, "length": 90 },
    { "cmd": "item.delete", "items": ["ovl_label"], "ripple": false }
  ]
}"#;

const ERRORS: &[(&str, &str)] = &[
    (
        "STALE_REVISION",
        "ifRevision differs from the current revision. The reply includes the current revision and a summary of what changed.",
    ),
    (
        "NOT_FOUND",
        "The requested ID or named resource does not exist.",
    ),
    (
        "WRONG_KIND",
        "The operation expects a different kind of timeline item, lane or component.",
    ),
    (
        "OUT_OF_SOURCE",
        "The trim, move or extension exceeds the available source frames.",
    ),
    (
        "TOO_SHORT",
        "The result would be shorter than the minimum duration for that item.",
    ),
    (
        "OVERLAP",
        "The requested edit would overlap an item that must occupy separate frames.",
    ),
    (
        "LANE_CONFLICT",
        "An explicitly selected lane is occupied at the requested range.",
    ),
    (
        "LANE_NOT_EMPTY",
        "Deleting the lane would leave its items without a compatible lane.",
    ),
    (
        "TRANSCRIPT_CHANGED",
        "The supplied transcript hash is stale. Read the words and hash again.",
    ),
    (
        "BAD_ARGS",
        "An argument is missing, has the wrong type or falls outside its accepted range.",
    ),
    (
        "UNKNOWN_COMMAND",
        "The command ID is unknown. Inspect the registry for valid names.",
    ),
    (
        "PLANNED",
        "This registry command is scheduled for a later release.",
    ),
    (
        "EXISTS",
        "The requested output or package path is already in use.",
    ),
    (
        "BUSY",
        "The operation conflicts with a job or writer that is still active.",
    ),
    (
        "PERMISSION_DENIED",
        "The project access level refuses the requested tool operation.",
    ),
    (
        "NO_SESSION",
        "The tool requires an open agent session from session.start.",
    ),
];

const PARITY_LEAD: Paragraph = Paragraph(
    "The build checks for drift between editor and agent tools. Run the native registry, effective keymap and MCP schema checks here. Each built command's example stages a dry run against the bundled sample cut. The report includes results and side effects.",
    "",
    &[],
);

const PARITY_RULES: &[Paragraph] = &[
    Paragraph(
        "Every effective shortcut and documented gesture names a native registry command, window command or view tool.",
        "",
        &[],
    ),
    Paragraph(
        "Every built command example runs through the Core edit.apply path as a dry run against the same clean sample cut.",
        "",
        &["edit.apply"],
    ),
    Paragraph(
        "Every built registry entry documents UI access beyond the command palette.",
        "",
        &[],
    ),
    Paragraph(
        "Commands have IDs, titles and examples. Tool schemas declare required fields.",
        "",
        &[],
    ),
];
