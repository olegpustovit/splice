use super::{
    CAPTION, DOC_BODY, DOC_LEDE, INK, MOTION, MUTED, ModelPoint, QUIET, SAGE, STATUS_LINK,
    SUBHEADING, TABLE_EDGE, TABLE_HEAD, TABLE_LINE, TABLE_ROW, TABLE_TEXT, doc_label,
    model_subsection, table_cell, table_row, table_tag,
};
use pxui::{
    A11yStates, Button, Label, Node, Role, TextSpan, Ui,
    draw::flags::rect::BOTTOM,
    geom::{Edges, Rgba},
    ui::{Align, Look, Size, State, States},
};
use splice_core::commands::effect_schemas;
use splice_json::Value as Json;

const BODY_WIDTH: f32 = 561.6;
const BULLET_GUTTER: f32 = 20.0;
const SECTION: Node = Node::column().w(Size::Grow).gap(20.0);
const FIRST_SUBSECTION: Edges = Edges::new(0.0, 12.0, 0.0, 6.0);

pub(super) fn timeline(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(SECTION.padding(FIRST_SUBSECTION), |ui| {
        model_subsection(ui, "Structure", width, STRUCTURE, picked);
        model_subsection(ui, "Creating and moving", width, CREATING, picked);
        subsection_table(ui, "Parent edits", "Parent edits", width, &mut |ui| {
            let spine = column_width(width, 130.44, 0.2337);
            let child = width - spine;
            let columns = [("Spine edit", spine), ("Anchored children", child)];
            document_table(ui, "Parent edits", &columns, &mut |ui| {
                for &(edit, outcome) in PARENT_EDITS {
                    table_row(ui, &mut |ui| {
                        cell(ui, spine, (edit, &[]), INK);
                        cell(ui, child, outcome, MUTED);
                    });
                }
            });
        });
        model_subsection(ui, "Compositing and audio", width, COMPOSITING, picked);
        model_subsection(
            ui,
            "Several video clips and crossfades",
            width,
            CROSSFADES,
            picked,
        );
    });
}

pub(super) fn effects(ui: &mut Ui, width: f32, picked: &mut Option<usize>) {
    ui.column(SECTION.padding(Edges::new(0.0, 0.0, 0.0, 6.0)), |ui| {
        ui.column(Node::column().w(Size::Grow).max_w(624.0), |ui| {
            doc_label(ui, "A screen take brings in its recorded components. Window, Cursor, Camera Layout and Keystroke Chips are additive effects on a clip, a lane or the project. Those scopes make a setting visible, overridable or removable where it applies. The v13 Click Zoom effect is retired. Native zooms are timeline items that can be edited independently.", DOC_LEDE.role(Role::Paragraph), INK);
        });
        subsection_table(ui, "Four kinds of layer", "Four kinds of layer", width, &mut |ui| {
            layer_table(ui, width);
        });
        model_subsection(ui, "Scopes and resolution", width, SCOPES, picked);
        render_order(ui, width);
        model_subsection(ui, "Window on a clip vs a lane", width, WINDOWS, picked);
        model_subsection(ui, "UI", width, EFFECT_UI, picked);
        subsection_table(ui, "Every Recorder feature as a layer", "Recorder features", width, &mut |ui| {
            recorder_table(ui, width);
        });
        ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
            model_subsection(ui, "Agents and the Rust core", width, AGENTS, picked);
            native_registry(ui, width);
        });
        model_subsection(ui, "Research behind this", width, RESEARCH, picked);
    });
}

fn subsection_table(
    ui: &mut Ui,
    title: &str,
    key: &'static str,
    width: f32,
    draw: &mut dyn FnMut(&mut Ui),
) {
    ui.column(Node::column().w(Size::Grow).gap(4.0), |ui| {
        doc_label(ui, title, SUBHEADING, INK);
        ui.view_of(key, &width.to_bits(), &mut *draw);
    });
}

// Below the smallest measured page, scale the intrinsic widths together so
// each column still has room. Above it the CSS tables distribute extra width.
fn column_width(width: f32, compact: f32, share: f32) -> f32 {
    compact * (width / 448.0).min(1.0) + (width - 448.0).max(0.0) * share
}

fn document_table(ui: &mut Ui, name: &str, columns: &[(&str, f32)], draw: &mut dyn FnMut(&mut Ui)) {
    ui.column(Node::column().w(Size::Grow).pad_y(8.0), |ui| {
        let table = ui.node(
            "model-table",
            Node::column().w(Size::Grow).role(Role::Table),
        );
        ui.a11y_name(&table, name);
        ui.within(&table, &mut |ui| {
            ui.row(
                TABLE_ROW
                    .padding(Edges::new(0.0, 6.5, 0.0, 6.0))
                    .look(Look::NONE.border(TABLE_EDGE, 1.0).sides(BOTTOM)),
                |ui| {
                    for &(title, width) in columns {
                        table_cell(ui, Size::Px(width), &mut |ui| {
                            let label = ui.label(title, TABLE_HEAD);
                            ui.a11y_name(&label, title);
                            ui.style(
                                &label,
                                &States::of(Look::NONE.text(QUIET)),
                                State::default(),
                            );
                        });
                    }
                },
            );
            draw(ui);
        });
    });
}

type Styled = (&'static str, &'static [TextSpan]);

fn cell(ui: &mut Ui, width: f32, content: Styled, color: Rgba) {
    table_cell(ui, Size::Px(width), &mut |ui| {
        styled(
            ui,
            (width - 16.0).max(1.0),
            content,
            TABLE_TEXT,
            TABLE_LINE,
            Role::Cell,
            color,
        );
    });
}

fn styled(
    ui: &mut Ui,
    width: f32,
    (text, spans): Styled,
    label: Label,
    line: f32,
    role: Role,
    color: Rgba,
) {
    if spans.is_empty() {
        doc_label(ui, text, label, color);
        return;
    }
    let lines = ui
        .text_service()
        .measure_spans(text, label, Some(width), spans)
        .expect("valid model table or list spans")
        .lines;
    let text_node = ui.node(
        "model-text",
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(lines as f32 * line)
            .role(role)
            .look(Look::NONE.text(color)),
    );
    ui.text_spans(&text_node, text, label, spans)
        .expect("valid model table or list spans");
}

fn render_order(ui: &mut Ui, width: f32) {
    let width = (width - BULLET_GUTTER).clamp(1.0, BODY_WIDTH);
    ui.column(Node::column().w(Size::Grow).gap(6.0), |ui| {
        doc_label(ui, "Render order per spine clip", SUBHEADING, INK);
        ui.column(Node::column().w(Size::Grow).role(Role::List), |ui| {
            for (number, point) in ["1.", "2.", "3.", "4.", "5.", "6."]
                .into_iter()
                .zip(RENDER_ORDER)
            {
                ui.row(
                    Node::row()
                        .w(Size::Grow)
                        .align(Align::Start)
                        .role(Role::ListItem),
                    |ui| {
                        doc_label(
                            ui,
                            number,
                            DOC_BODY.w_px(BULLET_GUTTER).role(Role::Unknown),
                            MUTED,
                        );
                        ui.column(Node::column().w_px(width), |ui| {
                            styled(
                                ui,
                                width,
                                (point.text, point.spans),
                                DOC_BODY.role(Role::Paragraph),
                                19.44,
                                Role::Paragraph,
                                MUTED,
                            );
                        });
                    },
                );
            }
        });
    });
}

fn layer_table(ui: &mut Ui, width: f32) {
    let kind = column_width(width, 79.16, 0.0338);
    let examples = column_width(width, 106.41, 0.2786);
    let surface = column_width(width, 118.41, 0.1893);
    let detail = width - kind - examples - surface;
    let columns = [
        ("Kind", kind),
        ("What it is", detail),
        ("Examples", examples),
        ("Where you edit it", surface),
    ];
    document_table(ui, "Four kinds of layer", &columns, &mut |ui| {
        for &[kind_text, detail_text, examples_text, surface_text] in LAYER_ROWS {
            table_row(ui, &mut |ui| {
                cell(ui, kind, kind_text, INK);
                cell(ui, detail, detail_text, MUTED);
                cell(ui, examples, examples_text, MUTED);
                cell(ui, surface, surface_text, MUTED);
            });
        }
    });
}

fn recorder_table(ui: &mut Ui, width: f32) {
    let feature = column_width(width, 151.50, 0.4482);
    let status = column_width(width, 132.86, 0.0920);
    let layer = width - feature - status;
    let columns = [
        ("Recorder feature", feature),
        ("Splice layer", layer),
        ("Status", status),
    ];
    document_table(ui, "Recorder features", &columns, &mut |ui| {
        for &(feature_text, layer_text, states) in RECORDER_ROWS {
            table_row(ui, &mut |ui| {
                cell(ui, feature, feature_text, INK);
                cell(ui, layer, layer_text, MUTED);
                table_cell(ui, Size::Px(status), &mut |ui| {
                    ui.row(
                        Node::row().w(Size::Grow).gap(4.0).wrap().role(Role::Cell),
                        |ui| {
                            for &(state, built) in states {
                                let (color, edge) = if built {
                                    (SAGE, SAGE)
                                } else {
                                    (MUTED, TABLE_EDGE)
                                };
                                table_tag(ui, state, color, edge, !built);
                            }
                        },
                    );
                });
            });
        }
    });
}

fn native_registry(ui: &mut Ui, width: f32) {
    let disclosure = ui.button(
        "native-effect-registry",
        "Native effect types and scopes",
        &Button::DEFAULT
            .size(Size::Fit, Size::Px(19.44))
            .label(CAPTION.w(Size::Fit))
            .look(&STATUS_LINK),
    );
    let handle = disclosure.handle;
    let mut open = ui.state(&handle, 0) != 0;
    if disclosure.clicked {
        open = !open;
        ui.set_state(&handle, i64::from(open));
    }
    let states = if open {
        A11yStates::EXPANDABLE.with(A11yStates::EXPANDED)
    } else {
        A11yStates::EXPANDABLE
    };
    ui.set_role(&handle, Role::Button, states);
    if open {
        ui.view_of("native-effect-catalog", &width.to_bits(), |ui| {
            let title_width = width * 0.32;
            let scope_width = width - title_width;
            let columns = [("Effect", title_width), ("Applies to", scope_width)];
            document_table(ui, "Native effect registry", &columns, &mut |ui| {
                for (id, schema) in effect_schemas().as_object().unwrap_or_default() {
                    let title = schema.field("title").and_then(Json::as_str).unwrap_or(id);
                    let scope = schema
                        .field("appliesTo")
                        .and_then(Json::as_str)
                        .unwrap_or_default();
                    table_row(ui, &mut |ui| {
                        table_cell(ui, Size::Px(title_width), &mut |ui| {
                            doc_label(ui, title, TABLE_TEXT, INK)
                        });
                        table_cell(ui, Size::Px(scope_width), &mut |ui| {
                            doc_label(ui, scope, TABLE_TEXT, MUTED)
                        });
                    });
                }
            });
        });
    }
}
const STRUCTURE: &[ModelPoint] = &[
    ModelPoint {
        text: "Lane order. The project orders Captions, Graphics (titles), Overlays (b-roll, picture-in-picture, images), Motion, Camera and Audio above the magnetic Video spine. The expandable take rows sit below it.",
        spans: &[
            TextSpan::new(0, 11).weight(500).color(INK),
            TextSpan::new(151, 156).weight(500).color(INK),
        ],
        link: None,
    },
    ModelPoint {
        text: "Each secondary lane has a stable ID and an editable name. Lane order can change; empty lanes persist until you remove them.",
        spans: &[],
        link: None,
    },
    ModelPoint {
        text: "Each overlay has an anchor such as {clip, sourceFrame} or {pinned:true, frame}, a lane, a length in frames and, for media, a source in-point. A clip-anchored item may extend past its parent. The start can also be addressed by word or marker.",
        spans: &[
            TextSpan::new(20, 26).color(MOTION),
            TextSpan::new(35, 54).color(MOTION),
            TextSpan::new(58, 78).color(MOTION),
            TextSpan::new(82, 86).color(MOTION),
            TextSpan::new(90, 96).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Start is derived. At constant speed, start = parent.start + (sourceFrame − parent.source) / parent.speed. Ramps use the source time map. The anchor makes the item follow ripples, reorders and speed changes.",
        spans: &[
            TextSpan::new(0, 17).weight(500).color(INK),
            TextSpan::new(37, 106).color(MOTION),
        ],
        link: None,
    },
];

const CREATING: &[ModelPoint] = &[
    ModelPoint {
        text: "Add as Overlay places media above the spine and creates a lane when there is no compatible one free.",
        spans: &[TextSpan::new(0, 14).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "With an implicit lane, overlaps bump to the nearest free lane of that kind. Untouched items keep their lanes. An occupied explicit lane returns LANE_CONFLICT.",
        spans: &[TextSpan::new(144, 157).color(MOTION)],
        link: None,
    },
    ModelPoint {
        text: "Horizontal drag re-anchors to the clip beneath the new start. A guide connects the item to its anchor.",
        spans: &[TextSpan::new(0, 15).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Vertical drag, or ⌥↑ / ⌥↓, moves an item between compatible lanes.",
        spans: &[TextSpan::new(0, 13).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Hold the grave key when moving a spine clip to leave its overlays in place. They re-anchor beneath that frame (carryOverlays:false).",
        spans: &[
            TextSpan::new(0, 18).weight(500).color(INK),
            TextSpan::new(111, 130).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Lift to Overlay (⌥⌘↑) moves a spine clip to a lane and closes the gap. Replace Spine Clip (⌥⌘↓) puts a media overlay on the spine and re-anchors the old clip’s children.",
        spans: &[
            TextSpan::new(0, 15).weight(500).color(INK),
            TextSpan::new(77, 95).weight(500).color(INK),
        ],
        link: None,
    },
];

const COMPOSITING: &[ModelPoint] = &[
    ModelPoint {
        text: "Layer order. From the bottom, take picture and cursor › behind-presenter overlays › camera and key chips › other overlays and graphics. Captions usually sit above titles.",
        spans: &[TextSpan::new(0, 12).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Behind Presenter places an overlay under the take’s camera, as in the v13 Presenter row.",
        spans: &[TextSpan::new(0, 16).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Motion and camera zooms affect the spine. V13 proposed an optional way for overlays to inherit the transform.",
        spans: &[TextSpan::new(0, 23).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Overlay audio follows its anchor. B-roll is silent by default. Audio mixes across lanes.",
        spans: &[],
        link: None,
    },
    ModelPoint {
        text: "Take components (screen, camera, mic, system audio, cursor and keystrokes) share a source clock and move with the take.",
        spans: &[],
        link: None,
    },
];

const CROSSFADES: &[ModelPoint] = &[
    ModelPoint {
        text: "The spine shows one clip at a time away from transitions. Cut between takes on the spine, or use a clip fade to fade its picture.",
        spans: &[TextSpan::new(0, 34).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Two pictures at once live on an overlay lane for b-roll, picture-in-picture or a second take.",
        spans: &[TextSpan::new(0, 44).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "A crossfade is an overlap. Lift the incoming clip to an overlay, overlap it with the outgoing picture and fade its opacity in. Native transition.add also draws a dissolve at a spine edit when given the incoming clip, kind:\"dissolve\" and a frame length.",
        spans: &[
            TextSpan::new(0, 26).weight(500).color(INK),
            TextSpan::new(134, 148).color(MOTION),
            TextSpan::new(217, 232).color(MOTION),
        ],
        link: Some(("s-registry", "Command registry")),
    },
];

const SCOPES: &[ModelPoint] = &[
    ModelPoint {
        text: "Three scopes, one model. Effect instances can live on the project, a lane (including lan_spine) or an item. Project Window frames spine clips. Media overlays get Window from their own lane or item; other effect types declare which owners and defaults they accept.",
        spans: &[
            TextSpan::new(0, 24).weight(500).color(INK),
            TextSpan::new(85, 94).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Per-parameter override. For effects that inherit, resolution runs project › lane › item. For example, a clip that sets only Window padding still follows the project’s corner radius. Changing the project corners reaches that clip and leaves its pad override in place.",
        spans: &[TextSpan::new(0, 23).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Turning off is an override too. enabled:false switches off an inherited effect in that scope. The opener and end card in the sample turn off project Window so their pictures can fill the frame.",
        spans: &[
            TextSpan::new(0, 31).weight(500).color(INK),
            TextSpan::new(32, 45).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Effects without a scope default. Window is additive. With no styling override, recorded cursors, camera and keystrokes use their native component defaults. Click-driven zoom is a camera item in the current model.",
        spans: &[TextSpan::new(0, 32).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Instance count is part of the schema. Most types are unique per scope. Native mask instances can stack on one clip or overlay. Each region has its own start and end range and can blur, pixelate, cover or spotlight.",
        spans: &[
            TextSpan::new(0, 37).weight(500).color(INK),
            TextSpan::new(78, 82).color(MOTION),
        ],
        link: None,
    },
];

const RENDER_ORDER: &[ModelPoint] = &[
    ModelPoint {
        text: "Decode the take picture. The Cursor treatment follows the screen’s source coordinates, so cropping and camera zoom affect it with the picture.",
        spans: &[TextSpan::new(29, 35).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Apply the built-in crop and fit or fill inside the space the Window leaves.",
        spans: &[
            TextSpan::new(19, 23).weight(500).color(INK),
            TextSpan::new(28, 39).weight(500).color(INK),
        ],
        link: None,
    },
    ModelPoint {
        text: "Draw the Window backdrop and frame the picture with padding, corners, inset and shadow.",
        spans: &[TextSpan::new(9, 15).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Apply Motion and Camera. In content mode the window stays put. In frame mode the window and backdrop move together.",
        spans: &[
            TextSpan::new(6, 23).weight(500).color(INK),
            TextSpan::new(28, 35).color(MOTION),
            TextSpan::new(66, 71).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Draw behind-presenter overlays, then the take’s Camera Layout and Keystroke Chips. Those layers have their own placement.",
        spans: &[
            TextSpan::new(50, 63).weight(500).color(INK),
            TextSpan::new(68, 83).weight(500).color(INK),
        ],
        link: None,
    },
    ModelPoint {
        text: "Draw overlay lanes with their own resolved effects, then graphics and captions according to lane order. Spine fades and transitions apply to the relevant clip groups.",
        spans: &[],
        link: None,
    },
];

const WINDOWS: &[ModelPoint] = &[
    ModelPoint {
        text: "On a clip, Window frames that clip’s pixels and fills the canvas behind them.",
        spans: &[TextSpan::new(0, 9).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "On a lane, each item draws the default Window on its own. Items can override an inherited parameter. For rounded, shadowed b-roll cards, use background:\"none\" and zero padding on the media overlay lane.",
        spans: &[
            TextSpan::new(0, 9).weight(500).color(INK),
            TextSpan::new(141, 158).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Across a crossfade, the lift-and-overlap recipe uses each clip’s own backdrop. With a native spine dissolve and matching backgrounds, the shared backdrop stays in place as the pictures blend. Padding and corners can interpolate through the transition.",
        spans: &[TextSpan::new(0, 18).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Composite framing around several items together is a separate idea from the v13 proposal. A lane Window applies to individual items.",
        spans: &[TextSpan::new(0, 17).weight(500).color(INK)],
        link: None,
    },
];

const EFFECT_UI: &[ModelPoint] = &[
    ModelPoint {
        text: "Effects browser in the Program toolbar lists live effects. Use Selection for the active clip or overlay and Lane for its lane. Project applies the effect at the top scope where its schema permits. The v13 browser also highlighted eligible targets while dragging an effect onto them.",
        spans: &[TextSpan::new(0, 15).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Inspector cards show where values come from. Editing an inherited field creates an override here and the parameter dot resets it. The menu offers Reset to inherited, Override here, Turn off here, Use for whole lane and Use for project, according to scope.",
        spans: &[TextSpan::new(0, 15).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Lanes are selectable. Click a lane name or the Video label to edit that lane’s effects. The Video label selects lan_spine. With no item or lane selected, Canvas shows the project effects.",
        spans: &[
            TextSpan::new(0, 21).weight(500).color(INK),
            TextSpan::new(114, 123).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Badges distinguish effect scope by shape. Solid fx means a local effect, fx• marks an override, and dashed fx means inherited only. The timeline badge lists scopes, and lanes with effects show fx on their headers.",
        spans: &[
            TextSpan::new(0, 41).weight(500).color(INK),
            TextSpan::new(48, 50).color(MOTION),
            TextSpan::new(73, 78).color(MOTION),
            TextSpan::new(109, 111).color(MOTION),
            TextSpan::new(195, 197).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "Paste Effects (⌘C then ⌥⌘V) merges copied effects into the target’s local settings. The effect.paste command can also request mode:\"replace\".",
        spans: &[
            TextSpan::new(0, 13).weight(500).color(INK),
            TextSpan::new(96, 108).color(MOTION),
            TextSpan::new(134, 148).color(MOTION),
        ],
        link: None,
    },
];

const AGENTS: &[ModelPoint] = &[
    ModelPoint {
        text: "Commands include effect.add, effect.set with reset, effect.remove, effect.paste and effect.promote. cursor.set and keys.set are clip shorthands.",
        spans: &[
            TextSpan::new(17, 27).color(MOTION),
            TextSpan::new(29, 39).color(MOTION),
            TextSpan::new(45, 50).color(MOTION),
            TextSpan::new(52, 65).color(MOTION),
            TextSpan::new(67, 79).color(MOTION),
            TextSpan::new(84, 98).color(MOTION),
            TextSpan::new(100, 110).color(MOTION),
            TextSpan::new(115, 123).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "items.inspect returns effectsResolved, including source scopes and implicit defaults. project.info gives the project and lane effects. commands.describe {id:\"effect.add\"} returns the live type catalogue and parameter ranges.",
        spans: &[
            TextSpan::new(0, 13).color(MOTION),
            TextSpan::new(22, 37).color(MOTION),
            TextSpan::new(86, 98).color(MOTION),
            TextSpan::new(135, 170).color(MOTION),
        ],
        link: None,
    },
    ModelPoint {
        text: "In native Rust the validator and editor share the effect schemas. splice_eval prepares resolved state by project revision so warmed frame evaluation doesn’t allocate.",
        spans: &[TextSpan::new(66, 77).color(MOTION)],
        link: Some(("s-registry", "Command registry")),
    },
];

const RESEARCH: &[ModelPoint] = &[
    ModelPoint {
        text: "Premiere. The v13 design cited Adobe’s track-effects request to apply a treatment to every clip of one track. It compared that with adjustment layers and master clip effects.",
        spans: &[TextSpan::new(0, 9).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Final Cut Pro. V13 compared the inspector’s ordered effects and checkboxes, Paste Attributes for choosing effects, and reusable effect presets. V13 cited the Final Cut Pro User Guide.",
        spans: &[TextSpan::new(0, 14).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "DaVinci Resolve. The Color page’s clip › group › timeline hierarchy was the reference for ordering the project, lane and item scopes. V13 cited the Resolve manual.",
        spans: &[TextSpan::new(0, 16).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Camtasia. The v13 design borrowed the idea of highlighted drop targets and an effect tray from Device Frame, Corner Rounding, Drop Shadow, Keystroke and Cursor effects. V13 cited TechSmith.",
        spans: &[TextSpan::new(0, 9).weight(500).color(INK)],
        link: None,
    },
    ModelPoint {
        text: "Screen Studio, Cap and Tella. Window controls drew on Screen Studio and Cap backgrounds, corners and shadows. Cap applied padding after crop, and Tella let editors apply a background to one clip only.",
        spans: &[TextSpan::new(0, 29).weight(500).color(INK)],
        link: Some(("s-sources", "Sources")),
    },
];

const PARENT_EDITS: &[(&str, Styled)] = &[
    (
        "Move, reorder, ripple, speed change",
        (
            "Follow. Speed scales the offset, because the anchor is in source time.",
            &[],
        ),
    ),
    (
        "Split",
        (
            "Each child goes with the half that contains its anchor frame (reported in sideEffects as re-anchored).",
            &[TextSpan::new(74, 85).color(MOTION)],
        ),
    ),
    (
        "Trim, or a range delete that removes the anchor frame",
        (
            "Re-anchor at the parent’s new edge when a trim removes the anchor frame. The child remains in the timeline.",
            &[],
        ),
    ),
    (
        "Delete the whole clip",
        (
            "The children are deleted and reported in sideEffects. Undo restores them. Delete, Keep Overlays (overlays:\"keep\") creates a gap to carry them.",
            &[
                TextSpan::new(41, 52).color(MOTION),
                TextSpan::new(74, 95).weight(500).color(INK),
                TextSpan::new(97, 112).color(MOTION),
            ],
        ),
    ),
    (
        "Pinned items",
        ("Keep their frame and show a PIN badge.", &[]),
    ),
];

const LAYER_ROWS: &[[Styled; 4]] = &[
    [
        ("Component", &[]),
        (
            "What a take recorded, on one source clock. Components can be shown or hidden.",
            &[],
        ),
        (
            "Screen, camera, mic, system audio, cursor path and clicks, keystroke events",
            &[],
        ),
        (
            "Component lanes under a take (eye), component.set, keys.hide",
            &[
                TextSpan::new(36, 49).color(MOTION),
                TextSpan::new(51, 60).color(MOTION),
            ],
        ),
    ],
    [
        ("Effect", &[]),
        (
            "A treatment attached to a project, lane, or item. Inheritable values become defaults for eligible items at narrower scopes.",
            &[],
        ),
        ("Window, Cursor, Camera Layout, Keystroke Chips, Color", &[]),
        (
            "Effects browser, inspector cards, effect.*",
            &[TextSpan::new(34, 42).color(MOTION)],
        ),
    ],
    [
        ("Time-ranged item", &[]),
        (
            "A timeline item with a start and duration. Its start is anchored the same way as an overlay.",
            &[],
        ),
        ("Camera zooms, motion scale, titles, b-roll, captions", &[]),
        (
            "Timeline lanes, motion.add, zoom.add",
            &[
                TextSpan::new(16, 26).color(MOTION),
                TextSpan::new(28, 36).color(MOTION),
            ],
        ),
    ],
    [
        ("Built-in property", &[]),
        (
            "Intrinsic clip timing or appearance, independent of effect stack order.",
            &[],
        ),
        ("Crop and fit, speed, fades, gain", &[]),
        (
            "Inspector, clip.crop, clip.speed, item.set, audio.set",
            &[
                TextSpan::new(11, 20).color(MOTION),
                TextSpan::new(22, 32).color(MOTION),
                TextSpan::new(34, 42).color(MOTION),
                TextSpan::new(44, 53).color(MOTION),
            ],
        ),
    ],
];

const RECORDER_ROWS: &[(Styled, Styled, &[(&str, bool)])] = &[
    (
        ("Background: wallpaper, gradient, colour, image, blur", &[]),
        (
            "Window backgrounds use wallpaper, gradients, solid colour or an image. Apply blur to the image.",
            &[],
        ),
        &[("Window", true)],
    ),
    (
        ("Padding, rounded corners, inset, shadow", &[]),
        ("Window effect.", &[]),
        &[("Window", true)],
    ),
    (
        (
            "Output aspect (Auto, 16:9, 9:16, 1:1, 4:3, 16:10, 4:5)",
            &[],
        ),
        (
            "Canvas shape via project.set (Auto or preset).",
            &[TextSpan::new(17, 28).color(MOTION)],
        ),
        &[("Auto + 7 shapes", true)],
    ),
    (
        (
            "Crop; time-varying reframe (cropKeys)",
            &[TextSpan::new(28, 36).color(MOTION)],
        ),
        (
            "clip.crop and crop.keys.",
            &[
                TextSpan::new(0, 9).color(MOTION),
                TextSpan::new(14, 23).color(MOTION),
            ],
        ),
        &[("crop", true), ("keys", true)],
    ),
    (
        ("Clip track: split, trim, remove, speed, undo", &[]),
        ("Spine commands and history.", &[]),
        &[("Spine edits", true)],
    ),
    (
        ("Auto-zoom from clicks", &[]),
        (
            "Camera items from intent.autoZoom using take activity.",
            &[TextSpan::new(18, 33).color(MOTION)],
        ),
        &[("Camera items", true)],
    ),
    (
        ("Manual zooms; instant zoom; disable one zoom", &[]),
        (
            "Camera lane. zoom.set / zoom.remove.",
            &[
                TextSpan::new(13, 21).color(MOTION),
                TextSpan::new(24, 35).color(MOTION),
            ],
        ),
        &[("items", true), ("zoom.set", true)],
    ),
    (
        ("Cursor: size, click rings, hide", &[]),
        ("Cursor effect.", &[]),
        &[("Cursor", true)],
    ),
    (
        (
            "Cursor: smoothing, hide when idle, loop, always arrow, rotate, remove shakes",
            &[],
        ),
        ("Cursor effect parameters.", &[]),
        &[("Cursor", true)],
    ),
    (
        ("Hide-cursor ranges", &[]),
        (
            "Cursor hidden per clip.",
            &[TextSpan::new(7, 13).color(MOTION)],
        ),
        &[("clip", true), ("v13 ranges", false)],
    ),
    (
        ("Click sound", &[]),
        (
            "Cursor clickSound in export.",
            &[TextSpan::new(7, 17).color(MOTION)],
        ),
        &[("export", true)],
    ),
    (
        (
            "Motion blur (cursor, zoom, pan); screen animation focused/smooth",
            &[],
        ),
        (
            "Cursor and Zoom motionBlur. Zoom spring presets.",
            &[
                TextSpan::new(16, 26).color(MOTION),
                TextSpan::new(33, 39).color(MOTION),
            ],
        ),
        &[("cursor / zoom", true)],
    ),
    (
        ("Camera: size, corner, roundness, shadow", &[]),
        ("Camera Layout effect.", &[]),
        &[("Camera Layout", true)],
    ),
    (
        ("Camera: mirror, shrink while zoomed", &[]),
        ("Camera Layout parameters.", &[]),
        &[("Camera Layout", true)],
    ),
    (
        ("Camera layout track (Default, Fullscreen, Hidden)", &[]),
        (
            "camera.layout keys (bubble/full/hidden) and component.set.",
            &[
                TextSpan::new(0, 13).color(MOTION),
                TextSpan::new(44, 57).color(MOTION),
            ],
        ),
        &[("keys", true), ("visibility", true)],
    ),
    (
        ("Keyboard-shortcut overlay", &[]),
        (
            "Keystroke Chips and per-event keys.hide.",
            &[TextSpan::new(30, 39).color(MOTION)],
        ),
        &[("Keystroke Chips", true)],
    ),
    (
        ("Audio volume and mute per track", &[]),
        (
            "audio.set and component.set.",
            &[
                TextSpan::new(0, 9).color(MOTION),
                TextSpan::new(14, 27).color(MOTION),
            ],
        ),
        &[("Audio", true)],
    ),
    (
        ("Mic noise reduction and normalise", &[]),
        (
            "Voice Isolation + audio.set LUFS.",
            &[TextSpan::new(18, 27).color(MOTION)],
        ),
        &[("macOS audio", true)],
    ),
    (
        ("Masks and highlights", &[]),
        (
            "Timed, stackable mask.add instances for blur, pixelate, cover or spotlight.",
            &[TextSpan::new(17, 25).color(MOTION)],
        ),
        &[("Masks", true)],
    ),
    (
        ("Speed up typing segments", &[]),
        (
            "intent.speedTyping from recorded or supplied ranges.",
            &[TextSpan::new(0, 18).color(MOTION)],
        ),
        &[("Typing ranges", true)],
    ),
    (
        ("Presets: save, apply, export JSON", &[]),
        (
            "JSON looks with look.save / look.apply.",
            &[
                TextSpan::new(16, 25).color(MOTION),
                TextSpan::new(28, 38).color(MOTION),
            ],
        ),
        &[("Looks", true)],
    ),
    (
        ("Captions, background music", &[]),
        (
            "Captions lanes with captions.generate. Music via music.add.",
            &[
                TextSpan::new(20, 37).color(MOTION),
                TextSpan::new(49, 58).color(MOTION),
            ],
        ),
        &[("Captions", true), ("Music", true)],
    ),
    (
        (
            "Recording (display, window, area, camera, audio, countdown, widgets)",
            &[],
        ),
        (
            "record.start / record.stop on macOS.",
            &[
                TextSpan::new(0, 12).color(MOTION),
                TextSpan::new(15, 26).color(MOTION),
            ],
        ),
        &[("capture", true), ("v13 widgets", false)],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::testing;

    #[test]
    fn native_effect_disclosure_uses_the_installed_catalogue() {
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let mut draw = |ui: &mut Ui| native_registry(ui, 448.0);
        testing::frame(&mut ui, &[], &mut draw);
        let trigger = testing::find(&ui, Role::Button, "Native effect types and scopes").unwrap();
        assert!(testing::find(&ui, Role::Table, "Native effect registry").is_none());
        let x = f64::from(trigger.x + trigger.w * 0.5);
        let y = f64::from(trigger.y + trigger.h * 0.5);
        testing::click(&mut ui, x, y, &mut draw);
        testing::frame(&mut ui, &[], &mut draw);
        assert!(testing::find(&ui, Role::Table, "Native effect registry").is_some());
        for (id, schema) in effect_schemas().as_object().unwrap() {
            let title = schema.field("title").and_then(Json::as_str).unwrap_or(id);
            let scope = schema.field("appliesTo").and_then(Json::as_str).unwrap();
            assert!(testing::find(&ui, Role::Cell, title).is_some(), "{title}");
            assert!(testing::find(&ui, Role::Cell, scope).is_some(), "{scope}");
        }
        assert!(testing::find(&ui, Role::Cell, "Click Zoom (0.3, retired)").is_none());
        testing::click(&mut ui, x, y, &mut draw);
        testing::frame(&mut ui, &[], &mut draw);
        assert!(testing::find(&ui, Role::Table, "Native effect registry").is_none());
    }
}
