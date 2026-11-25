use pxui::draw::flags::rect::LEFT;
use pxui::geom::{Edges, Rgba};
use pxui::ui::{Align, Bar, Flags, Look, State, States};
use pxui::{Fill, Label, Node, Paint, Point, Pressable, Rect, Role, Size, Stroke, TextSpan, Ui};

use super::{
    ACCENT, BG, DOC_BODY, DOC_LEDE, HEADING, INK, LINE, MOTION, MUTED, PANEL, QUIET, SAGE,
    SECTION_CAPTION, SECTIONS, STATUS_LINK, SUBHEADING, SURFACE, TABLE_EDGE, TABLE_TEXT, doc_label,
    spec_table, table_cell, table_row, table_tag,
};

mod icons;
#[cfg(test)]
mod tests;

pub(super) fn document(
    ui: &mut Ui,
    anchor: &str,
    narrow: bool,
    picked: &mut Option<usize>,
) -> bool {
    if !matches!(
        anchor,
        "s-arch"
            | "s-ui"
            | "s-icons"
            | "s-media"
            | "s-rust"
            | "s-alloc"
            | "s-build"
            | "s-tests"
            | "s-perf"
    ) {
        return false;
    }
    let width = super::spec_table_width(ui, narrow);
    match anchor {
        "s-arch" => {
            doc_label(ui, "Architecture", HEADING, INK);
            architecture(ui, width, picked);
        }
        "s-ui" => {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .align(Align::Start)
                    .gap(8.0)
                    .wrap(),
                |ui| {
                    doc_label(ui, "UI framework: PXUI", HEADING.w(Size::Fit), INK);
                    ui.column(
                        Node::column()
                            .w(Size::Fit)
                            .padding(Edges::new(0.0, 17.0, 0.0, 0.0)),
                        |ui| {
                            ui.row(
                                Node::row()
                                    .w_px(48.0)
                                    .h_px(18.0)
                                    .center()
                                    .look(Look::NONE.border(MOTION, 1.0)),
                                |ui| {
                                    doc_label(
                                        ui,
                                        "proposed",
                                        Label::BODY
                                            .font(1)
                                            .weight(300)
                                            .size(10.0)
                                            .line_height(16.0)
                                            .w(Size::Fit),
                                        MOTION,
                                    )
                                },
                            );
                        },
                    );
                },
            );
            framework(ui, width, narrow, picked);
        }
        "s-icons" => {
            doc_label(ui, "Icon system", HEADING, INK);
            icon_system(ui, width, picked);
        }
        "s-media" => {
            doc_label(ui, "Media on macOS (no ffmpeg)", HEADING, INK);
            ui.view_of("media-document", &width.to_bits(), |ui| {
                media_section(ui, width)
            });
        }
        "s-rust" => {
            doc_label(ui, "Rust rules", HEADING, INK);
            rust_section(ui, width, narrow, picked);
        }
        "s-alloc" => {
            doc_label(ui, "Zero allocation", HEADING, INK);
            ui.view_of("allocation-document", &width.to_bits(), |ui| {
                alloc_section(ui, width)
            });
        }
        "s-build" => {
            doc_label(ui, "Build speed", HEADING, INK);
            ui.view_of("build-document", &width.to_bits(), |ui| {
                points(ui, width, BUILD_POINTS)
            });
        }
        "s-tests" => {
            doc_label(ui, "Testing", HEADING, INK);
            testing_section(ui, width, picked);
        }
        "s-perf" => {
            doc_label(ui, "Performance budgets", HEADING, INK);
            ui.view_of("performance-document", &width.to_bits(), |ui| {
                performance_section(ui, width)
            });
        }
        _ => return false,
    }
    true
}

// Keep a fractional advance at the 78-column boundary through PXUI's glyph rounding.
const PROSE: f32 = 562.0;
const LEDE: f32 = 624.0;
const CONTROL: Rgba = Rgba::rgb(20, 20, 21);

type Run = (&'static str, Rgba, u16);
type Paragraph = (&'static str, &'static [Run]);

const fn code(text: &'static str) -> Run {
    (text, MOTION, 0)
}

const fn strong(text: &'static str) -> Run {
    (text, INK, 500)
}

fn rich(
    ui: &mut Ui,
    width: f32,
    style: Label,
    role: Role,
    color: Rgba,
    line_height: f32,
    (text, runs): Paragraph,
) {
    let style = style.role(role);
    if runs.is_empty() {
        doc_label(ui, text, style.w_px(width), color);
        return;
    }
    let mut spans = [TextSpan::default(); 8];
    let mut end = 0;
    for (span, &(run, color, weight)) in spans.iter_mut().zip(runs) {
        let start = end
            + text[end..]
                .find(run)
                .expect("engineering span matches its text");
        end = start + run.len();
        *span = TextSpan::new(start as u32, end as u32)
            .color(color)
            .weight(weight);
    }
    let spans = &spans[..runs.len()];
    let lines = ui
        .text_service()
        .measure_spans(text, style, Some(width), spans)
        .expect("valid Engineering document spans")
        .lines;
    let node = ui.node(
        "engineering-paragraph",
        Node::DEFAULT
            .w_px(width)
            .h_px(lines as f32 * line_height)
            .role(role)
            .look(Look::NONE.text(color)),
    );
    ui.text_spans(&node, text, style, spans)
        .expect("valid Engineering document spans");
}

fn paragraph(ui: &mut Ui, width: f32, text: Paragraph) {
    rich(
        ui,
        width.min(PROSE).max(1.0),
        DOC_BODY,
        Role::Paragraph,
        MUTED,
        19.44,
        text,
    );
}

fn points(ui: &mut Ui, width: f32, points: &[Paragraph]) {
    let width = (width - 20.0).clamp(1.0, PROSE);
    ui.column(Node::column().w(Size::Grow).role(Role::List), |ui| {
        for &point in points {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .align(Align::Start)
                    .role(Role::ListItem),
                |ui| {
                    doc_label(ui, "•", DOC_BODY.w_px(20.0).role(Role::Unknown), MUTED);
                    paragraph(ui, width, point);
                },
            );
        }
    });
}

fn block(ui: &mut Ui, top: f32, bottom: f32, draw: &mut dyn FnMut(&mut Ui)) {
    ui.column(
        Node::column()
            .w(Size::Grow)
            .padding(Edges::new(0.0, top, 0.0, bottom)),
        draw,
    );
}

fn heading(ui: &mut Ui, title: &str) {
    block(ui, 20.0, 6.0, &mut |ui| {
        doc_label(ui, title, SUBHEADING, INK)
    });
}

fn links(ui: &mut Ui, targets: &[&'static str], picked: &mut Option<usize>) {
    ui.row(Node::row().w(Size::Grow).gap(12.0).wrap(), |ui| {
        for &anchor in targets {
            document_link(ui, anchor, picked);
        }
    });
}

fn document_link(ui: &mut Ui, anchor: &'static str, picked: &mut Option<usize>) {
    let Some((index, &(_, _, title, _))) = SECTIONS
        .iter()
        .enumerate()
        .find(|(_, section)| section.0 == anchor)
    else {
        return;
    };
    let link = ui.pressable(
        anchor,
        Node::row()
            .w(Size::Fit)
            .h_px(19.44)
            .align(Align::Center)
            .role(Role::Link),
        &Pressable::DEFAULT.look(&STATUS_LINK),
        |ui, _| doc_label(ui, title, DOC_BODY.w(Size::Fit).role(Role::Unknown), MOTION),
    );
    ui.a11y_name(&link.handle, title);
    if link.clicked {
        *picked = Some(index);
    }
}

fn architecture(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        ui.view_of("architecture-content", &width.to_bits(), |ui| {
            paragraph(ui, width, ("The core is a single-writer state machine. Everything that changes the project is an op list applied in one transaction. The UI, MCP and CLI only produce ops and read snapshots. Media, rendering and audio are separate subsystems. They read an immutable evaluation of the timeline and never write to it.", &[]));
            block(ui, 8.0, 8.0, &mut |ui| diagram(ui, width.min(960.0)));
            paragraph(ui, width, (
                "Below these sit the *_sys crates and PXUI's platform bindings. They wrap AVFoundation, VideoToolbox, CoreMedia, CoreVideo, IOSurface, Metal, CoreAudio, ScreenCaptureKit, Speech and AppKit. Splice keeps unsafe in *_sys crates.",
                const { &[code("*_sys"), code("unsafe"), code("*_sys")] },
            ));
            heading(ui, "Crates");
            crate_table(ui, width);
            heading(ui, "Threads");
            thread_table(ui, width);
            heading(ui, "Project file");
            paragraph(ui, width, ("Full spec in the repo in docs/FORMAT.md (splice/1).", const { &[code("docs/FORMAT.md"), code("splice/1")] }));
            block(ui, 8.0, 6.0, &mut |ui| package_tree(ui));
            points(ui, width, PACKAGE_NOTES);
            heading(ui, "Undo in Rust");
            points(ui, width, NATIVE_UNDO);
        });
        block(ui, 8.0, 0.0, &mut |ui| links(ui, &["s-registry", "s-mcp", "s-media", "s-sources"], picked));
    });
}

const PACKAGE_NOTES: &[Paragraph] = &[
    (
        "The journal is the truth. A commit is one fsynced line with ops, minted IDs, label, source, session and the resulting state hash. Each record links to the prior hash. Undo appends a record too (revertOf or restoreTo), so retained revisions stay restorable.",
        &[
            strong("The journal is the truth."),
            code("revertOf"),
            code("restoreTo"),
        ],
    ),
    (
        "Open the snapshot and replay later records, verifying hashes. A torn last line is truncated. Corruption elsewhere leaves the last verified revision read-only.",
        &[strong("Open")],
    ),
    (
        "Canonical JSON uses fixed key order, integer frames and shortest round-trip floats. The same project state has the same bytes and hash.",
        &[strong("Canonical JSON")],
    ),
    (
        "FCPXML and OpenTimelineIO are planned interchange formats. The native package uses splice/1.",
        &[code("splice/1")],
    ),
];

const NATIVE_UNDO: &[Paragraph] = &[
    (
        "Recorder keeps a snapshot stack of its Project value. Splice stores a base and checkpoints for its journal, and keeps revision handles in splice_kit for human undo and redo.",
        &[code("Project"), code("splice_kit")],
    ),
    (
        "Session and batch undo rebuild from the retained base, skip the reverted batches and replay later operations. The core appends the restore or revert record as a new revision.",
        &[],
    ),
];

const PACKAGE_TREE: &[&str] = &[
    "Name.splice/                 a package: Finder shows one file",
    "  project.json               snapshot at rev (tmp + fsync + rename)",
    "  journal.ops                append-only, one fsynced line per batch, hash-chained",
    "  journal.base.json          base snapshot for retained history",
    "  sessions.json              agent sessions and checkpoints",
    "  media.json                 med_ id → location and probe",
    "  transcripts/<med>.json     words with IDs and frames, plus the transcript hash",
    "  takes/<med>/               recordings and Recorder-compatible events.json",
    "  exports/                   rendered output",
    "  friction.jsonl             MCP friction log",
    "  cache/                     disposable: thumbnails, peaks, proxies",
    "  lock                       single writer (flock + pid + host)",
];

fn package_tree(ui: &mut Ui) {
    code_panel(ui, "project-package", "Project package", PACKAGE_TREE);
}

fn horizontal_scroll(ui: &mut Ui, key: &'static str, name: &str, look: Look) -> pxui::Handle {
    let scroll = ui.node(
        key,
        Node::column()
            .w(Size::Grow)
            .role(Role::ScrollArea)
            .flags(Flags::SCROLL_X | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
            .bar(Bar::new(TABLE_EDGE, 6.0).gap(2.0))
            .look(look),
    );
    ui.a11y_name(&scroll, name);
    scroll
}

fn code_panel(ui: &mut Ui, key: &'static str, name: &str, lines: &[&str]) {
    let scroll = horizontal_scroll(ui, key, name, Look::NONE.fill(SURFACE).border(LINE, 1.0));
    ui.within(&scroll, &mut |ui| {
        ui.column(
            Node::column()
                .w(Size::Fit)
                .padding(Edges::new(13.0, 9.0, 13.0, 9.0)),
            |ui| {
                for line in lines {
                    doc_label(
                        ui,
                        line,
                        Label::MONO.size(11.0).line_height(17.6).w(Size::Fit),
                        INK,
                    );
                }
            },
        );
    });
}

const CRATES: &[(&str, &str, &str, &str)] = &[
    (
        "splice_model",
        "Project data: arenas, IDs, frames, anchors and lanes",
        "std",
        "any",
    ),
    (
        "splice_core",
        "Registry, apply, settle and anchor rules, revisions, undo, sessions, replay and diff",
        "splice_model, splice_json",
        "any",
    ),
    (
        "splice_json",
        "Hand-written JSON reader and writer (project files, MCP)",
        "std",
        "any",
    ),
    (
        "splice_eval",
        "Timeline → per-frame render list and audio gains. Resolves effects by scope and projects caption word timing.",
        "splice_model",
        "any",
    ),
    (
        "splice_mcp",
        "JSON-RPC framing, tool schemas, same-user socket server and stdio shim",
        "splice_core, splice_eval, splice_json and native clients",
        "any (live: macOS)",
    ),
    (
        "splice_cli",
        "Headless apply, snapshot and describe, with native export where available",
        "splice_core, splice_mcp",
        "any",
    ),
    (
        "splice_media",
        "Import and probe, thumbnails and waveform peaks",
        "apple_sys, splice_model",
        "macOS",
    ),
    (
        "splice_gpu",
        "OGPU compositor shared by preview, MCP frame grabs and export",
        "ogpu_core",
        "portable (Metal in Splice)",
    ),
    (
        "splice_av",
        "AVFoundation decode, audio, composition and export, with CoreText caption raster",
        "apple_sys, splice_eval, splice_gpu and OGPU Metal",
        "macOS",
    ),
    (
        "splice_capture",
        "ScreenCaptureKit recording and Recorder-compatible take assembly",
        "apple_sys",
        "macOS",
    ),
    (
        "splice_kit",
        "PXUI-shaped command routing, gesture phases and undo mechanics",
        "std",
        "any",
    ),
    (
        "splice_app",
        "UI shell: timeline, viewer, inspector, captions, shortcuts and command palette",
        "pxui, pxui_host, pxui_sdl, splice_kit and the core",
        "macOS",
    ),
    (
        "*_sys",
        "Foreign bindings, including Apple media and speech, QuickJS and ThorVG. Unsafe stays here.",
        "std, cc (build)",
        "varies",
    ),
    (
        "splice_testkit",
        "Fixtures, golden snapshots, seeded operations and allocation probes",
        "splice_core, splice_json",
        "any",
    ),
    (
        "tools/gate",
        "Workspace shape and dependency policy",
        "std",
        "any",
    ),
];

const THREADS: &[(&str, &str, &str)] = &[
    (
        "Main",
        "UI, core apply, live MCP request dispatch",
        "One writer. The socket forwards requests to the editor loop, where project edits are applied.",
    ),
    (
        "Window / frame loop",
        "Preview scheduling",
        "Polls the current program and submits frames through the prepared Metal surface. It can keep the previous frame while a source is pending.",
    ),
    (
        "AVFoundation callbacks",
        "Decode into CVPixelBuffer / IOSurface",
        "The composition broker delivers source frames for the current request. Seek generations guard stale completions.",
    ),
    (
        "AVFoundation audio",
        "Playback audio and native track mixes",
        "AVFoundation owns the playback clock and audio output for assembled media.",
    ),
    (
        "Export",
        "Sequential decode, composite and encode",
        "The export stepper owns Metal handles on its calling thread. It evaluates and composites output in frame order.",
    ),
    (
        "Jobs",
        "Capture, transcription, thumbnails and peaks",
        "Native media and speech work use their own services and callbacks. The project writer publishes completed edits.",
    ),
];

// Approximate the reference's auto table widths with its compact widths and growth shares.
fn column_width(width: f32, compact: f32, share: f32) -> f32 {
    compact * (width / 448.0).min(1.0) + (width - 448.0).max(0.0) * share
}

fn crate_table(ui: &mut Ui, width: f32) {
    let name = column_width(width, 121.6, 0.1405);
    let role = column_width(width, 149.9, 0.736);
    let deps = column_width(width, 104.7, 0.062);
    let os = width - name - role - deps;
    spec_table(
        ui,
        "Crates",
        &[
            ("Crate", Size::Px(name)),
            ("Role", Size::Px(role)),
            ("May depend on", Size::Px(deps)),
            ("OS", Size::Px(os)),
        ],
        &mut |ui| {
            for &(id, description, dependencies, platform) in CRATES {
                table_row(ui, &mut |ui| {
                    cells(
                        ui,
                        &[
                            (name, id),
                            (role, description),
                            (deps, dependencies),
                            (os, platform),
                        ],
                    )
                });
            }
        },
    );
}

fn thread_table(ui: &mut Ui, width: f32) {
    let name = column_width(width, 118.6, 0.1298);
    let owns = column_width(width, 137.3, 0.2238);
    let rule = width - name - owns;
    spec_table(
        ui,
        "Threads",
        &[
            ("Thread", Size::Px(name)),
            ("Owns", Size::Px(owns)),
            ("Rule", Size::Px(rule)),
        ],
        &mut |ui| {
            for &(thread, responsibility, contract) in THREADS {
                table_row(ui, &mut |ui| {
                    cells(
                        ui,
                        &[(name, thread), (owns, responsibility), (rule, contract)],
                    )
                });
            }
        },
    );
}

fn cells(ui: &mut Ui, cells: &[(f32, &str)]) {
    for (index, &(width, text)) in cells.iter().enumerate() {
        cell(ui, width, text, if index == 0 { INK } else { MUTED });
    }
}

fn cell(ui: &mut Ui, width: f32, text: &str, color: Rgba) {
    table_cell(ui, Size::Px(width), &mut |ui| {
        doc_label(ui, text, TABLE_TEXT, color)
    });
}

const FRAMEWORK_LEDE: Paragraph = (
    "Splice's UI is built on PXUI, the in-house framework that rays uses and that LocalWave builds its OS on. Commands, undo, menus, gesture phases and video surfaces put PXUI to work in a production editor. Splice builds generic editor mechanics in PXUI's shape in splice_kit first and exercises them in the editor. The plan is to move the tested pieces that fit into PXUI so other apps on the OS can use them.",
    &[code("splice_kit")],
);

const FRAMEWORK_STATUS: &[Paragraph] = &[
    (
        "PXUI's public API has been shaped by the Unification Plan from LocalWave. The pinned checkout includes themes, a frame lifecycle, typed keys, retained views, lists, tables and drop targets. Splice uses the public UI and open host APIs, with command routing and document undo in splice_kit.",
        &[code("splice_kit")],
    ),
    (
        "The editor uses std and cc around the platform shims, safe APIs around native bindings and zero allocations after warmup. The cargo t runner enforces 5 s per case and 60 s per run. The headless layout tests exercise the same public UI API used by the app.",
        &[code("std"), code("cc"), code("cargo t")],
    ),
];

const KIT_LEDE: Paragraph = (
    "splice_kit depends on std (and pxui_types where needed). It uses fixed command tables and preallocated state. The longer-term plan is to upstream generic pieces with their tests. Repo doc: docs/KIT.md.",
    &[
        code("splice_kit"),
        code("std"),
        code("pxui_types"),
        code("docs/KIT.md"),
    ],
);

const KIT_COMMANDS: &[Paragraph] = &[
    (
        "Command { id: CommandId(u32), name, title, keys, group } lives in static tables. The keys field holds a static slice of Shortcut values.",
        &[code(
            "Command { id: CommandId(u32), name, title, keys, group }",
        )],
    ),
    (
        "Responder chain. Responders::responds registers a node, its parent and its commands. Responders::resolve starts at focus and walks through its ancestors. Text fields get their editing keys first.",
        &[strong("Responder chain."), code("Responders::resolve")],
    ),
    (
        "Validation. validate fills a caller-owned CommandState buffer in table order. Menus and the palette use enabled, checked and title. Dispatch checks availability.",
        &[strong("Validation."), code("CommandState")],
    ),
    (
        "Output. Invoked carries a command ID, a source and the target responder. Keys, controls and the command palette share editor dispatch. Editing actions are translated through the registry into the core ops that MCP applies.",
        &[strong("Output."), code("Invoked")],
    ),
    (
        "Keymap overrides use key = value. Chords use named keys and exact modifiers. The shortcut list reflects the active bindings, and the parser reports conflicting chords with both source lines.",
        &[code("key = value")],
    ),
];

const KIT_UNDO: &[Paragraph] = &[
    (
        "Merge policy. Merge::{Step, Gesture, Burst, Repair} governs coalescing. seal closes an active merge. Count and byte caps bound retained entries.",
        &[
            strong("Merge policy."),
            code("Merge::{Step, Gesture, Burst, Repair}"),
            code("seal"),
        ],
    ),
    (
        "Entries hold a label, merge mode, time and a u64 payload in a preallocated ring. The payload is the app's revision handle.",
        &[code("u64")],
    ),
    (
        "Ended seals a gesture. Cancelled discards the partial step and returns the revision to restore. Text fields keep their own editing undo.",
        &[code("Ended"), code("Cancelled")],
    ),
    (
        "The project's history stays in the core journal. Splice uses revision handles and labels for human undo and redo. Agent batches retain their session and batch identity.",
        &[],
    ),
];

const ADOPTION: Paragraph = (
    "Adoption. (1) The pxui/ submodule pins the checkout. Splice uses pxui, pxui_host, pxui_sdl (including show_dialog), mem_sys and PXUI's cargo t. (2) Commands and undo mechanics live in splice_kit. Program frames share a Metal device and sampled texture with the PXUI renderer. (3) Generic kit APIs are candidates for a later PXUI release. (4) The dev build links libpxui dynamically. The app bundle carries it in Frameworks/.",
    &[
        strong("Adoption."),
        code("pxui/"),
        code("pxui_host"),
        code("pxui_sdl"),
        code("show_dialog"),
        code("mem_sys"),
        code("cargo t"),
        code("splice_kit"),
    ],
);

const RISKS: Paragraph = (
    "Risks. The pinned API evolves, and SDL3 needs native host work for AppKit integrations. Splice's renderer runs on Metal. VoiceOver and automation depend on the host exposing the text, table and focus information PXUI produces.",
    &[strong("Risks.")],
);

fn framework(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        ui.view_of("framework-content", &(width.to_bits(), narrow), |ui| {
            rich(
                ui,
                width.min(LEDE),
                DOC_LEDE,
                Role::Paragraph,
                INK,
                21.06,
                FRAMEWORK_LEDE,
            );
            block(ui, 20.0, 6.0, &mut |ui| {
                let abi = pxui::PxuiUiDesc::default().abi_built;
                let title = ui.label_fmt(
                    format_args!(
                        "Where PXUI stands (pinned ABI {}.{})",
                        abi >> 16,
                        abi & 0xffff
                    ),
                    SUBHEADING,
                );
                ui.style(&title, &States::of(Look::NONE.text(INK)), State::default());
            });
            points(ui, width, FRAMEWORK_STATUS);
            block(ui, 6.0, 8.0, &mut |ui| framework_table(ui, width));
            heading(ui, "Built in splice_kit now, moved into PXUI later");
            paragraph(ui, width, KIT_LEDE);
            block(ui, 6.0, 8.0, &mut |ui| kit_cards(ui, width, narrow));
            let callout_width = width.min(576.0);
            ui.column(
                Node::column()
                    .w_px(callout_width)
                    .padding(Edges::new(14.0, 8.0, 12.0, 8.0))
                    .look(Look::NONE.fill(CONTROL).border(ACCENT, 2.0).sides(LEFT)),
                |ui| paragraph(ui, (callout_width - 26.0).max(1.0), ADOPTION),
            );
            block(ui, 8.0, 0.0, &mut |ui| paragraph(ui, width, RISKS));
        });
        block(ui, 8.0, 0.0, &mut |ui| {
            links(ui, &["s-arch", "s-registry", "s-icons"], picked)
        });
    });
}

const FRAMEWORK_ROWS: &[(&str, &str, &str, &str)] = &[
    (
        "Command system: responder chain, validation, key equivalents",
        "Splice routes shortcuts and responder state in splice_kit. Text fields handle their own edits.",
        "kit",
        "splice_kit · Splice",
    ),
    (
        "Video inside the UI tree",
        "External Metal textures reach PXUI as Image values. The app and compositor share an OGPU device.",
        "ready",
        "PXUI handoff · Splice decode",
    ),
    (
        "Document undo manager with grouping and coalescing",
        "Splice uses a kit undo stack, with durable revisions in the core journal.",
        "kit",
        "splice_kit · core history",
    ),
    (
        "Gesture phases on continuous controls",
        "PXUI exposes scrub phases. Splice groups its edit gestures by ID.",
        "partial",
        "PXUI · Splice",
    ),
    (
        "Native NSMenu bar, key equivalents",
        "Splice draws its menus and palette in PXUI and resolves shortcuts through its router.",
        "partial",
        "PXUI host · Splice",
    ),
    (
        "Caption editing: styled spans, per-span hits",
        "PXUI supports text editing and styled label spans. Splice owns the word model, caption inspector and timing.",
        "partial",
        "PXUI spans · Splice",
    ),
    (
        "Drag out to Finder, between windows, typed pasteboard",
        "The host receives file drops. PXUI supports in-Ui drag and drop.",
        "partial",
        "PXUI host",
    ),
    (
        "Vector icons, brand mark",
        "Splice draws glyphs with PXUI strokes and curves. The brand mark uses filled primitives.",
        "ready",
        "Splice · PXUI",
    ),
    (
        "Inspector panels, multi-window",
        "The inspector lives in the editor UI. PXUI separates trees by surface.",
        "partial",
        "PXUI · Splice",
    ),
    (
        "Automation for MCP parity",
        "PXUI exposes a typed accessibility tree and action injection. The command registry connects editing surfaces.",
        "ready",
        "PXUI a11y · Splice MCP",
    ),
    (
        "Pinch, trackpad scroll, virtual lists, themes, focus, tooltips, goldens",
        "Present in PXUI, including virtualized lists and the headless testing API.",
        "ready",
        "PXUI",
    ),
];

fn framework_table(ui: &mut Ui, width: f32) {
    let need = column_width(width, 100.6, 0.35);
    let present = column_width(width, 159.5, 0.527);
    let gap = column_width(width, 70.0, 0.0)
        .max(62.0)
        .min((width - need - present) * 0.55);
    let owner = width - need - present - gap;
    spec_table(
        ui,
        "PXUI capabilities",
        &[
            ("Splice needs", Size::Px(need)),
            ("PXUI today", Size::Px(present)),
            ("Gap", Size::Px(gap)),
            ("Owner", Size::Px(owner)),
        ],
        &mut |ui| {
            for &(feature, state, status, responsibility) in FRAMEWORK_ROWS {
                table_row(ui, &mut |ui| {
                    cell(ui, need, feature, INK);
                    cell(ui, present, state, MUTED);
                    table_cell(ui, Size::Px(gap), &mut |ui| {
                        let ink = if status == "ready" { SAGE } else { MOTION };
                        table_tag(ui, status, ink, ink, false);
                    });
                    cell(ui, owner, responsibility, MUTED);
                });
            }
        },
    );
}

fn kit_cards(ui: &mut Ui, width: f32, narrow: bool) {
    let card_width = if narrow { width } else { (width - 12.0) * 0.5 };
    let grid = Node::column().w(Size::Grow).gap(12.0);
    let draw = |ui: &mut Ui| {
        for (name, accessible_name, items) in [
            ("Commands", "PXUI Commands", KIT_COMMANDS),
            ("Undo", "PXUI Undo", KIT_UNDO),
        ] {
            note_card(ui, card_width, name, accessible_name, items);
        }
    };
    if narrow {
        ui.column(grid, draw);
    } else {
        ui.row(grid.align(Align::Stretch), draw);
    }
}

fn note_card(ui: &mut Ui, card_width: f32, name: &str, accessible_name: &str, items: &[Paragraph]) {
    let card = ui.column(
        Node::column()
            .w_px(card_width)
            .padding(Edges::new(13.0, 9.0, 13.0, 15.0))
            .gap(6.0)
            .role(Role::Group)
            .look(Look::NONE.fill(PANEL).border(LINE, 1.0)),
        |ui| {
            doc_label(
                ui,
                name,
                SECTION_CAPTION.line_height(13.0).role(Role::Heading),
                QUIET,
            );
            points(ui, card_width - 26.0, items);
        },
    );
    ui.a11y_name(&card, accessible_name);
}

const ICON_POINTS: &[Paragraph] = &[
    (
        "Grid: 24 units, a 3-unit margin (2.5 for wide shapes), drawn at 12–16 px in controls.",
        &[strong("Grid:")],
    ),
    (
        "Stroke: 1.6, round caps and joins, corner radius 1.5–2. The brand mark uses a filled square.",
        &[strong("Stroke:")],
    ),
    (
        "Colour: currentColor in the prototype. Selected tabs and the effects browser use the accent. Lane heads use the lane's tone.",
        &[strong("Colour:"), code("currentColor")],
    ),
    (
        "Mark: a rounded square with a pulse cut out. The rising pulse represents a signal and an edit in one shape.",
        &[strong("Mark:")],
    ),
    (
        "Naming: glyph names describe shapes (layers, film). The prototype maps semantic names onto them (overlay → layers, primary → film).",
        &[
            strong("Naming:"),
            code("layers"),
            code("film"),
            code("overlay → layers"),
            code("primary → film"),
        ],
    ),
];

fn icon_system(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        paragraph(ui, width, (
            "One glyph table drives the v13 titlebar, tabs, toolbars, lane heads, effects browser, media and inspector. Each glyph is defined once in icons.js, mounted as an inline SVG sprite and referenced by name. Splice's native controls use PXUI paths and fills, including the filled brand mark. The gallery draws the v13 catalogue with those same primitives. A glyph has the same shape wherever the prototype references its name.",
            const { &[code("icons.js")] },
        ));
        block(ui, 6.0, 0.0, &mut |ui| points(ui, width, ICON_POINTS));
        icons::gallery(ui, width);
        block(ui, 8.0, 0.0, &mut |ui| links(ui, &["s-ui", "s-effects"], picked));
    });
}

const DIAGRAM_NAME: &str = "Splice architecture. The PXUI shell, MCP and CLI apply operations to the single-writer core. Its immutable snapshot feeds the evaluator, which supplies the compositor, media and audio.";
const DIAGRAM_STRONG: Rgba = Rgba::rgb(110, 110, 105);

struct Stage {
    bounds: Rect,
    edge: Rgba,
    labels: &'static [&'static str],
}

const STAGES: &[Stage] = &[
    Stage {
        bounds: Rect::new(20.0, 20.0, 250.0, 70.0),
        edge: TABLE_EDGE,
        labels: &[
            "UI shell (PXUI)",
            "menus · keymap · timeline view · inspector",
            "returns intents",
        ],
    },
    Stage {
        bounds: Rect::new(355.0, 20.0, 250.0, 70.0),
        edge: MOTION,
        labels: &[
            "splice_mcp",
            "JSON-RPC 2.0 · tool schemas",
            "stdio shim ⇄ Unix socket ⇄ app",
        ],
    },
    Stage {
        bounds: Rect::new(690.0, 20.0, 250.0, 70.0),
        edge: TABLE_EDGE,
        labels: &[
            "splice_cli",
            "apply ops.json · snapshot · export",
            "headless, same core",
        ],
    },
    Stage {
        bounds: Rect::new(20.0, 140.0, 920.0, 96.0),
        edge: ACCENT,
        labels: &[
            "splice_core · single writer",
            "registry (static table) · apply(ops) → changes + side effects · revisions · journal · sessions · replay · settle/anchor rules",
            "splice_model: arenas with stable IDs · clips, items, lanes, markers · integer frames · no platform I/O",
            "splice_json: hand-written reader/writer for project files and MCP payloads",
        ],
    },
    Stage {
        bounds: Rect::new(300.0, 270.0, 360.0, 54.0),
        edge: TABLE_EDGE,
        labels: &[
            "splice_eval",
            "frame f → layers, transforms, fades and audio gains",
        ],
    },
    Stage {
        bounds: Rect::new(20.0, 370.0, 250.0, 80.0),
        edge: TABLE_EDGE,
        labels: &[
            "splice_gpu",
            "one OGPU compositor for preview,",
            "frame grabs and export",
            "Metal backend in Splice",
        ],
    },
    Stage {
        bounds: Rect::new(355.0, 370.0, 250.0, 80.0),
        edge: TABLE_EDGE,
        labels: &[
            "splice_media / splice_av",
            "decode → CVPixelBuffer (IOSurface)",
            "frame access · encode · probe",
            "thumbnails · waveform peaks",
        ],
    },
    Stage {
        bounds: Rect::new(690.0, 370.0, 250.0, 80.0),
        edge: TABLE_EDGE,
        labels: &[
            "splice_av · audio",
            "AVFoundation playback and mixes",
            "clip and lane gain from evaluation",
            "native preview and export",
        ],
    },
];

fn diagram(ui: &mut Ui, width: f32) {
    let node = ui.node(
        "architecture-diagram",
        Node::DEFAULT
            .w_px(width)
            .h_px(width * (470.0 / 960.0))
            .role(Role::Image),
    );
    ui.a11y_name(&node, DIAGRAM_NAME);
    ui.canvas(&node, |paint, rect| {
        let scale = rect.w / 960.0;
        let point = |(x, y)| (rect.x + x * scale, rect.y + y * scale);
        let text = |paint: &mut Paint<'_>, at, size: f32, value: &str, color| {
            let (x, y) = point(at);
            let style = Label::MONO.size(size * scale).line_height(size * scale);
            let span = TextSpan::new(0, value.len() as u32).color(color);
            paint
                .text_spans(Point::new(x, y), style, 0.0, value, &[span])
                .expect("valid Architecture diagram text");
        };

        for stage in STAGES {
            let box_ = stage.bounds;
            let (x, y) = point((box_.x, box_.y));
            let rect = Rect::new(x, y, box_.w * scale, box_.h * scale);
            paint.fill(rect, Fill::new(PANEL).fractional());
            paint.frame(rect, Stroke::new(stage.edge).width(scale));
            let first = if box_.y < 100.0 { 16.0 } else { 13.0 };
            for (index, &label) in stage.labels.iter().enumerate() {
                let (size, position, ink) = if index == 0 {
                    (11.0, first, INK)
                } else {
                    (10.0, first + 4.0 + index as f32 * 16.0, QUIET)
                };
                text(paint, (box_.x + 14.0, box_.y + position), size, label, ink);
            }
        }
        for path in [
            &[(145.0, 90.0), (145.0, 140.0)][..],
            &[(480.0, 90.0), (480.0, 140.0)],
            &[
                (815.0, 90.0),
                (815.0, 120.0),
                (600.0, 120.0),
                (600.0, 140.0),
            ],
            &[(480.0, 236.0), (480.0, 270.0)],
            &[
                (380.0, 324.0),
                (380.0, 352.0),
                (145.0, 352.0),
                (145.0, 370.0),
            ],
            &[(480.0, 324.0), (480.0, 370.0)],
            &[
                (580.0, 324.0),
                (580.0, 352.0),
                (815.0, 352.0),
                (815.0, 370.0),
            ],
        ] {
            for segment in path.windows(2) {
                paint.line(
                    point(segment[0]),
                    point(segment[1]),
                    Stroke::new(DIAGRAM_STRONG).width(scale),
                );
            }
            let &(x, y) = path.last().expect("arrow endpoint");
            paint.triangle(
                point((x - 3.5, y - 7.0)),
                point((x + 3.5, y - 7.0)),
                point((x, y)),
                Fill::new(MUTED),
            );
        }
        for (at, label) in [
            ((155.0, 113.0), "ops [{cmd,…}]"),
            ((490.0, 113.0), "edit.apply"),
            ((490.0, 249.0), "immutable snapshot per revision"),
        ] {
            text(paint, at, 10.0, label, QUIET);
        }
    });
}

type DetailRow = (&'static str, Paragraph, Paragraph);

fn detail_columns(width: f32, compact: [f32; 2], shares: [f32; 2]) -> [f32; 3] {
    let first = column_width(width, compact[0], shares[0]);
    let second = column_width(width, compact[1], shares[1]);
    [first, second, width - first - second]
}

fn detail_table(
    ui: &mut Ui,
    name: &'static str,
    heads: [&str; 3],
    widths: [f32; 3],
    rows: &[DetailRow],
    mut parity_link: Option<&mut Option<usize>>,
) {
    let [a, b, c] = widths;
    spec_table(
        ui,
        name,
        &[
            (heads[0], Size::Px(a)),
            (heads[1], Size::Px(b)),
            (heads[2], Size::Px(c)),
        ],
        &mut |ui| {
            for &(name, detail, notes) in rows {
                table_row(ui, &mut |ui| {
                    cell(ui, a, name, INK);
                    table_cell(ui, Size::Px(b), &mut |ui| {
                        rich(
                            ui,
                            (b - 16.0).max(1.0),
                            TABLE_TEXT,
                            Role::Cell,
                            MUTED,
                            15.4,
                            detail,
                        );
                        if name == "Parity"
                            && let Some(picked) = parity_link.as_deref_mut()
                        {
                            document_link(ui, "s-parity", picked);
                        }
                    });
                    table_cell(ui, Size::Px(c), &mut |ui| {
                        rich(
                            ui,
                            (c - 16.0).max(1.0),
                            TABLE_TEXT,
                            Role::Cell,
                            MUTED,
                            15.4,
                            notes,
                        );
                    });
                });
            }
        },
    );
}

const MEDIA_LEAD: Paragraph = (
    "Splice uses native macOS frameworks for import, playback, export and recording. It follows Recorder's system-framework approach and calls Apple media services from Rust through apple_sys.",
    &[code("apple_sys")],
);

const MEDIA_ROWS: &[DetailRow] = &[
    (
        "Import, probe",
        ("AVFoundation AVURLAsset, CoreMedia", &[code("AVURLAsset")]),
        (
            "Read duration, tracks, dimensions, codec and rate. The clip uses integer frames on the project timeline.",
            &[],
        ),
    ),
    (
        "Preview decode",
        (
            "AVPlayer and AVPlayerItemVideoOutput, with an AVFoundation composition broker",
            &[code("AVPlayer"), code("AVPlayerItemVideoOutput")],
        ),
        (
            "The broker delivers decoded sources. Splice evaluates anchors and layers and composites the result on the GPU.",
            &[],
        ),
    ),
    (
        "Zero copy to GPU",
        (
            "CoreVideo CVPixelBuffer with kCVPixelBufferMetalCompatibilityKey and IOSurface backing",
            &[
                code("CVPixelBuffer"),
                code("kCVPixelBufferMetalCompatibilityKey"),
            ],
        ),
        (
            "Decoded BGRA IOSurfaces are wrapped as Metal texture views on the compositor's device. OGPU then shares the Program texture with PXUI.",
            &[],
        ),
    ),
    (
        "Compositing",
        (
            "OGPU on Metal, with embedded MSL from splice_gpu/shaders/layer.metal",
            &[code("splice_gpu/shaders/layer.metal")],
        ),
        (
            "One compositor serves preview, frame grabs and export. Native fixtures compare their rendered output.",
            &[],
        ),
    ),
    (
        "Export decode",
        (
            "AVAssetReader through the composition broker",
            &[code("AVAssetReader")],
        ),
        (
            "Exports decode and composite sequentially in output-frame order. They can also read a mixed PCM track.",
            &[],
        ),
    ),
    (
        "Encode",
        (
            "AVAssetWriter with a pixel-buffer adaptor and pooled IOSurface buffers",
            &[code("AVAssetWriter")],
        ),
        (
            "H.264, HEVC and ProRes. GIF export uses Splice's streaming GIF writer.",
            &[],
        ),
    ),
    (
        "Audio",
        (
            "AVMutableAudioMix and AVAssetReaderAudioMixOutput",
            &[
                code("AVMutableAudioMix"),
                code("AVAssetReaderAudioMixOutput"),
            ],
        ),
        (
            "Splice evaluates track gains and fades. AVFoundation plays the mix and supplies mixed audio for export.",
            &[],
        ),
    ),
    (
        "Waveforms, thumbnails",
        (
            "AVFoundation audio samples and AVAssetImageGenerator",
            &[code("AVAssetImageGenerator")],
        ),
        (
            "Waveform buckets and thumbnails are derived media. The viewer can reuse its prepared tiles and cached images.",
            &[],
        ),
    ),
    (
        "Transcription",
        (
            "SpeechAnalyzer and SpeechTranscriber on macOS 26 or later",
            &[code("SpeechAnalyzer"), code("SpeechTranscriber")],
        ),
        (
            "On-device results become word IDs with source-frame timing. The base app targets macOS 15. The transcript hash guards word anchors.",
            &[],
        ),
    ),
    (
        "OCR",
        (
            "Vision text recognition, VNRecognizeTextRequest",
            &[code("VNRecognizeTextRequest")],
        ),
        (
            "Frame analysis can return recognized text for an agent to inspect.",
            &[],
        ),
    ),
    (
        "Capture",
        (
            "ScreenCaptureKit SCShareableContent, SCContentFilter and SCStream to AVAssetWriter. SCStreamConfiguration selects microphone and system audio. SCScreenshotManager for stills, AVCaptureSession for camera, and a listen-only CGEventTap for input events.",
            &[
                code("SCShareableContent"),
                code("SCContentFilter"),
                code("SCStream"),
                code("AVAssetWriter"),
                code("SCStreamConfiguration"),
                code("SCScreenshotManager"),
                code("AVCaptureSession"),
                code("CGEventTap"),
            ],
        ),
        (
            "record.start, record.stop and splice record share the signed Splice.app capture path. Takes keep Recorder's layout and events.json.",
            &[
                code("record.start"),
                code("record.stop"),
                code("splice record"),
                code("Splice.app"),
                code("events.json"),
            ],
        ),
    ),
];

const BINDING_LEAD: Paragraph = (
    "apple_sys follows LocalWave's lw_apple_sys pattern. One C header defines the ABI, ARC Objective-C shims own native objects, and Rust exposes safe wrappers for each service. Small Swift shims expose SpeechAnalyzer and Translation through C.",
    &[
        code("apple_sys"),
        code("lw_apple_sys"),
        code("SpeechAnalyzer"),
    ],
);

const BINDING_CODE: &[&str] = &[
    "crates/apple_sys/",
    "  build.rs                 cc::Build with ARC and blocks, plus xcrun swiftc",
    "  native/platform.h        the C ABI, opaque handles and _Static_assert layouts",
    "  native/capture.m         ScreenCaptureKit sources, geometry and stills",
    "  native/recording.m       SCStream and AVCaptureSession to AVAssetWriter",
    "  native/events.m, native/input_events.m    native ring and listen-only CGEventTap",
    "  native/composition.m     AVFoundation decode, playback, mix and export",
    "  src/capture/recording.rs safe owner with Drop",
    "",
    "// native/platform.h, event ring excerpt",
    "typedef struct SpliceEventRing SpliceEventRing;",
    "typedef void (*SpliceWake)(uint64_t token);",
    "SpliceEventRing *splice_events_create(size_t capacity, SpliceWake wake, uint64_t token, char **error);",
    "size_t splice_events_peek(SpliceEventRing *ring, SpliceEvent *out, size_t capacity);",
    "bool splice_events_ack(SpliceEventRing *ring, size_t count);",
    "void splice_events_release(SpliceEventRing *ring);",
];

const BINDING_NOTES: Paragraph = (
    "Native callbacks push into a bounded ring and can signal a wake token. Rust peeks into caller-owned storage and acknowledges the batch once consumed. The shims return diagnostic strings and release opaque ownership through their C entry points.",
    &[],
);

fn media_section(ui: &mut Ui, width: f32) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        paragraph(ui, width, MEDIA_LEAD);
        block(ui, 6.0, 0.0, &mut |ui| {
            let table_width = width.max(486.9);
            let extra = table_width - 486.9;
            let first = 101.8 + 0.0209 * extra;
            let second = 230.3 + 0.4874 * extra;
            let draw = |ui: &mut Ui| {
                detail_table(
                    ui,
                    "Native media",
                    ["Concern", "Framework", "Notes"],
                    [first, second, table_width - first - second],
                    MEDIA_ROWS,
                    None,
                );
            };
            if table_width > width {
                let scroll = horizontal_scroll(
                    ui,
                    "media-table-scroll",
                    "Native media table",
                    Look::NONE.fill(BG),
                );
                ui.within(&scroll, &mut |ui| {
                    ui.column(Node::column().w_px(table_width), draw);
                });
            } else {
                draw(ui);
            }
        });
        heading(ui, "Binding style: LocalWave's lw_apple_sys pattern");
        paragraph(ui, width, BINDING_LEAD);
        block(ui, 8.0, 6.0, &mut |ui| {
            code_panel(ui, "native-capture-abi", "Native capture ABI", BINDING_CODE)
        });
        paragraph(ui, width, BINDING_NOTES);
    });
}

const RUST_LEAD: Paragraph = (
    "Splice adapts rays' AGENTS.md and tools/gate. The workspace gate checks source shape and crate dependency policy.",
    &[code("AGENTS.md"), code("tools/gate")],
);

const RUST_CARDS: &[(&str, &[Paragraph])] = &[
    (
        "Toolchain and lints",
        &[
            (
                "rust-toolchain.toml pins 1.99.1, the minimal profile, rustfmt and clippy. The workspace uses Rust edition 2024.",
                &[
                    code("rust-toolchain.toml"),
                    code("1.99.1"),
                    code("rustfmt"),
                    code("clippy"),
                ],
            ),
            (
                "Workspace lints deny warnings and unsafe code by default. Product crates inherit the lints with workspace = true.",
                &[code("workspace = true")],
            ),
            (
                "Use make check for local validation and the Mac gate for native work.",
                &[code("make check")],
            ),
            (
                "Mark deliberate shortcuts with ponytail: and an upgrade trigger.",
                &[code("ponytail:")],
            ),
        ],
    ),
    (
        "Shape rules (gate)",
        &[
            (
                "unsafe belongs in *_sys crates. The workspace lint and source gate enforce that boundary.",
                &[code("unsafe"), code("*_sys")],
            ),
            (
                "The gate rejects async, macro_rules!, proc macros, impl Trait and generic declarations in product source.",
                &[code("async"), code("macro_rules!"), code("impl Trait")],
            ),
            (
                "Derives are limited to Clone, Copy, Debug, PartialEq, Eq, Hash and Default.",
                &[],
            ),
            (
                "Return errors for control flow. The gate rejects ignored tests and HashMap.",
                &[code("HashMap")],
            ),
        ],
    ),
    (
        "Dependencies",
        &[
            (
                "Use std and vetted workspace crates. The gate allows cc for builds and checks a per-crate dependency list.",
                &[code("std"), code("cc")],
            ),
            (
                "Splice owns its JSON reader, writer, hashing and command metadata. Dependency additions need measured evidence.",
                &[],
            ),
        ],
    ),
    (
        "UI and threads",
        &[
            (
                "The UI reads snapshots and returns intents. The project writer applies registry ops after the frame.",
                &[],
            ),
            (
                "Window, Metal and AppKit work stays on the initial thread. Existing services and callbacks handle background work.",
                &[],
            ),
        ],
    ),
];

const REGISTRY_LEAD: Paragraph = (
    "The registry pairs plain data with a decoder per command. Typed Ops feed the core's single apply path.",
    &[code("Ops"), code("apply")],
);

const REGISTRY_CODE: &[&str] = &[
    "pub struct Command {",
    "    pub id: &'static str, pub title: &'static str, pub group: &'static str,",
    "    pub params: &'static [(&'static str, &'static str)],  // names and types, ! means required",
    "    pub ui: &'static [&'static str], pub example: &'static str,",
    "    // Also carries the risk, shortcut, release tier and decoder.",
    "}",
    "pub const COMMANDS: &[Command] = &[ /* rows in stable order */ ];",
    "pub enum Op { ItemSplit { item: Target, at: FrameArg }, /* … */ }",
    "pub fn apply(project: &mut Project, ops: &[Json], source: Source, dry_run: bool)",
    "    -> Result<ApplyReport, ApplyError> { /* stage and publish one batch */ }",
];

const REGISTRY_NOTES: &[Paragraph] = &[
    (
        "splice_json reads the JSON. The core resolves cmd in COMMANDS and decodes it to Op. Tests check that each row's documented example decodes to a distinct variant.",
        &[
            code("splice_json"),
            code("cmd"),
            code("COMMANDS"),
            code("Op"),
        ],
    ),
    (
        "commands.describe serves the live catalog. The MCP TypeScript generator reads the command table and tool schemas. The native Spec also uses the live rows.",
        &[code("commands.describe")],
    ),
];

fn rust_section(ui: &mut Ui, width: f32, narrow: bool, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        paragraph(ui, width, RUST_LEAD);
        block(ui, 6.0, 0.0, &mut |ui| {
            let grid = Node::column().w(Size::Grow).gap(12.0);
            ui.column(grid, |ui| {
                let columns = if narrow { 1 } else { 2 };
                let card_width = if narrow { width } else { (width - 12.0) * 0.5 };
                for row in RUST_CARDS.chunks(columns) {
                    ui.row(
                        Node::row().w(Size::Grow).gap(12.0).align(Align::Stretch),
                        |ui| {
                            for &(name, items) in row {
                                note_card(ui, card_width, name, name, items);
                            }
                        },
                    );
                }
            });
        });
        heading(ui, "The registry without macros");
        paragraph(ui, width, REGISTRY_LEAD);
        block(ui, 8.0, 8.0, &mut |ui| {
            code_panel(
                ui,
                "rust-command-registry",
                "Rust command registry excerpt",
                REGISTRY_CODE,
            )
        });
        points(ui, width, REGISTRY_NOTES);
        block(ui, 8.0, 0.0, &mut |ui| links(ui, &["s-registry"], picked));
    });
}

const ALLOC_LEAD: Paragraph = (
    "Steady state allocates nothing in the paths covered by the warm probes. Playback, scrubbing, timeline interaction and evaluation reuse prepared storage. Growing the project, importing media and preparing a revision can allocate. Counts cover Rust and a counting PXUI library.",
    &[strong("Steady state allocates nothing")],
);

const ALLOC_POINTS: &[Paragraph] = &[
    (
        "Counting allocator. mem_sys records alloc_count, alloc_bytes_total, local_alloc_count and local_peak_bytes. Tests call local_reset before a warmed pass. The development profile builds the allocator at opt-level = 3.",
        &[
            strong("Counting allocator."),
            code("mem_sys"),
            code("alloc_count"),
            code("alloc_bytes_total"),
            code("local_alloc_count"),
            code("local_peak_bytes"),
            code("local_reset"),
            code("opt-level = 3"),
        ],
    ),
    (
        "Tests assert it. Prepared render plans, trim, move and set with a warmed ApplyWorkspace, timeline interactions, and the shared-texture window path have zero-allocation checks.",
        &[strong("Tests assert it."), code("ApplyWorkspace")],
    ),
    (
        "Typed IDs and arenas. Clips and items keep dense arena columns and ordered views. Lanes and markers keep stable typed IDs. The wire uses prefixed IDs such as clp_ and ovl_.",
        &[strong("Typed IDs and arenas."), code("clp_"), code("ovl_")],
    ),
    (
        "Pools. The viewer reuses three prepared Program texture slots. The compositor reuses Metal uniform pages. AVFoundation owns decoded pixel buffers and gives the export writer a pixel-buffer pool.",
        &[strong("Pools.")],
    ),
    (
        "Boundary buffers. FramePlan and edit workspace storage are reused after warmup. JSON writes can use caller storage, and native labels use stack formatting.",
        &[strong("Boundary buffers."), code("FramePlan")],
    ),
    (
        "Memory budgets. Prepared slots and layer counts have explicit limits. The v13 design's idle memory target was under 150 MB, from Recorder AC-APP-3.",
        &[strong("Memory budgets.")],
    ),
];

fn alloc_section(ui: &mut Ui, width: f32) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        paragraph(ui, width, ALLOC_LEAD);
        block(ui, 6.0, 0.0, &mut |ui| points(ui, width, ALLOC_POINTS));
    });
}

const BUILD_POINTS: &[Paragraph] = &[
    (
        "Dev profile. Cargo.toml uses opt-level = 1, debug = 0, incremental = true, codegen-units = 256 and overflow-checks = true. Warm incremental builds share the same target directory.",
        &[
            strong("Dev profile."),
            code("Cargo.toml"),
            code("opt-level = 1"),
            code("debug = 0"),
            code("incremental = true"),
            code("codegen-units = 256"),
            code("overflow-checks = true"),
        ],
    ),
    (
        "Per-crate override. mem_sys uses opt-level = 3 in dev because the allocator runs on every allocation in a counting build.",
        &[
            strong("Per-crate override."),
            code("mem_sys"),
            code("opt-level = 3"),
        ],
    ),
    (
        "Release. codegen-units = 16, lto = \"thin\" and debug = \"line-tables-only\".",
        &[
            strong("Release."),
            code("codegen-units = 16"),
            code("lto = \"thin\""),
            code("debug = \"line-tables-only\""),
        ],
    ),
    (
        "Historical build budgets. v13 cites rays' tools/buildtime limits of 1 s for a leaf, 5 s for a middle crate and 15 s for the top crate after a one-file edit. Splice's workspace gate is make check.",
        &[
            strong("Historical build budgets."),
            code("tools/buildtime"),
            code("make check"),
        ],
    ),
    (
        "Graph shape. Keep portable core crates separate from native *_sys leaves. Reuse the warm shared target/ directory and measure focused checks on the normal profile.",
        &[strong("Graph shape."), code("*_sys"), code("target/")],
    ),
    (
        "macOS links through PXUI's cc-stack.sh, which uses the system linker, a 256 MB main-thread stack and local dylib rpaths. Native checks use the shared Apple Silicon Mac mini. Linux covers portable checks.",
        &[code("cc-stack.sh")],
    ),
];

const TEST_CALLOUT: Paragraph = (
    "The core is tested through the same surface agents use. Integration tests build JSON or Rust ops and call apply, or exercise JSON-RPC through splice_mcp. Both paths check atomic edits and revisions. Native window tests cover UI intents and frame delivery.",
    &[
        strong("The core is tested through the same surface agents use."),
        code("apply"),
        code("splice_mcp"),
    ],
);

const TEST_ROWS: &[DetailRow] = &[
    (
        "Unit",
        (
            "Anchor math, settle and bump, frame resolution and the JSON codec.",
            &[],
        ),
        (
            "In-crate test modules, including src/*_tests.rs.",
            &[code("src/*_tests.rs")],
        ),
    ),
    (
        "Core integration",
        (
            "Input project and ops exercise atomic edits and diff or snapshot results. Every documented COMMANDS example must decode.",
            &[code("COMMANDS")],
        ),
        (
            "crates/splice_core/src/edit and crates/splice_core/src/commands/tests.rs",
            &[
                code("crates/splice_core/src/edit"),
                code("crates/splice_core/src/commands/tests.rs"),
            ],
        ),
    ),
    (
        "Invariants",
        (
            "Seeded operations and fixture cases cover ordered lanes, anchor resolution, undo and deterministic journal replay.",
            &[],
        ),
        (
            "splice_testkit and core edit and history tests",
            &[code("splice_testkit")],
        ),
    ),
    (
        "MCP conformance",
        (
            "Transcript fixtures cover tool ordering, schemas and error cases. CLI tests exercise stale revisions, idempotency and dry runs.",
            &[],
        ),
        (
            "fixtures/mcp, splice_mcp in-process tests and splice_cli integration tests",
            &[code("fixtures/mcp"), code("splice_mcp"), code("splice_cli")],
        ),
    ),
    (
        "Parity",
        (
            "Live tool and command catalogs are checked alongside the registered shortcuts.",
            &[],
        ),
        ("Spec parity report and registry tests", &[]),
    ),
    (
        "Media (macOS)",
        (
            "Small decode and encode fixtures, shared textures, preview and export comparisons, and audio tests.",
            &[],
        ),
        (
            "Serialized native cases in make mac-check",
            &[code("make mac-check")],
        ),
    ),
    (
        "Performance",
        (
            "Warm allocation counts and elapsed-time assertions for selected apply, eval, compose and export cases.",
            &[],
        ),
        (
            "Focused cargo t budget or native test filters",
            &[code("cargo t")],
        ),
    ),
];

const TEST_RUNNER: &[Paragraph] = &[
    (
        "cargo t only. The normal limits are 5 s per case and 60 s per run. The runner serializes GPU and window cases, lists the slowest tests and records target/t-times.tsv.",
        &[strong("cargo t only."), code("target/t-times.tsv")],
    ),
    (
        "The designated slow profile uses RAYS_SLOW=1 or --profile slow, with 60 s per case and 600 s per run. Keep normal regressions within the normal deadlines.",
        &[code("RAYS_SLOW=1"), code("--profile slow")],
    ),
    (
        "Use focused portable checks locally and make mac-check for the native tests and signed-bundle checks. Keep deadlines and assertions intact when diagnosing a regression.",
        &[code("make mac-check")],
    ),
];

fn testing_section(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(Node::column().w(Size::Grow), |ui| {
        block(ui, 0.0, 8.0, &mut |ui| {
            let w = width.min(576.0);
            let callout = ui.column(
                Node::column()
                    .w_px(w)
                    .padding(Edges::new(14.0, 8.0, 12.0, 8.0))
                    .role(Role::Group)
                    .look(Look::NONE.fill(CONTROL).border(ACCENT, 2.0).sides(LEFT)),
                |ui| {
                    rich(
                        ui,
                        (w - 26.0).max(1.0),
                        DOC_BODY.line_height(16.8),
                        Role::Paragraph,
                        MUTED,
                        16.8,
                        TEST_CALLOUT,
                    )
                },
            );
            ui.a11y_name(&callout, "Testing contract");
        });
        detail_table(
            ui,
            "Test layers",
            ["Layer", "What", "Where"],
            detail_columns(width, [90.2, 179.9], [0.019, 0.765]),
            TEST_ROWS,
            Some(picked),
        );
        heading(ui, "Runner and hard timeouts");
        points(ui, width, TEST_RUNNER);
    });
}

const PERF_ROWS: &[DetailRow] = &[
    (
        "Apply one op, 2k items",
        (
            "p99 under 1 ms. ApplyWorkspace checks zero allocations on selected warm edits.",
            &[code("ApplyWorkspace")],
        ),
        ("splice_core budget cases", &[code("splice_core")]),
    ),
    (
        "timeline.snapshot, 2k items",
        ("v13 target under 2 ms for a concise snapshot.", &[]),
        ("MCP snapshot fixtures and size checks", &[]),
    ),
    (
        "Evaluate one frame",
        (
            "p99 under 100 µs, zero warm allocations (2,000 keys over 200 titles). v13 target under 50 µs.",
            &[],
        ),
        ("splice_eval warm-frame test", &[code("splice_eval")]),
    ),
    (
        "Scrub: input to glass",
        (
            "v13 target under one display frame cached and under three within a GOP on a cold seek.",
            &[],
        ),
        ("Native paused-seek frame test", &[]),
    ),
    (
        "Playback",
        (
            "Warm 4K compose p90 under 16 ms. v13 targeted two 4K60 streams on M1.",
            &[],
        ),
        ("4K compose gate, separate decode and readback timings", &[]),
    ),
    (
        "Timeline view",
        (
            "Warm interactions have zero-allocation probes. v13 targeted 120 fps with 2,000 blocks.",
            &[],
        ),
        ("PXUI timeline gesture and paint tests", &[]),
    ),
    (
        "Export",
        (
            "8 s of 1080p60 HEVC in under 4 s. v13 targeted 60 s of H.264 in under 30 s.",
            &[],
        ),
        ("Native export budget case", &[]),
    ),
    (
        "Audio callback",
        (
            "v13 mixer target zero allocations, zero locks and under 25% of the buffer period.",
            &[],
        ),
        ("Current AVFoundation gain, mix and export checks", &[]),
    ),
    (
        "Idle",
        ("v13 target under 1% CPU and 150 MB RAM.", &[]),
        ("Recorder AC-APP-3", &[]),
    ),
    (
        "Cold launch to editable",
        ("v13 target under 400 ms for a 10-minute project.", &[]),
        ("Current signed-bundle open smoke", &[]),
    ),
    (
        "Rebuild after one-file edit",
        ("v13 target leaf 1 s, middle 5 s and top 15 s.", &[]),
        ("rays buildtime reference", &[code("buildtime")]),
    ),
];

fn performance_section(ui: &mut Ui, width: f32) {
    detail_table(
        ui,
        "Performance budgets",
        ["Path", "Budget", "Measured by"],
        detail_columns(width, [135.3, 197.1], [0.1374, 0.6485]),
        PERF_ROWS,
        None,
    );
}

#[cfg(test)]
mod workflow_tests {
    use super::super::Selection;
    use super::tests::draw;
    use super::{
        ALLOC_LEAD, ALLOC_POINTS, BINDING_CODE, BUILD_POINTS, MEDIA_ROWS, PERF_ROWS, REGISTRY_CODE,
        RUST_CARDS, TEST_ROWS,
    };
    use pxui::{Event, FrameIn, Rect, Role, Ui, testing};

    const TITLES: &[(&str, &str)] = &[
        ("Media on macOS", "Media on macOS (no ffmpeg)"),
        ("Rust rules", "Rust rules"),
        ("Zero allocation", "Zero allocation"),
        ("Build speed", "Build speed"),
        ("Testing", "Testing"),
        ("Performance budgets", "Performance budgets"),
    ];

    #[test]
    fn engineering_workflows_960() {
        workflows(960.0);
    }

    #[test]
    fn engineering_workflows_800() {
        workflows(800.0);
    }

    #[test]
    fn engineering_workflows_480() {
        workflows(480.0);
    }

    fn workflows(width: f32) {
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let input = FrameIn::new(width, 640.0);
        let mut selection = Selection::default();
        let content = if width > 820.0 {
            width - 284.0
        } else {
            width - 32.0
        };
        let left = if width > 820.0 { 252.0 } else { 16.0 };
        let top = if width > 820.0 { 55.0 } else { 21.0 };
        for _ in 0..2 {
            draw(&mut ui, &input, &mut selection);
        }
        mem_sys::set_peer(pxui::library_counter);

        for (index, &(nav, title)) in TITLES.iter().enumerate() {
            activate(&mut ui, &input, &mut selection, width, Role::Button, nav);
            let title_rect = bounds(&ui, width, title, Role::Heading, title);
            assert_near(title_rect.x, left);
            assert_near(title_rect.y, top);
            if let Some(&(_, next)) = TITLES.get(index + 1) {
                bounds(&ui, width, title, Role::Heading, next);
            }
            match index {
                0 => {
                    let table = bounds(&ui, width, title, Role::Table, "Native media");
                    assert_near(table.w, content.max(486.9));
                    table_rows(&ui, table, MEDIA_ROWS.len());
                    let binding = bounds(
                        &ui,
                        width,
                        title,
                        Role::Heading,
                        "Binding style: LocalWave's lw_apple_sys pattern",
                    );
                    let code = bounds(&ui, width, title, Role::ScrollArea, "Native capture ABI");
                    assert_near(code.w, content);
                    assert_near(code.h, BINDING_CODE.len() as f32 * 17.6 + 18.0);
                    assert!(binding.y >= table.y + table.h);
                    if width == 480.0 {
                        let scroll =
                            bounds(&ui, width, title, Role::ScrollArea, "Native media table");
                        assert_near(scroll.w, content);
                        scroll_at(
                            &mut ui,
                            &input,
                            &mut selection,
                            (220.0, f64::from(table.y + 70.0)),
                            (-1.0, 0.0),
                        );
                        let shifted = find(&ui, Role::Table, "Native media");
                        assert_near(table.x - shifted.x, 24.0);
                        assert_near(table.y, shifted.y);
                    }
                }
                1 => {
                    let cards = [0, 1, 2, 3]
                        .map(|i| bounds(&ui, width, title, Role::Group, RUST_CARDS[i].0));
                    if width > 820.0 {
                        for pair in [&cards[..2], &cards[2..]] {
                            assert_near(pair[0].y, pair[1].y);
                            assert_near(pair[0].h, pair[1].h);
                            assert_near(pair[0].w, 332.0);
                            assert_near(pair[1].x - pair[0].x - pair[0].w, 12.0);
                        }
                        assert_near(cards[2].y - cards[0].y - cards[0].h, 12.0);
                    } else {
                        for pair in cards.windows(2) {
                            assert_near(pair[1].x, pair[0].x);
                            assert_near(pair[0].w, content);
                            assert_near(pair[1].y - pair[0].y - pair[0].h, 12.0);
                        }
                    }
                    let heading = bounds(
                        &ui,
                        width,
                        title,
                        Role::Heading,
                        "The registry without macros",
                    );
                    let code = bounds(
                        &ui,
                        width,
                        title,
                        Role::ScrollArea,
                        "Rust command registry excerpt",
                    );
                    assert!(heading.y >= cards[3].y + cards[3].h);
                    assert_near(code.h, REGISTRY_CODE.len() as f32 * 17.6 + 18.0);
                }
                2 => {
                    let lead = bounds(&ui, width, title, Role::Paragraph, ALLOC_LEAD.0);
                    assert_near(lead.w, content.min(562.0));
                    bullets(&ui, width, title, ALLOC_POINTS);
                }
                3 => bullets(&ui, width, title, BUILD_POINTS),
                4 => {
                    let callout = bounds(&ui, width, title, Role::Group, "Testing contract");
                    assert_near(callout.w, content.min(576.0));
                    let table = bounds(&ui, width, title, Role::Table, "Test layers");
                    assert_near(table.w, content);
                    table_rows(&ui, table, TEST_ROWS.len());
                    bounds(&ui, width, title, Role::Heading, "Runner and hard timeouts");
                    let link = find(&ui, Role::Link, "Parity check");
                    assert!(link.y >= table.y && link.y + link.h <= table.y + table.h);
                }
                _ => {
                    let table = bounds(&ui, width, title, Role::Table, "Performance budgets");
                    assert_near(table.w, content);
                    table_rows(&ui, table, PERF_ROWS.len());
                }
            }
            let idle = &mut || draw(&mut ui, &input, &mut selection);
            for _ in 0..8 {
                idle();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(idle);
            println!("WARM|{width}|{title}|0 allocations");
        }

        activate(
            &mut ui,
            &input,
            &mut selection,
            width,
            Role::Button,
            "Testing",
        );
        activate(
            &mut ui,
            &input,
            &mut selection,
            width,
            Role::Link,
            "Parity check",
        );
        assert_near(find(&ui, Role::Heading, "Parity check").y, top);
        println!("LINK|{width}|Testing|Parity check");
        activate(
            &mut ui,
            &input,
            &mut selection,
            width,
            Role::Button,
            "Rust rules",
        );
        activate(
            &mut ui,
            &input,
            &mut selection,
            width,
            Role::Link,
            "Command registry",
        );
        assert_near(find(&ui, Role::Heading, "Command registry").y, top);
        println!("LINK|{width}|Rust rules|Command registry");
    }

    fn table_rows(ui: &Ui, table: Rect, count: usize) {
        let rows = ui
            .a11y_nodes()
            .iter()
            .filter(|node| {
                node.is_role(Role::Row)
                    && node.rect.y >= table.y - 0.1
                    && node.rect.y + node.rect.h <= table.y + table.h + 0.1
                    && (node.rect.x - table.x).abs() < 0.1
            })
            .count();
        assert_eq!(rows, count + 1);
    }

    fn bullets(ui: &Ui, width: f32, title: &str, items: &[super::Paragraph]) {
        let mut previous = None;
        for &(text, _) in items {
            let r = bounds(ui, width, title, Role::Paragraph, text);
            if let Some(y) = previous {
                assert_near(r.y, y);
            }
            previous = Some(r.y + r.h);
        }
    }

    fn scroll_at(
        ui: &mut Ui,
        input: &FrameIn,
        selection: &mut Selection,
        origin: (f64, f64),
        delta: (f64, f64),
    ) {
        draw(
            ui,
            &input.events(&[
                Event::pointer(Event::POINTER_MOVED, 0, origin.0, origin.1),
                Event::pointer(Event::SCROLLED, 0, delta.0, delta.1),
            ]),
            selection,
        );
        draw(ui, input, selection);
    }

    fn activate(
        ui: &mut Ui,
        input: &FrameIn,
        selection: &mut Selection,
        width: f32,
        role: Role,
        name: &str,
    ) {
        let source = super::super::SECTIONS[selection.section].2;
        let locate = |ui: &Ui| {
            if role == Role::Link {
                let heading = find(ui, Role::Heading, source);
                return ui
                    .a11y_nodes()
                    .iter()
                    .filter(|node| {
                        node.is_role(role)
                            && ui.a11y_name_of(node) == name
                            && node.rect.y >= heading.y
                    })
                    .min_by(|a, b| a.rect.y.total_cmp(&b.rect.y))
                    .unwrap_or_else(|| panic!("missing link {name} in {source}"))
                    .rect;
            }
            find(ui, role, name)
        };
        if role == Role::Button && width <= 820.0 && selection.scroll > 0.0 {
            scroll_at(
                ui,
                input,
                selection,
                (f64::from(width - 12.0), 290.0),
                (0.0, f64::from(selection.scroll / 24.0 + 1.0)),
            );
        }
        let r = locate(ui);
        let x = r.x + r.w / 2.0;
        let y = r.y + r.h / 2.0;
        if role == Role::Button && width <= 820.0 && !(6.0..width - 6.0).contains(&x) {
            scroll_at(
                ui,
                input,
                selection,
                (120.0, 48.0),
                (f64::from((width / 2.0 - x) / 24.0), 0.0),
            );
        } else if !(90.0..625.0).contains(&y) && (role == Role::Link || width > 820.0) {
            let origin_x = if role == Role::Button {
                120.0
            } else {
                f64::from(width - 12.0)
            };
            scroll_at(
                ui,
                input,
                selection,
                (origin_x, 300.0),
                (0.0, f64::from((300.0 - y) / 24.0)),
            );
        }
        let r = locate(ui);
        let x = r.x + r.w / 2.0;
        let y = r.y + r.h / 2.0;
        assert!(
            (0.0..width).contains(&x) && (0.0..640.0).contains(&y),
            "{name}: {r:?}"
        );
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            draw(
                ui,
                &input.events(&[Event::pointer(kind, 0, x.into(), y.into())]),
                selection,
            );
        }
        draw(ui, input, selection);
    }

    fn assert_near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.2, "{actual} vs {expected}");
    }

    fn find(ui: &Ui, role: Role, name: &str) -> Rect {
        testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {role:?} {name}"))
    }

    fn bounds(ui: &Ui, width: f32, section: &str, role: Role, name: &str) -> Rect {
        let r = find(ui, role, name);
        println!(
            "GEOM|{width}|{section}|{role:?}|{name}|{}|{}|{}|{}",
            r.x, r.y, r.w, r.h
        );
        r
    }
}
