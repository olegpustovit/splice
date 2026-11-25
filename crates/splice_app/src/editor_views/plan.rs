use pxui::{
    Label, Node, Pressable, Role, Ui,
    draw::flags::rect::BOTTOM,
    geom::Edges,
    ui::{Align, Look, Size},
};

use super::{
    BG, DOC_BODY, DOC_LEDE, HEADING, INK, LINE, MOTION, MUTED, PANEL, QUIET, SAGE, SECTION_CAPTION,
    SECTIONS, STATUS_LINK, SUBHEADING, TABLE_EDGE, TABLE_HEAD, TABLE_ROW, TABLE_TEXT, doc_label,
    spec_table_width, table_cell, table_row, table_tag,
};

const PROSE: f32 = 562.0;
const LEDE: f32 = 624.0;

pub(super) fn document(
    ui: &mut Ui,
    anchor: &str,
    narrow: bool,
    picked: &mut Option<usize>,
    external: &mut Option<&'static str>,
) -> bool {
    let width = spec_table_width(ui, narrow);
    match anchor {
        "s-m1" => {
            ui.row(
                Node::row()
                    .w(Size::Grow)
                    .gap(8.0)
                    .wrap()
                    .align(Align::Center),
                |ui| {
                    doc_label(ui, M1_TITLE, HEADING.w(Size::Fit), INK);
                    table_tag(ui, M1_STATUS, SAGE, SAGE, false);
                },
            );
            history(ui, "Historical plan from 18 November 2026", picked);
            prose(ui, M1_LEDE, DOC_LEDE, LEDE, INK);
            table(ui, width, SURFACE_TABLE, picked);
            doc_label(ui, "Board", SUBHEADING, INK);
            table(ui, width, BOARD_TABLE, picked);
            cards(ui, narrow, width, &CARDS[..2]);
            doc_label(ui, "Recording, almost headless", SUBHEADING, INK);
            prose(ui, M1_RECORDING, DOC_BODY, PROSE, MUTED);
            cards(ui, narrow, width, &CARDS[2..]);
            prose(ui, M1_SHORTCUTS, DOC_BODY, PROSE, MUTED);
        }
        "s-roadmap" => {
            doc_label(ui, ROADMAP_TITLE, HEADING, INK);
            history(ui, "Historical v13 milestones and exit criteria", picked);
            table(ui, width, MILESTONE_TABLE, picked);
        }
        "s-open" => {
            doc_label(ui, OPEN_TITLE, HEADING, INK);
            doc_label(
                ui,
                "Questions recorded in the v13 plan",
                SECTION_CAPTION,
                QUIET,
            );
            bullets(ui, QUESTIONS, width);
        }
        "s-research" => {
            doc_label(ui, RESEARCH_TITLE, HEADING, INK);
            doc_label(
                ui,
                "Research recorded for the November 2026 v13 spec",
                SECTION_CAPTION,
                QUIET,
            );
            doc_label(ui, "How editors arrange layers", SUBHEADING, INK);
            table(ui, width, LAYER_TABLE, picked);
            doc_label(ui, "Evidence", SUBHEADING, INK);
            bullets(ui, EVIDENCE, width);
            doc_label(ui, "Candidates considered", SUBHEADING, INK);
            table(ui, width, CANDIDATE_TABLE, picked);
        }
        "s-sources" => {
            doc_label(ui, SOURCES_TITLE, HEADING, INK);
            source_links(ui, external);
            doc_label(ui, "Codebases", SECTION_CAPTION.role(Role::Heading), QUIET);
            prose(ui, CODEBASES, DOC_BODY, PROSE, MUTED);
        }
        _ => return false,
    }
    true
}

pub(super) fn open(url: &str) {
    match std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
    {
        Ok(status) if status.success() => (),
        Ok(status) => eprintln!("splice: opening {url} returned {status}"),
        Err(error) => eprintln!("splice: could not open {url}: {error}"),
    }
}

fn history(ui: &mut Ui, text: &str, picked: &mut Option<usize>) {
    ui.row(
        Node::row()
            .w(Size::Grow)
            .gap(12.0)
            .wrap()
            .align(Align::Center),
        |ui| {
            doc_label(ui, text, SECTION_CAPTION.w(Size::Fit), QUIET);
            jump(
                ui,
                "s-registry",
                "Current command registry",
                DOC_BODY,
                Size::Fit,
                picked,
            );
        },
    );
}

fn prose(ui: &mut Ui, text: &str, style: Label, limit: f32, ink: pxui::Rgba) {
    ui.column(Node::column().w(Size::Grow).max_w(limit), |ui| {
        doc_label(ui, text, style.role(Role::Paragraph), ink);
    });
}

fn bullets(ui: &mut Ui, points: &[&str], width: f32) {
    let width = (width - 20.0).clamp(1.0, PROSE);
    ui.column(
        Node::column().w(Size::Grow).pad_y(6.0).role(Role::List),
        |ui| {
            for &point in points {
                ui.row(
                    Node::row()
                        .w(Size::Grow)
                        .align(Align::Start)
                        .role(Role::ListItem),
                    |ui| {
                        doc_label(
                            ui,
                            "\u{2022}",
                            DOC_BODY.w_px(20.0).role(Role::Unknown),
                            MUTED,
                        );
                        doc_label(ui, point, DOC_BODY.w_px(width).role(Role::Paragraph), MUTED);
                    },
                );
            }
        },
    );
}

fn cards(ui: &mut Ui, narrow: bool, width: f32, cards: &[(&str, &[&str])]) {
    let card_width = if narrow { width } else { (width - 12.0) * 0.5 };
    let draw = |ui: &mut Ui| {
        for &(title, points) in cards {
            ui.column(
                Node::column()
                    .w_px(card_width)
                    .padding(Edges::new(13.0, 9.0, 13.0, 9.0))
                    .look(Look::NONE.fill(PANEL).border(LINE, 1.0)),
                |ui| {
                    doc_label(ui, title, SECTION_CAPTION.role(Role::Heading), QUIET);
                    bullets(ui, points, card_width - 26.0);
                },
            );
        }
    };
    if narrow {
        ui.column(Node::column().w(Size::Grow).gap(12.0), draw);
    } else {
        ui.row(Node::row().w(Size::Grow).gap(12.0), draw);
    }
}

fn link(ui: &mut Ui, key: &'static str, title: &str, style: Label, width: Size) -> bool {
    let response = ui.pressable(
        key,
        Node::column().w(width).role(Role::Link),
        &Pressable::DEFAULT.look(&STATUS_LINK),
        |ui, _| doc_label(ui, title, style.w(width).role(Role::Unknown), MOTION),
    );
    ui.a11y_name(&response.handle, title);
    response.clicked
}

fn jump(
    ui: &mut Ui,
    anchor: &'static str,
    title: &str,
    style: Label,
    width: Size,
    picked: &mut Option<usize>,
) {
    if link(ui, anchor, title, style, width) {
        *picked = SECTIONS.iter().position(|section| section.0 == anchor);
    }
}

fn source_links(ui: &mut Ui, external: &mut Option<&'static str>) {
    for &(title, sources) in SOURCE_GROUPS {
        doc_label(ui, title, SECTION_CAPTION.role(Role::Heading), QUIET);
        ui.column(Node::column().w(Size::Grow).max_w(PROSE), |ui| {
            ui.row(Node::row().w(Size::Grow).gap(7.2).wrap(), |ui| {
                for (index, &(label, url)) in sources.iter().enumerate() {
                    if index > 0 {
                        doc_label(ui, "\u{b7}", DOC_BODY.w(Size::Fit), MUTED);
                    }
                    if link(ui, url, label, DOC_BODY, Size::Fit) {
                        *external = Some(url);
                    }
                }
            });
        });
    }
}

#[derive(Clone, Copy)]
struct Table {
    title: &'static str,
    headers: &'static [&'static str],
    rows: &'static [&'static [&'static str]],
    widths: &'static [[f32; 3]],
}

const SURFACE_TABLE: Table = Table {
    title: "0.1 surfaces",
    headers: SURFACES_HEAD,
    rows: SURFACES,
    widths: &[[127.06, 185.89, 209.64]],
};
const BOARD_TABLE: Table = Table {
    title: "Board",
    headers: BOARD_HEAD,
    rows: BOARD,
    widths: &[
        [98.84, 119.45, 127.77],
        [214.48, 401.28, 476.66],
        [53.09, 61.33, 64.66],
    ],
};
const MILESTONE_TABLE: Table = Table {
    title: "Roadmap",
    headers: MILESTONES_HEAD,
    rows: MILESTONES,
    widths: &[[83.5, 94.34, 98.7], [255.28, 431.83, 503.06]],
};
const LAYER_TABLE: Table = Table {
    title: "How editors arrange layers",
    headers: LAYERS_HEAD,
    rows: LAYERS,
    widths: &[[111.64, 116.83, 118.92], [154.33, 248.91, 287.06]],
};
const CANDIDATE_TABLE: Table = Table {
    title: "Candidates considered",
    headers: CANDIDATES_HEAD,
    rows: CANDIDATES,
    widths: &[
        [89.39, 102.61, 107.94],
        [121.42, 191.89, 220.31],
        [117.41, 182.45, 208.7],
    ],
};

// Interpolate the cached browser's column widths at the three reference widths.
fn measured(width: f32, [compact, desktop, medium]: [f32; 3]) -> f32 {
    if width < 448.0 {
        compact * width / 448.0
    } else if width <= 676.0 {
        compact + (desktop - compact) * ((width - 448.0) / 228.0)
    } else if width <= 768.0 {
        desktop + (medium - desktop) * ((width - 676.0) / 92.0)
    } else {
        medium * width / 768.0
    }
}

fn table(ui: &mut Ui, width: f32, table: Table, picked: &mut Option<usize>) {
    let mut columns = [("", Size::Grow); 4];
    for (index, &heading) in table.headers.iter().enumerate() {
        let size = table
            .widths
            .get(index)
            .map_or(Size::Grow, |&points| Size::Px(measured(width, points)));
        columns[index] = (heading, size);
    }
    ui.column(Node::column().w(Size::Grow).pad_y(8.0), |ui| {
        let body = ui.node("plan-table", Node::column().w(Size::Grow).role(Role::Table));
        ui.a11y_name(&body, table.title);
        ui.within(&body, &mut |ui| {
            ui.row(
                TABLE_ROW
                    .min_h(26.5)
                    .padding(Edges::new(0.0, 6.0, 0.0, 6.5))
                    .look(Look::NONE.fill(BG).border(TABLE_EDGE, 1.0).sides(BOTTOM)),
                |ui| {
                    for &(title, width) in &columns[..table.headers.len()] {
                        table_cell(ui, width, &mut |ui| doc_label(ui, title, TABLE_HEAD, QUIET));
                    }
                },
            );
            for (index, row) in table.rows.iter().enumerate() {
                table_row(ui, &mut |ui| {
                    for (cell, &text) in row.iter().enumerate() {
                        table_cell(ui, columns[cell].1, &mut |ui| {
                            if table.title == ROADMAP_TITLE && index == 0 && cell == 0 {
                                jump(ui, "s-m1", text, TABLE_TEXT, Size::Grow, picked);
                                table_tag(ui, M1_NOW, SAGE, SAGE, false);
                            } else {
                                doc_label(
                                    ui,
                                    text,
                                    TABLE_TEXT,
                                    if cell == 0 { INK } else { MUTED },
                                );
                            }
                        });
                    }
                });
            }
        });
    });
}

// Cached Splice v13, index.html sections s-m1 through s-sources.
const M1_TITLE: &str = "M1 · Splice 0.1: the headless core";
const ROADMAP_TITLE: &str = "Roadmap";
const OPEN_TITLE: &str = "Open questions";
const RESEARCH_TITLE: &str = "Research";
const SOURCES_TITLE: &str = "Sources";
const M1_STATUS: &str = "hard milestone";
const M1_LEDE: &str = "Build the core and its MCP surface first, with every editing capability in the core, and the thinnest possible UI. Cut 0.1 lands today, 18 Nov 2026; a Codex loop keeps building after the cut until the M1 exit. The board is PLAN.md in the repo; the rules are AGENTS.md.";
const M1_RECORDING: &str = "Secondary to editing, but in 0.1: agents need real recordings to make projects, to edit them, and to feed the self-improvement loop. The hard requirement is that an agent can record from ssh or a terminal without a prompt.";
const M1_SHORTCUTS: &str = "Deliberate shortcuts in 0.1 (each a ponytail: with an upgrade trigger): preview and export share an AVFoundation composition instead of the M3 Metal compositor; titles and captions render through CoreAnimation in export and PXUI text in preview; the viewer copies one RGBA frame per tick into pxui::images.";
const CODEBASES: &str = "../recorder: docs/SPEC.md §2 stack choices, docs/MCP.md, Render/FrameSource.swift, Render/Exporter.swift, Editor/Transcriber.swift, RecorderCore/EditOps.swift, IntentOps.swift, Agent/MCPServer.swift. ../rays: AGENTS.md, Cargo.toml profiles, .cargo/config.toml, tools/gate, tools/buildtime, docs/development.md, crates/flow_eval and crates/lru allocation tests.";
const SURFACES_HEAD: &[&str] = &["Surface", "0.1 behavior"];
const SURFACES: &[&[&str]] = &[
    &[
        "make dev",
        "Debug profile (fast incremental) → bundled, signed Splice.app → ~/.local/bin/splice",
    ],
    &[
        "make install",
        "The same with the release profile. One bundle ID and signer for both, so macOS privacy grants persist across rebuilds",
    ],
    &[
        "splice",
        "Opens the viewer with the native macOS open panel to pick a .splice project",
    ],
    &[
        "splice <path>",
        "Opens the viewer on that project; splice new <path> creates one",
    ],
    &[
        "splice mcp",
        "MCP over stdio. Never opens UI. Editing, export and screen recording all run headless; when a viewer has the project open, calls go to it so the human sees edits live",
    ],
    &[
        "splice apply · snapshot · describe · export · frame · friction · check",
        "Headless CLI over the same core",
    ],
    &[
        "Viewer",
        "A window with the program frame (letterboxed to the project aspect) and a play bar. Space plays and pauses, ←/→ steps, ⌘Z/⇧⌘Z undo and redo. Nothing else",
    ],
    &[
        "splice record <new.splice>",
        "Records a display, window or area (with mic, system audio, camera and input events) into a project; the take lands as an editable clip. The same as MCP record.start/record.stop",
    ],
    &[
        "splice permissions [--request]",
        "Shows or requests the four privacy grants for Splice.app, with the signing identity and install path",
    ],
    &[
        "Export",
        "AVFoundation, no approval, <project>.splice/exports/<name>-r<rev>.mp4, never overwrites unless asked",
    ],
];
const BOARD_HEAD: &[&str] = &["Phase", "What", "Where", "Cut"];
const BOARD: &[&[&str]] = &[
    &[
        "T0 · Foundations",
        "Workspace, toolchain 1.99.1, PXUI submodule, cargo t (done); Makefile, gate, testkit with zero-alloc probes",
        "Linux",
        "0.1",
    ],
    &[
        "T1 · Model and format",
        "Arenas and IDs, canonical JSON, the .splice package, the journal with crash recovery at every byte offset",
        "Linux",
        "0.1",
    ],
    &[
        "T2 · Core",
        "Registry (all 61 rows), apply, Tier A commands, history with last/session/to and a 500-batch property test, eval, budgets; Tier B and planned rows later",
        "Linux",
        "0.1 → 0.3",
    ],
    &[
        "T3 · MCP and CLI",
        "JSON-RPC stdio, 19 tools, friction log, transcript tests, the CLI; evals in 0.2",
        "Linux",
        "0.1",
    ],
    &[
        "T4 · Media",
        "apple_sys in LocalWave's binding pattern, probe, AVComposition lowering shared by preview and export, export job, view.frames",
        "Mac",
        "0.1",
    ],
    &[
        "T5 · Viewer",
        "pxui_host window, open panel, frame view, play bar, live socket, offscreen --snapshot",
        "Mac",
        "0.1",
    ],
    &[
        "T6 · splice_kit",
        "Commands, responder chain, keymap, undo manager; gesture phases in 0.2",
        "Linux",
        "0.1",
    ],
    &[
        "T7 · Install, locate, sign",
        "LocalWave model: Info.plist with the .splice package UTI, bundle script, Developer ID signing, symlink install, bundle check",
        "Mac",
        "0.1 (notarize 0.3)",
    ],
    &[
        "T8 · Recording",
        "Recorder's capture over MCP and splice record: stable identity, a capture agent, permissions once, ScreenCaptureKit bindings, takes as projects",
        "Mac",
        "0.1 (after T7)",
    ],
    &[
        "T9 · Captions",
        "SpeechAnalyzer words with frame timing, caption commands, burn-in",
        "Mac + Linux",
        "0.2",
    ],
];
const MILESTONES_HEAD: &[&str] = &["Milestone", "Scope", "Exit"];
const MILESTONES: &[&[&str]] = &[
    &[
        "M1 · Headless core",
        "Splice 0.1 today: model, journal-first format, core with Tier A commands and full history, 19-tool MCP and CLI, AVFoundation export and frames, a viewer with a play bar, splice_kit commands and undo, make dev/install with a signed bundle. 0.2: the rest of the built registry, captions and transcription, headless recording, evals. 0.3: every registry row, notarization, budgets.",
        "Every registry row is built and reachable over MCP; the friction backlog is empty or triaged",
    ],
    &[
        "K · Kit → PXUI",
        "Move splice_kit (commands, responder chain, undo, gesture phases) into PXUI as an additive step after its 2.0 freeze; then video textures, NSMenu bar, filled icon paths and styled spans, the same way.",
        "Splice and rays use them from PXUI; LocalWave apps inherit them",
    ],
    &[
        "M3 · Playback",
        "VT decode, frame cache, the one Metal compositor for preview and export (replacing the M1 AVFoundation lowering), audio output, thumbnails and peaks",
        "Playback budget met on M1",
    ],
    &[
        "M4 · App",
        "Timeline view, inspector, menus, keymap, palette, agent banner, at parity with this prototype",
        "The parity check passes against the real UI",
    ],
    &[
        "M5 · Export",
        "Metal-compositor export (replacing M1's AVFoundation lowering), encoder presets, jobs",
        "Export budget and pixel parity ≤ 1",
    ],
    &[
        "M6 · Text, captions and intents",
        "SpeechAnalyzer with forced alignment and a provider trait, word anchors, cut.words and cut.pauses, captions (generate, set, split, merge, 20+ presets, safe zones, SRT/VTT, burn-in, translate), the intent.* recipes",
        "Word timing within 1 frame on the benchmark set; planned registry rows built",
    ],
    &[
        "M8 · Motion graphics",
        "Parameter paths and the keyframe engine (param.set, param.keys, curve editor), templates and brand kit, text animators (captions move onto them), compound clips; then masks and point tracking",
        "Every numeric parameter can be keyframed from the UI and MCP; caption presets are templates",
    ],
    &[
        "M7 · Recorder bridge",
        "Import Recorder takes with events; then switch Recorder's MCP to frames, clip IDs and live sessions",
        "One agent workflow spans both apps",
    ],
];
const M1_NOW: &str = "now";
const LAYERS_HEAD: &[&str] = &["Editor", "Model", "Upper layers when the base changes"];
const LAYERS: &[&[&str]] = &[
    &[
        "Final Cut Pro",
        "Magnetic primary storyline; connected clips and storylines; roles",
        "Move and delete with the parent; the grave key overrides; a partial range delete re-attaches to the nearest clip",
    ],
    &[
        "FCPXML",
        "Anchored children with lane + offset in parent time",
        "Follow automatically",
    ],
    &[
        "Premiere Pro",
        "Numbered V/A tracks at absolute times; drag overwrites",
        "Track targeting and sync lock; red out-of-sync indicators",
    ],
    &[
        "DaVinci Resolve",
        "Absolute tracks; Place on Top",
        "Linked selection, auto track select and sync lock",
    ],
    &[
        "CapCut",
        "Main track + overlays, magnet toggle",
        "Reportedly follow with the magnet on (unverified)",
    ],
    &[
        "Descript",
        "Scenes own layers",
        "Clipped at scene boundaries",
    ],
    &[
        "Tella",
        "Screen and camera slots; auto-packing Effect and Overlay rows; Presenter front/behind",
        "Not documented",
    ],
    &[
        "Screen Studio, Cap",
        "Zoom, layout and scene rows",
        "Cap patched keystroke and caption drift at trims (PR #1726)",
    ],
    &[
        "iMovie",
        "Overlay attached to the clip below",
        "Moves with the main clip",
    ],
    &[
        "OpenTimelineIO",
        "Stack of tracks with clips and gaps",
        "Positional only",
    ],
];
const CANDIDATES_HEAD: &[&str] = &[
    "",
    "A. Absolute tracks + sync lock",
    "B. FCP connected clips, positional lanes",
    "C. Anchored lanes (chosen)",
];
const CANDIDATES: &[&[&str]] = &[
    &["Sync", "Manual", "Automatic", "Automatic"],
    &[
        "Agent addressing",
        "Track index + time, stale after any ripple",
        "Anchor, but lane numbers shift",
        "Anchor + stable lane ID",
    ],
    &[
        "Main risk",
        "Silent drift, overwrites",
        "Cascades, shifting lanes",
        "Cascades, mitigated by diff, undo and preview",
    ],
];
const QUESTIONS: &[&str] = &[
    "PXUI timing: will the command, undo and texture additions fit the Unification Plan before the 2.0 freeze? The plan lives in LocalWave's AGENTS.md; agree the step with its owner.",
    "Caption safe zones: measure the TikTok, Reels and Shorts UI overlays per aspect and device (current values are estimates).",
    "Speech provider: is SpeechAnalyzer word timing good enough after forced alignment, or is WhisperKit the default? Benchmark before M6.",
    "Several speakers: one Captions lane per speaker from diarization, or speaker colour as a cue parameter?",
    "Burn-in vs caption tracks: export both (burned pixels plus an embedded caption track)? Leaning yes, per export preset.",
    "Audio lanes: anchor music and voiceover like overlays (pinned by default), or keep a separate audio model? Leaning toward the same item model with kind:\"audio\" and a pinned default.",
    "Inherit transform: which overlays inherit spine zoom by default (callout boxes yes, b-roll no)?",
    "Time-ranged effects: limit an effect to part of a clip with handles inside the clip (Camtasia), or as segments in a lane fx row? Needed for hide-cursor ranges, camera layout changes and blur regions.",
    "Shared background across a crossfade: draw the Window background once when both sides resolve the same one.",
    "Effect presets: one file format for an effect stack and for a whole style (project + lane stacks), importable from Recorder presets.",
    "Capture: keep Recorder as the capture app, or fold capture into Splice after M7?",
    "Variable frame rate sources: conform to the sequence rate at import, or keep a time map per clip as Recorder's TimeMap does?",
    "Lane reorder semantics: does compositing order follow lane order strictly, or can an item override it?",
];
const EVIDENCE: &[&str] = &[
    "Sync drift and overwrites are the top complaints about track editors (Premiere sync lock in ripple delete; a 2026 request for a magnetic mode; Shotcut marker drift).",
    "Anchored models fail differently: b-roll vanishes with its dialogue, and titles move on a slip. The cure is visibility (connectors, side effects, undo notices), not dropping anchoring.",
    "B-Script (CHI 2019, 110 participants): inserting b-roll from a transcript was faster and easier than from a timeline. LAVE (IUI 2024): users want \"the final say\", and LLM precision trims were often wrong.",
    "Agent interfaces: Premiere UXP wraps typed actions in executeTransaction with an undo string. fcp-mcp-server groups 102 operations into 15 verbs with dry run and a journal. Descript chose not to make its MCP 1:1; Splice diverges on purpose but keeps fast perception on unpublished projects.",
];
const CARDS: &[(&str, &[&str])] = &[
    (
        "The build loop",
        &[
            "make loop runs codex exec once per iteration (AGENT=claude for Claude Code). Each iteration does one task from the board or the friction backlog: test first, rays rules, make check, commit, log.",
            "Order: a red check, then friction clusters, then the next task in the current cut. Blocked twice means a written blocked: note, not a weaker test.",
            "An iteration without a commit stops the loop for a human; a STOP file ends it cleanly.",
        ],
    ),
    (
        "Machines",
        &[
            "Linux box: model, format, core, MCP, CLI, kit, gate and evals. Mac crates are cfg-gated so the workspace stays green here.",
            "Mac mini over ssh (macOS 27, arm64, Developer ID): media, viewer, bundle, signing, install. make mac-sync · mac-check · mac-run · mac-pull.",
            "The Mini is shared with another Rust project: Mac tasks only, one sync and check per task, nice and two build jobs, no polling, offscreen snapshots instead of the screen.",
        ],
    ),
    (
        "Stable identity and location",
        &[
            "One location: ~/Applications/Splice.app, symlinked from ~/.local/bin/splice. make dev and make install replace that same bundle in place. Capture never runs from target/.",
            "One signer: Developer ID (team 7ZH3FYV964) with the hardened runtime, for dev and release. No ad-hoc fallback. The designated requirement is pinned in the repo, and the bundle check refuses a mismatch, since grants are keyed on bundle ID + requirement.",
            "A signer change resets only Splice's own grants, and says so (Recorder's rule).",
        ],
    ),
    (
        "Capture agent and permissions",
        &[
            "macOS charges capture to the responsible process. So capture runs in Splice.app, started by LaunchServices as an accessory process (no Dock icon, no window), reached over the same-user socket. splice mcp and splice record are its clients, and grants stay with Splice.app, not the terminal or the agent.",
            "splice permissions --request asks once for Screen Recording, Microphone, Camera and Input Monitoring; a person clicks once on the Mini. After that, records start silently.",
            "A missing grant is TCC_DENIED with the exact fix; needsHuman:true when only a person can help (macOS can periodically re-confirm screen access).",
        ],
    ),
];
const SOURCE_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Lane models",
        &[
            (
                "FCP connect clips",
                "https://support.apple.com/guide/final-cut-pro/ver7a77ef9e/mac",
            ),
            (
                "FCP remove clips",
                "https://support.apple.com/guide/final-cut-pro/verc5d34129/mac",
            ),
            (
                "FCPXML timing",
                "https://developer.apple.com/documentation/professional-video-applications/timing-attributes.md",
            ),
            (
                "Gollner: bet against tracks",
                "https://blog.alex4d.com/2012/07/24/fcpx-apples-bet-against-tracks/",
            ),
            (
                "Premiere sync lock",
                "https://community.adobe.com/t5/premiere-pro-discussions/sync-lock-not-respected-during-ripple-delete/m-p/15305272",
            ),
            (
                "Tella multi-track",
                "https://www.tella.com/help/editing/multi-track-timeline.md",
            ),
            (
                "Descript scenes",
                "https://help.descript.com/getting-started/scenes.md",
            ),
            (
                "OTIO structure",
                "https://opentimelineio.readthedocs.io/en/latest/tutorials/otio-timeline-structure.html",
            ),
        ],
    ),
    (
        "Research",
        &[
            ("B-Script (CHI 2019)", "https://arxiv.org/abs/1902.11216"),
            ("LAVE (IUI 2024)", "https://arxiv.org/html/2402.10294v1"),
            (
                "ChunkyEdit (CHI 2024)",
                "https://research.adobe.com/publication/chunkyedit-text-first-video-interview-editing-via-chunking",
            ),
        ],
    ),
    (
        "Agent interfaces and MCP",
        &[
            (
                "Premiere UXP SequenceEditor",
                "https://developer.adobe.com/premiere-pro/uxp/ppro_reference/classes/sequenceeditor/",
            ),
            (
                "fcp-mcp-server",
                "https://github.com/DareDev256/fcp-mcp-server",
            ),
            (
                "davinci-resolve-mcp",
                "https://github.com/samuelgursky/davinci-resolve-mcp",
            ),
            (
                "Descript: don't ship your API as an MCP",
                "https://www.descript.com/blog/article/dont-ship-your-api-as-an-mcp",
            ),
            (
                "Remotion MCP shutdown",
                "https://www.remotion.dev/docs/ai/mcp",
            ),
            (
                "MCP tools 2025-11-25",
                "https://modelcontextprotocol.io/specification/2025-11-25/server/tools",
            ),
            (
                "MCP 2026-07-28 changelog",
                "https://modelcontextprotocol.io/specification/2026-07-28/changelog",
            ),
            (
                "Writing tools for agents",
                "https://www.anthropic.com/engineering/writing-tools-for-agents",
            ),
            (
                "Advanced tool use",
                "https://www.anthropic.com/engineering/advanced-tool-use",
            ),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Event, FrameIn, Handle, testing};
    use std::cell::Cell;

    #[test]
    fn plan_sections_measure_at_reference_widths_without_warm_allocations() {
        mem_sys::set_peer(pxui::library_counter);
        for (width, article_width) in [(960.0, 676.0), (800.0, 768.0), (480.0, 448.0)] {
            let narrow = width <= 820.0;
            for (anchor, title, tables) in [
                ("s-m1", M1_TITLE, 2),
                ("s-roadmap", ROADMAP_TITLE, 1),
                ("s-open", OPEN_TITLE, 0),
                ("s-research", RESEARCH_TITLE, 2),
                ("s-sources", SOURCES_TITLE, 0),
            ] {
                let mut ui = crate::viewer_ui().unwrap();
                ui.set_a11y(true);
                let frame = FrameIn::new(width, 640.0);
                let section = Cell::new(Handle::default());
                let build = &mut |ui: &mut Ui| {
                    let node = ui.node(
                        "plan-test",
                        Node::column().w_px(article_width).pad_y(20.0).gap(8.0),
                    );
                    section.set(node);
                    ui.within(&node, &mut |ui| {
                        assert!(document(ui, anchor, narrow, &mut None, &mut None));
                    });
                };
                for _ in 0..3 {
                    ui.frame(&frame, build).unwrap();
                }
                let area = ui.rect(&section.get());
                assert_eq!(area.w, article_width);
                assert!(area.h > 150.0);
                assert!(testing::find(&ui, Role::Heading, title).is_some());
                assert_eq!(
                    ui.a11y_nodes()
                        .iter()
                        .filter(|node| node.is_role(Role::Table))
                        .count(),
                    tables
                );
                eprintln!(
                    "{width} {anchor}: section {:.2} × {:.2}, tables {:?}",
                    area.w,
                    area.h,
                    ui.a11y_nodes()
                        .iter()
                        .filter(|node| node.is_role(Role::Table))
                        .map(|node| (node.rect.w, node.rect.h))
                        .collect::<Vec<_>>()
                );
                splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                    .warm(&mut || {
                        ui.frame(&frame, build).unwrap();
                    });
            }
        }
    }

    #[test]
    fn plan_sections_stay_warm_in_the_full_spec() {
        mem_sys::set_peer(pxui::library_counter);
        for width in [960.0, 800.0, 480.0] {
            let mut ui = crate::viewer_ui().unwrap();
            let frame = FrameIn::new(width, 640.0);
            let build = &mut |ui: &mut Ui| {
                let _ = super::super::spec(ui, super::super::Selection::default(), &[]);
            };
            for _ in 0..4 {
                ui.frame(&frame, build).unwrap();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    ui.frame(&frame, build).unwrap();
                });
        }
    }

    fn click(ui: &mut Ui, frame: &FrameIn<'_>, draw: &mut dyn FnMut(&mut Ui), label: &str) {
        let target = testing::find(ui, Role::Link, label).unwrap();
        let x = f64::from(target.x + target.w * 0.5);
        let y = f64::from(target.y + target.h * 0.5);
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            ui.frame(&frame.events(&[Event::pointer(kind, 0, x, y)]), draw)
                .unwrap();
        }
    }

    #[test]
    fn plan_links_emit_internal_and_external_targets_from_real_clicks() {
        assert_eq!(
            SOURCE_GROUPS
                .iter()
                .map(|(_, links)| links.len())
                .sum::<usize>(),
            20
        );
        for anchor in ["s-roadmap", "s-sources"] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let section = Cell::new(None);
            let external = Cell::new(None);
            let frame = FrameIn::new(800.0, 640.0);
            let build = &mut |ui: &mut Ui| {
                let mut picked = None;
                let mut url = None;
                ui.column(Node::column().w_px(768.0).gap(8.0), |ui| {
                    assert!(document(ui, anchor, true, &mut picked, &mut url));
                });
                section.set(picked.or(section.get()));
                external.set(url.or(external.get()));
            };
            for _ in 0..3 {
                ui.frame(&frame, build).unwrap();
            }
            assert_eq!((section.get(), external.get()), (None, None));
            if anchor == "s-roadmap" {
                for (label, target) in [
                    ("M1 \u{b7} Headless core", "s-m1"),
                    ("Current command registry", "s-registry"),
                ] {
                    click(&mut ui, &frame, build, label);
                    assert_eq!(section.take().map(|index| SECTIONS[index].0), Some(target));
                    assert_eq!(external.take(), None);
                }
            } else {
                for &(label, url) in SOURCE_GROUPS.iter().flat_map(|(_, links)| *links) {
                    click(&mut ui, &frame, build, label);
                    assert_eq!(external.take(), Some(url), "{label}");
                    assert_eq!(section.take(), None);
                }
            }
        }
    }
}
