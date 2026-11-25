use super::{
    ACCENT, BG, CAPTION, DOC_BODY, DOC_LEDE, HEADING, INK, MOTION, MUTED, ModelPoint, QUIET, SAGE,
    SUBHEADING, TABLE_EDGE, TABLE_HEAD, TABLE_LINE, TABLE_ROW, TABLE_TEXT, doc_label, model_cards,
    model_list, spec_table_width, table_cell, table_row, table_tag,
};
use pxui::{
    Label, Node, Role, TextSpan, Ui,
    draw::flags::rect::{BOTTOM, LEFT},
    geom::{Edges, Rgba},
    ui::{Align, Bar, Flags, Look, Size, State, States},
};

const POSITION: &str = "Agent-first: where Splice wins and where it falls short";
const IMPROVE: &str = "Self-improvement";
const FIRST_CARD: &str = "Where an MCP-first core wins";
const SECOND_CARD: &str = "Where it risks being a gimmick, and the counter";
const SESSION: &str = "Inside a session: the agent self-corrects";
const LOOP: &str = "Across sessions: the loop fixes the cause";
const PRIORITIES_TITLE: &str = r#"The 15 things that let Splice say "better""#;
const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const WARN: Rgba = Rgba::rgb(216, 200, 106);
const BARLEY: Rgba = Rgba::rgb(236, 230, 206);
const MIN_GAP_WIDTH: f32 = 675.44;
const COLUMN: Node = Node::column().w(Size::Grow);
type Text = (&'static str, &'static [TextSpan]);

pub(super) fn document(
    ui: &mut Ui,
    anchor: &str,
    narrow: bool,
    picked: &mut Option<usize>,
) -> bool {
    match anchor {
        "s-position" => {
            let width = spec_table_width(ui, narrow);
            heading(ui, width, POSITION, "proposed", MOTION);
            position(ui, width, narrow, picked);
        }
        "s-improve" => {
            let width = spec_table_width(ui, narrow);
            heading(ui, width, IMPROVE, "locked", SAGE);
            improve(ui, width, narrow, picked);
        }
        _ => return false,
    }
    true
}

fn heading(ui: &mut Ui, available: f32, text: &str, status: &str, color: Rgba) {
    let line = HEADING.w(Size::Fit).role(Role::Unknown);
    let badge = line.size(10.0).line_height(16.0);
    let mut measure = ui.text_service();
    let full = measure
        .measure(text, line, None)
        .expect("valid product heading")
        .width;
    let used = measure
        .measure(status, badge, None)
        .expect("valid heading badge")
        .width
        + 12.0
        + 8.0;
    // Keep the badge with the last title line, as the browser's inline badge does.
    let split = if full + used > available {
        text.rmatch_indices(' ')
            .find(|&(at, _)| {
                measure
                    .measure(&text[at + 1..], line, None)
                    .expect("valid heading line")
                    .width
                    + used
                    <= available
                    && measure
                        .measure(&text[..at], line, None)
                        .expect("valid heading line")
                        .width
                        <= available
            })
            .map(|(at, _)| at)
    } else {
        None
    };
    let (first, last) = match split {
        Some(at) => (&text[..at], &text[at + 1..]),
        None => ("", text),
    };
    let header = ui.column(COLUMN.role(Role::Heading), |ui| {
        if !first.is_empty() {
            doc_label(ui, first, line, INK);
        }
        ui.row(
            Node::row()
                .w(Size::Grow)
                .gap(8.0)
                .align(Align::Start)
                .wrap(),
            |ui| {
                let style = if split.is_none() && full > available {
                    line.w_px(available)
                } else {
                    line
                };
                doc_label(ui, last, style, INK);
                ui.column(
                    COLUMN.w(Size::Fit).padding(Edges::new(0.0, 17.0, 0.0, 0.0)),
                    |ui| {
                        ui.row(
                            Node::row()
                                .w(Size::Fit)
                                .h_px(18.0)
                                .pad_x(6.0)
                                .align(Align::Center)
                                .look(Look::NONE.border(color, 1.0)),
                            |ui| doc_label(ui, status, badge, color),
                        );
                    },
                );
            },
        );
    });
    ui.a11y_name(&header, text);
}

fn position(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(
        COLUMN.gap(20.0).padding(Edges::new(0.0, 0.0, 0.0, 6.0)),
        |ui| {
            ui.column(COLUMN.gap(8.0), |ui| {
                lede(ui, width, POSITION_LEDE);
                let callout_width = width.min(576.0);
                let box_ = ui.column(
                    COLUMN
                        .w_px(callout_width)
                        .padding(Edges::new(14.0, 8.0, 12.0, 8.0))
                        .role(Role::Group)
                        .look(Look::NONE.fill(CONTROL).border(ACCENT, 2.0).sides(LEFT)),
                    |ui| {
                        rich(
                            ui,
                            (callout_width - 26.0).max(1.0),
                            MARKET,
                            DOC_BODY.line_height(16.8),
                            Role::Paragraph,
                            16.8,
                            MUTED,
                        )
                    },
                );
                ui.a11y_name(&box_, "September 2026 market research");
                model_cards(
                    ui,
                    width,
                    narrow,
                    [(FIRST_CARD, WINS), (SECOND_CARD, COUNTERS)],
                    picked,
                );
            });
            ui.column(COLUMN.gap(6.0), |ui| {
                doc_label(ui, "Gap matrix", SUBHEADING, INK);
                rich(
                    ui,
                    width.min(561.6),
                    GAP_INTRO,
                    DOC_BODY,
                    Role::Paragraph,
                    19.44,
                    MUTED,
                );
                gap_matrix(ui, width);
            });
            ui.column(COLUMN.gap(6.0), |ui| {
                doc_label(ui, PRIORITIES_TITLE, SUBHEADING, INK);
                priorities(ui, width);
            });
            ui.column(COLUMN.gap(6.0), |ui| {
                doc_label(ui, "Risks", SUBHEADING, INK);
                model_list(ui, width, RISKS, picked);
            });
        },
    );
}

fn improve(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(COLUMN.padding(Edges::new(0.0, 0.0, 0.0, 6.0)), |ui| {
        ui.column(COLUMN.gap(8.0), |ui| {
            lede(ui, width, IMPROVE_LEDE);
            model_cards(
                ui,
                width,
                narrow,
                [(SESSION, SESSION_POINTS), (LOOP, LOOP_POINTS)],
                picked,
            );
        });
        ui.view_of("friction-table", &width.to_bits(), |ui| {
            friction_table(ui, width)
        });
        ui.column(COLUMN.padding(Edges::new(0.0, 6.0, 0.0, 0.0)), |ui| {
            rich(
                ui,
                width.min(561.6),
                EVALS,
                DOC_BODY,
                Role::Paragraph,
                19.44,
                MUTED,
            );
        });
    });
}

fn lede(ui: &mut Ui, width: f32, text: Text) {
    rich(
        ui,
        width.min(624.0),
        text,
        DOC_LEDE,
        Role::Paragraph,
        21.06,
        INK,
    );
}

fn rich(
    ui: &mut Ui,
    width: f32,
    (text, spans): Text,
    style: Label,
    role: Role,
    line: f32,
    color: Rgba,
) {
    let style = style.role(role);
    if spans.is_empty() {
        doc_label(ui, text, style.w_px(width), color);
        return;
    }
    let lines = ui
        .text_service()
        .measure_spans(text, style, Some(width), spans)
        .expect("valid product document spans")
        .lines;
    let label = ui.node(
        "product-prose",
        Node::DEFAULT
            .w_px(width)
            .h_px(lines as f32 * line)
            .role(role)
            .look(Look::NONE.text(color)),
    );
    ui.text_spans(&label, text, style, spans)
        .expect("valid product document spans");
}

fn priorities(ui: &mut Ui, width: f32) {
    let width = (width - 20.0).clamp(1.0, 561.6);
    let list = ui.column(COLUMN.role(Role::List), |ui| {
        for (index, &text) in PRIORITIES.iter().enumerate() {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .align(Align::Start)
                    .role(Role::ListItem),
                |ui| {
                    let label = ui.label_fmt(
                        format_args!("{}.", index + 1),
                        DOC_BODY.w_px(20.0).size(11.0).role(Role::Unknown),
                    );
                    tint(ui, &label, MUTED);
                    rich(ui, width, text, DOC_BODY, Role::Paragraph, 19.44, MUTED);
                },
            );
        }
    });
    ui.a11y_name(&list, "V13 priorities");
}

fn gap_matrix(ui: &mut Ui, width: f32) {
    let full = width.max(MIN_GAP_WIDTH);
    ui.column(COLUMN.pad_y(8.0), |ui| {
        if full > width {
            // Preserve the browser table's minimum content width on a compact page.
            let viewport = ui.node(
                "gap-scroll",
                COLUMN
                    .role(Role::ScrollArea)
                    .flags(Flags::SCROLL_X | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
                    .bar(Bar::new(TABLE_EDGE, 6.0).gap(2.0)),
            );
            ui.a11y_name(&viewport, "Gap matrix columns");
            ui.within(&viewport, &mut |ui| {
                ui.column(COLUMN.w_px(full), |ui| gap_table(ui, full));
            });
        } else {
            gap_table(ui, full);
        }
    });
}

fn gap_table(ui: &mut Ui, width: f32) {
    ui.view_of("gap-matrix-table", &width.to_bits(), |ui| {
        // The desktop CSS table shares extra width across the historical comparison columns.
        let extra = (width - MIN_GAP_WIDTH) / (768.0 - MIN_GAP_WIDTH);
        let columns = [
            ("Capability", 108.41 + 46.28 * extra),
            ("Resolve", 68.81 + 18.50 * extra),
            ("FCP", 75.41 + 9.25 * extra),
            ("Premiere", 75.41 + 8.32 * extra),
            ("CapCut / Descript", 75.40 + 10.18 * extra),
            ("Splice", 82.00),
            ("Now", 190.00 + 0.03 * extra),
        ];
        table(ui, "Gap matrix", width, &columns, &mut |ui| {
            for (index, row) in GAPS.iter().enumerate() {
                table_row(ui, &mut |ui| {
                    for (col, &text) in row.comparison.iter().enumerate() {
                        cell(
                            ui,
                            columns[col].1,
                            (text, &[]),
                            if col == 0 { INK } else { MUTED },
                        );
                    }
                    table_cell(ui, Size::Px(columns[5].1), &mut |ui| {
                        let color = if row.priority.starts_with("P0") {
                            SAGE
                        } else if row.priority == "P1" {
                            MOTION
                        } else {
                            WARN
                        };
                        table_tag(ui, row.priority, color, color, false);
                    });
                    table_cell(ui, Size::Px(columns[6].1), &mut |ui| {
                        if index == 0 {
                            tool_count(ui);
                        } else {
                            let color = if row.shipped { SAGE } else { MUTED };
                            let edge = if row.shipped { SAGE } else { TABLE_EDGE };
                            table_tag(ui, row.native, color, edge, false);
                        }
                    });
                });
            }
        });
    });
}

fn tool_count(ui: &mut Ui) {
    ui.row(
        Node::row()
            .w(Size::Fit)
            .h_px(18.0)
            .pad_x(6.0)
            .align(Align::Center)
            .look(Look::NONE.border(SAGE, 1.0)),
        |ui| {
            let label = ui.label_fmt(
                format_args!("{} tools, parity", splice_mcp::tool_catalog().len()),
                CAPTION.w(Size::Fit).line_height(16.0).role(Role::Cell),
            );
            tint(ui, &label, SAGE);
        },
    );
}

fn friction_table(ui: &mut Ui, width: f32) {
    let scale = (width / 448.0).min(1.0);
    let extra = (width - 448.0).max(0.0);
    let cause = 123.03 * scale + extra * 0.1706;
    let test = 118.05 * scale + extra * 0.17825;
    let fix = width - cause - test;
    let columns = [("Cause", cause), ("Fix", fix), ("Test", test)];
    ui.column(COLUMN.pad_y(8.0), |ui| {
        table(ui, "Friction", width, &columns, &mut |ui| {
            for &row in FRICTION {
                table_row(ui, &mut |ui| {
                    cell(ui, cause, row[0], INK);
                    cell(ui, fix, row[1], MUTED);
                    cell(ui, test, row[2], MUTED);
                });
            }
        });
    });
}

fn table(
    ui: &mut Ui,
    name: &str,
    width: f32,
    columns: &[(&str, f32)],
    rows: &mut dyn FnMut(&mut Ui),
) {
    let tab = ui.node("product-table", COLUMN.w_px(width).role(Role::Table));
    ui.a11y_name(&tab, name);
    ui.within(&tab, &mut |ui| {
        ui.row(
            TABLE_ROW
                .padding(Edges::new(0.0, 6.5, 0.0, 6.0))
                .look(Look::NONE.fill(BG).border(TABLE_EDGE, 1.0).sides(BOTTOM)),
            |ui| {
                for &(column, w) in columns {
                    table_cell(ui, Size::Px(w), &mut |ui| {
                        let label = ui.label(column, TABLE_HEAD);
                        ui.a11y_name(&label, column);
                        tint(ui, &label, QUIET);
                    });
                }
            },
        );
        rows(ui);
    });
}

fn cell(ui: &mut Ui, width: f32, content: Text, color: Rgba) {
    table_cell(ui, Size::Px(width), &mut |ui| {
        rich(
            ui,
            (width - 16.0).max(1.0),
            content,
            TABLE_TEXT,
            Role::Cell,
            TABLE_LINE,
            color,
        )
    });
}

fn tint(ui: &mut Ui, handle: &pxui::Handle, color: Rgba) {
    ui.style(
        handle,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}
const POSITION_LEDE: Text = (
    "The Rust core and its MCP surface own the edit. The UI adds direct manipulation, with editing gestures sent through the same command registry. Humans and agents work on one project, and both can inspect the timeline and verify the result of their edits.",
    &[],
);

const MARKET: Text = (
    "V13 research, September 2026. The notes reported that DaVinci Resolve Studio 21.1 had introduced an MCP server for Claude, Claude Code and Codex along with about 20 scripting APIs, citing RedShark and Digital Production. A Primal Video trial described a 24-minute recording reduced to seven minutes in about 15 minutes at $13–40 in tokens. Quality improved with a better transcript, and the resulting cut still needed a human pass. The v13 source review found no documented permissions, edit safeguards or audit controls. The review identified the quality of the edit contract as the opportunity for Splice.",
    &[TextSpan::new(0, 29).weight(500).color(INK)],
);

const GAP_INTRO: Text = (
    "V13 compared Resolve 20–21.1, FCP 11–12, Premiere 26, After Effects, CapCut, Descript, Submagic and Screen Studio. Splice marks v13 priorities. Now shows native support. The numbered goals are v13’s roadmap.",
    &[
        TextSpan::new(119, 125).weight(500).color(INK),
        TextSpan::new(148, 151).weight(500).color(INK),
    ],
);

const IMPROVE_LEDE: Text = (
    "Agents commit edits live and own their undo history. When an agent struggles, Splice records the friction. The build loop turns repeated problems into fixes and verifies them with regressions. The repo records the loop in docs/LOOP.md and the splice skill in .agents/skills/.",
    &[
        TextSpan::new(222, 234).color(MOTION),
        TextSpan::new(243, 249).color(MOTION),
        TextSpan::new(259, 274).color(MOTION),
    ],
);

const EVALS: Text = (
    "Evals. make eval runs natural-language tasks on seed projects through Codex or Claude with the skill and headless splice mcp. It uses splice check to verify the result and records passes, calls, errors, recoveries and undos. A drop from baseline fails the loop check.",
    &[
        TextSpan::new(0, 6).weight(500).color(INK),
        TextSpan::new(7, 16).color(MOTION),
        TextSpan::new(114, 124).color(MOTION),
        TextSpan::new(134, 146).color(MOTION),
    ],
);

const WINS: &[ModelPoint] = &[
    ModelPoint {
        text: "Every edit is reachable. Editing gestures and MCP use the registry. Native parity checks examples, tool schemas and window shortcuts.",
        spans: &[TextSpan::new(0, 24).weight(500).color(INK)],
        link: Some(("s-parity", "parity check")),
    },
    ModelPoint {
        text: "Transactions. Dry-run with a diff and ifRevision. A batch commits as one undo step. Session undo replays later human edits.",
        spans: &[
            TextSpan::new(0, 13).weight(500).color(INK),
            TextSpan::new(38, 48).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Data an agent can read. Clicks, cursor paths, keystrokes, words and silences support recipes such as zooming into a take’s click ranges.",
        spans: &[TextSpan::new(0, 23).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Deterministic and replayable. Stable IDs, integer frames and word anchors make an edit into a recipe agents can replay across projects.",
        spans: &[TextSpan::new(0, 29).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Trust through the edit history. Per-project off, read or live access, a fsynced journal, revision or session undo, and agent badges show what changed.",
        spans: &[TextSpan::new(0, 31).weight(500).color(INK)],
        link: None,
    },
];

const COUNTERS: &[ModelPoint] = &[
    ModelPoint {
        text: "Chat is slower for small edits → keep the ⌘K palette, shortcuts and direct manipulation.",
        spans: &[TextSpan::new(44, 48).color(BARLEY)],
        link: None,
    },
    ModelPoint {
        text: "Token cost and unfinished cuts → deterministic intent.* recipes with optional model planning.",
        spans: &[TextSpan::new(49, 57).color(MOTION)],
        link: None,
    },
    ModelPoint {
        text: "An agent needs to see its result → view.frames, view.check, audio.summary and takes.events help verify it.",
        spans: &[
            TextSpan::new(37, 48).color(MOTION),
            TextSpan::new(50, 60).color(MOTION),
            TextSpan::new(62, 75).color(MOTION),
            TextSpan::new(80, 92).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Generic-looking output → presets, brand kits and templates with exposed fields.",
        spans: &[],
        link: Some(("s-motion", "Motion graphics")),
    },
    ModelPoint {
        text: "Weak transcripts → v13 proposed forced alignment and provider choice. Native uses SpeechAnalyzer.",
        spans: &[],
        link: Some(("s-captions", "Captions")),
    },
];

const RISKS: &[ModelPoint] = &[
    ModelPoint {
        text: "The incumbent caught up. Resolve’s MCP release raised the bar for Splice’s API coverage, undo contract and recorded screen data.",
        spans: &[TextSpan::new(0, 24).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Transcripts cap agent quality. V13 proposed comparing SpeechAnalyzer, WhisperKit and Deepgram for word timing and fillers.",
        spans: &[TextSpan::new(0, 30).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Motion graphics scope creep. V13 focused on 2D text, shapes, masks and tracking. The native compositor includes 2D particles.",
        spans: &[TextSpan::new(0, 28).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "macOS media coverage. V13 flagged MKV, VP9 and AV1 decode coverage for verification with AVFoundation.",
        spans: &[TextSpan::new(0, 21).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Pressure from both sides. V13 aimed for editing depth plus polished screen delivery and transcript editing.",
        spans: &[TextSpan::new(0, 25).weight(500).color(INK)],
        link: Some(("s-roadmap", "Roadmap")),
    },
];

const SESSION_POINTS: &[ModelPoint] = &[
    ModelPoint {
        text: "Errors are results with a code and literal fix, plus valid or current where useful. On STALE_REVISION, read the latest state and rebase the intended edit.",
        spans: &[
            TextSpan::new(26, 30).color(MOTION),
            TextSpan::new(43, 46).color(MOTION),
            TextSpan::new(53, 58).color(MOTION),
            TextSpan::new(62, 69).color(MOTION),
            TextSpan::new(87, 101).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Use dryRun before a risky batch. Verify with view.frames and timeline.snapshot after it commits.",
        spans: &[
            TextSpan::new(4, 10).color(MOTION),
            TextSpan::new(45, 56).color(MOTION),
            TextSpan::new(61, 78).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Use history.undo with scope set to last, session or to a trusted revision when a result is wrong. Undo appends a new revision.",
        spans: &[
            TextSpan::new(4, 16).color(MOTION),
            TextSpan::new(22, 27).color(MOTION),
            TextSpan::new(35, 39).color(MOTION),
            TextSpan::new(41, 48).color(MOTION),
            TextSpan::new(52, 54).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "When a call was harder than expected, attach friction:\"expected X, got Y\" to the next tool call.",
        spans: &[TextSpan::new(45, 73).color(MOTION)],
        link: None,
    },
];

const LOOP_POINTS: &[ModelPoint] = &[
    ModelPoint {
        text: "The server logs each tool isError, tracks up to three follow-up calls in the session and marks a matching successful retry recovered. Compare the failed and recovered arguments to see what the agent expected.",
        spans: &[
            TextSpan::new(26, 33).color(MOTION),
            TextSpan::new(123, 132).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Agent undos (AGENT_UNDO) and friction notes are logged as well. Open project failures appear in <project>.splice/friction.jsonl and in the global log.",
        spans: &[
            TextSpan::new(13, 23).color(MOTION),
            TextSpan::new(96, 127).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "splice friction list groups by tool, command and code. The loop prioritizes clusters with at least two hits and calls the agent never recovered from.",
        spans: &[TextSpan::new(0, 20).color(MOTION)],
        link: None,
    },
    ModelPoint {
        text: "splice friction repro makes a transcript from the failed call. The change lands with its regression, and splice friction resolve closes the entry with the fix’s commit.",
        spans: &[
            TextSpan::new(0, 21).color(MOTION),
            TextSpan::new(105, 128).color(MOTION),
        ],
        link: None,
    },
];

const PRIORITIES: &[Text] = &[
    (
        "Full MCP coverage enforced by the parity test, with dry run, refs and session undo. (V13 built in the prototype; M1–M2)",
        &[TextSpan::new(84, 121).color(QUIET)],
    ),
    (
        "An agent-session review: a diff timeline, accept or reject per change, an audit log and permission scopes. (V13 M4)",
        &[TextSpan::new(107, 115).color(QUIET)],
    ),
    (
        "Frame and thumbnail rendering for the agent plus structured analysis: shots, loudness, silences. (V13 view.frames, audio.summary built; shots M6)",
        &[TextSpan::new(97, 145).color(QUIET)],
    ),
    (
        "Transcript-as-timeline editing with frame-accurate alignment and a choice of speech-to-text provider. (V13 M6)",
        &[TextSpan::new(102, 110).color(QUIET)],
    ),
    (
        "Filler, silence and retake detection with a review list; nothing is deleted silently. (V13 M6)",
        &[TextSpan::new(86, 94).color(QUIET)],
    ),
    (
        "Animated word captions: 20+ strong presets, per-word overrides, emoji, platform safe zones, SRT/VTT and burn-in. (V13 6 presets now; M6)",
        &[TextSpan::new(113, 136).color(QUIET)],
    ),
    (
        "A keyframe engine with Bezier and ease curves for every numeric parameter, and a graph editor. (V13 M8)",
        &[TextSpan::new(95, 103).color(QUIET)],
    ),
    (
        "Templates as parameter schemas (title, lower third, CTA, callout) and brand kits. (V13 M8)",
        &[TextSpan::new(82, 90).color(QUIET)],
    ),
    (
        "Screen-recording intelligence that stays editable: click zoom as real keyframes, cursor smoothing and hiding, keystroke chips. (V13 effects built; keyframe bake M8)",
        &[TextSpan::new(127, 164).color(QUIET)],
    ),
    (
        "Text animators driven by range selectors; captions and titles both use them. (V13 M8)",
        &[TextSpan::new(77, 85).color(QUIET)],
    ),
    (
        "A full trim toolset (ripple, roll, slip, slide) with J/L cuts on the spine. (V13 M4)",
        &[TextSpan::new(76, 84).color(QUIET)],
    ),
    (
        "Audio basics: voice isolation, loudness targets, auto-ducking, a keyboard-noise gate. (V13 M3 / M6)",
        &[TextSpan::new(86, 99).color(QUIET)],
    ),
    (
        "Compound clips and adjustment lanes with scoped overrides. (V13 adjustment scopes built; compound M8)",
        &[TextSpan::new(59, 101).color(QUIET)],
    ),
    (
        "Platform export presets and multi-aspect output from one timeline, with cursor-aware reframing. (V13 M5)",
        &[TextSpan::new(96, 104).color(QUIET)],
    ),
    (
        "FCPXML and OTIO export, so pros can finish elsewhere. Only the cut is promised to survive. (V13 M5+)",
        &[TextSpan::new(91, 100).color(QUIET)],
    ),
];

const FRICTION: &[[Text; 3]] = &[
    [
        ("Contract right, hint vague", &[]),
        (
            "Rewrite fix to name the call and arguments",
            &[TextSpan::new(8, 11).color(MOTION)],
        ),
        ("Transcript asserting the new hint", &[]),
    ],
    [
        ("Agent's expectation is reasonable", &[]),
        ("Accept it: alias, default, coercion, anchor form", &[]),
        ("The failing call, now passing", &[]),
    ],
    [
        ("Missing capability", &[]),
        ("A native registry command or parameter", &[]),
        ("Command test and transcript", &[]),
    ],
    [("Core bug", &[]), ("Fix it", &[]), ("Regression test", &[])],
    [
        ("Pattern agents keep missing", &[]),
        (
            "One line in the skill’s Pitfalls with the friction ID, removed after the core fixes the cause",
            &[],
        ),
        ("An eval task", &[]),
    ],
];

struct GapRow {
    comparison: [&'static str; 5],
    priority: &'static str,
    native: &'static str,
    shipped: bool,
}

const GAPS: &[GapRow] = &[
    GapRow {
        comparison: [
            "Agent API coverage",
            "Partial · MCP in Studio only",
            "FCPXML only",
            "Community bridges",
            "Underlord chat",
        ],
        priority: "P0 · moat",
        native: "",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Transcript editing, filler and silence cuts",
            "Partial",
            "Search only",
            "Text-based editing",
            "Descript is the best",
        ],
        priority: "P0",
        native: "word anchors, smart cuts",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Animated word captions",
            "Yes (v20)",
            "No (TO VERIFY)",
            "Weak",
            "The best",
        ],
        priority: "P0",
        native: "6 modes, word overrides",
        shipped: true,
    },
    GapRow {
        comparison: ["SRT/VTT, burn-in", "Yes", "Yes", "Yes", "Yes"],
        priority: "P0",
        native: "SRT/VTT and burn-in",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Keyframes with Bezier and ease curves, graph editor",
            "Yes (new keyframe editor)",
            "Basic · Motion full",
            "Yes",
            "CapCut yes",
        ],
        priority: "P0",
        native: "Bézier parameter keys",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Text animators per character / word / line",
            "Follower (TO VERIFY)",
            "Motion behaviors",
            "AE only",
            "Presets",
        ],
        priority: "P0",
        native: "title units, caption presets",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Templates with exposed parameters, brand kit",
            "Fusion macros",
            "Motion publish",
            "MOGRT",
            "Template library",
        ],
        priority: "P0",
        native: "templates and brand kit",
        shipped: true,
    },
    GapRow {
        comparison: ["Compound / nested clips", "Yes", "Yes", "Yes", "CapCut yes"],
        priority: "P0",
        native: "compound clips",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Ripple, roll, slip, slide; J/L cuts",
            "Yes",
            "Yes",
            "Yes",
            "Partial",
        ],
        priority: "P0",
        native: "roll, slip, slide",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Ducking, loudness target, voice cleanup",
            "Yes",
            "Partial",
            "Yes",
            "Studio Sound",
        ],
        priority: "P0",
        native: "duck, LUFS, isolation",
        shipped: true,
    },
    GapRow {
        comparison: ["Adjustment layers", "Yes", "Yes", "Yes", "CapCut yes"],
        priority: "P0",
        native: "scoped effects",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Cursor-aware zoom, cursor and keystroke layers",
            "No",
            "No",
            "No",
            "Screen Studio",
        ],
        priority: "P0 · edge",
        native: "cursor-aware layers",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Masks (Bezier, tracked), point tracking",
            "Yes",
            "Yes",
            "Yes",
            "Basic",
        ],
        priority: "P1",
        native: "tracked shape masks",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Speed ramps, time remap",
            "Yes",
            "Yes",
            "Yes",
            "CapCut curves",
        ],
        priority: "P1",
        native: "speed ramps",
        shipped: true,
    },
    GapRow {
        comparison: [
            "Shape layers, blend modes, mattes, motion blur",
            "Fusion",
            "Motion",
            "AE / EG",
            "Some",
        ],
        priority: "P1",
        native: "shapes, blend, blur",
        shipped: false,
    },
    GapRow {
        comparison: [
            "Scopes, LUTs, color wheels",
            "The best",
            "Yes",
            "Lumetri",
            "Basic",
        ],
        priority: "P1",
        native: "LUTs and colour controls",
        shipped: false,
    },
    GapRow {
        comparison: ["FCPXML / OTIO export", "Yes", "FCPXML", "OTIO beta", "No"],
        priority: "P1",
        native: "FCPXML",
        shipped: false,
    },
    GapRow {
        comparison: [
            "AI subject mask, stabilization, multicam",
            "Yes",
            "Yes",
            "Yes",
            "Some",
        ],
        priority: "P2",
        native: "subject cutout",
        shipped: false,
    },
    GapRow {
        comparison: [
            "3D, particles, node grading, HDR grading, Fairlight-grade mixing",
            "Yes",
            "Motion",
            "AE",
            "No",
        ],
        priority: "skip",
        native: "2D particles",
        shipped: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Event, testing};

    #[test]
    fn compact_gap_matrix_reaches_the_native_status_column() {
        let width = 448.0;
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let mut draw = |ui: &mut Ui| {
            ui.column(COLUMN.w_px(width), |ui| gap_matrix(ui, width));
        };
        testing::frame(&mut ui, &[], &mut draw);
        let viewport = testing::find(&ui, Role::ScrollArea, "Gap matrix columns").unwrap();
        let before = testing::find(&ui, Role::ColumnHeader, "Now").unwrap();
        assert_eq!(viewport.w, width);
        assert!(before.x > viewport.x + viewport.w, "{before:?}");
        assert_eq!(
            ui.a11y_nodes()
                .iter()
                .filter(|n| n.is_role(Role::Row))
                .count(),
            20
        );
        testing::frame(
            &mut ui,
            &[
                Event::pointer(Event::POINTER_MOVED, 0, 180.0, 100.0),
                Event::pointer(Event::SCROLLED, 0, -32.0, 0.0),
            ],
            &mut draw,
        );
        testing::frame(&mut ui, &[], &mut draw);
        let after = testing::find(&ui, Role::ColumnHeader, "Now").unwrap();
        assert!(after.x + after.w <= viewport.x + width, "{after:?}");
        let status = format!("{} tools, parity", splice_mcp::tool_catalog().len());
        let rect = testing::find(&ui, Role::Cell, &status).unwrap();
        assert!(rect.x + rect.w <= viewport.x + width, "{rect:?}");
    }
}
