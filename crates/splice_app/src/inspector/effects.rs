use super::{
    BODY, CAPTION, CHOICE_LOOK, FIELD, INK, INPUT, Intents, LINE, MUTED, PROPERTY_GAP, QUIET,
    SECTION_PAD, STRONG_LINE, Scope, choice_input, choice_option, choice_popup, ink,
    property_row_at, rule, slider_field, style_field,
};
use pxui::{
    Button, Fill, Node, Rect, Rgba, Size, Toggle, Ui, ValueCallbacks,
    geom::Edges,
    ui::{Look, States},
};
use splice_core::{
    commands::{Target, effect_schemas},
    edit::{effect_target_allowed, transition_capacity},
};
use splice_json::Value as Json;
use splice_model::{Effect, EffectValue, LaneId, Project, TimelineId};

const LABEL_WIDTH: f32 = 70.0;
const SOURCE_WIDTH: f32 = 36.0;
const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const TRACK: States =
    States::of(Look::NONE.fill(STRONG_LINE)).checked(Look::NONE.fill(Rgba::rgb(211, 228, 205)));
const KNOB: States =
    States::of(Look::NONE.fill(MUTED)).checked(Look::NONE.fill(Rgba::rgb(9, 9, 10)));
const SWITCH: Toggle<'static> = Toggle::DEFAULT
    .row_h(22.0)
    .switch(26.0, 14.0)
    .inset(0.0)
    .track(&TRACK)
    .knob(&KNOB)
    .press_row(true)
    .label(BODY);

pub(super) fn paint(
    ui: &mut Ui,
    project: &Project,
    owner: Target,
    width: f32,
    intents: &mut Intents,
) {
    let (scope, serial) = match owner {
        Target::Project => ("project-effects", 0),
        Target::Lane(id) => ("lane-effects", id.serial()),
        Target::Timeline(TimelineId::Clip(id)) => ("clip-effects", id.serial()),
        Target::Timeline(TimelineId::Item(id)) => ("item-effects", id.serial()),
        _ => return,
    };
    ui.id(scope, |ui| {
        ui.id(serial, |ui| {
            paint_owner(ui, project, &owner, width, intents)
        });
    });
}

fn paint_owner(ui: &mut Ui, project: &Project, owner: &Target, width: f32, intents: &mut Intents) {
    let (local, lane) = match *owner {
        Target::Project => (project.effects.as_slice(), None),
        Target::Lane(LaneId::SPINE) => (project.spine.effects.as_slice(), None),
        Target::Lane(id) => {
            let Some(lane) = project.lanes.iter().find(|lane| lane.id == id) else {
                return;
            };
            (lane.effects.as_slice(), None)
        }
        Target::Timeline(TimelineId::Clip(id)) => {
            let Some(data) = project
                .clips
                .find(id)
                .and_then(|slot| project.clips.data(slot))
            else {
                return;
            };
            (data.appearance.effects.as_slice(), Some(project.spine.id))
        }
        Target::Timeline(TimelineId::Item(id)) => {
            let Some(data) = project
                .items
                .find(id)
                .and_then(|slot| project.items.data(slot))
            else {
                return;
            };
            (data.appearance.effects.as_slice(), Some(data.lane))
        }
        _ => return,
    };
    let (lane_name, lane_effects) = match lane {
        Some(LaneId::SPINE) => ("Video lane", project.spine.effects.as_slice()),
        Some(id) => project
            .lanes
            .iter()
            .find(|lane| lane.id == id)
            .map_or(("Lane", &[][..]), |lane| {
                (lane.name.as_str(), lane.effects.as_slice())
            }),
        None => ("Lane", &[][..]),
    };
    let names = ["Project", lane_name, "Here"];
    let schemas = effect_schemas();
    let types = schemas.as_object().unwrap_or_default();
    let room = match *owner {
        Target::Timeline(TimelineId::Clip(id)) => transition_capacity(project, id),
        _ => 0,
    };
    let permitted = |kind: &str| effect_target_allowed(project, owner, kind);
    let sources = |kind: &str, allowed: bool| {
        [
            if allowed && inherits_project(owner, kind) {
                project.effects.as_slice()
            } else {
                &[]
            },
            if allowed && inherits_lane(kind) {
                lane_effects
            } else {
                &[]
            },
            local,
        ]
    };
    let addable = |kind: &str, schema: &Json| {
        permitted(kind)
            && (kind != "transition" || room >= 2)
            && (multiple(schema)
                || sources(kind, true)
                    .iter()
                    .all(|entries| !entries.iter().any(|effect| effect.kind == kind)))
    };
    let add_count = types
        .iter()
        .filter(|(kind, schema)| addable(kind, schema))
        .count();
    let card_width = (width - 2.0 * SECTION_PAD - 14.0).max(0.0);
    rule(ui);
    ui.column(
        Node::column().padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0)),
        |ui| {
            ui.row(
                Node::row().w(Size::Grow).h_px(20.0).center().gap(6.0),
                |ui| {
                    let header = ui.label("EFFECTS", CAPTION.grow().tracking(0.06));
                    ink(ui, header, QUIET);
                    if add_count > 0 {
                        let trigger = ui.button(
                            "add-effect",
                            "+ Effect",
                            &Button::sized(68.0, 18.0).label(CAPTION).look(&CHOICE_LOOK),
                        );
                        ui.id("add-effect-menu", |ui| {
                            choice_popup(
                                ui,
                                trigger,
                                ("Add effect", 190.0, add_count),
                                &mut |ui, opened| {
                                    let mut picked = false;
                                    for (index, (kind, schema)) in types
                                        .iter()
                                        .filter(|(kind, schema)| addable(kind, schema))
                                        .enumerate()
                                    {
                                        let first = opened && index == 0;
                                        ui.id(kind.as_str(), |ui| {
                                            if choice_option(
                                                ui,
                                                title(kind, schema),
                                                first,
                                                true,
                                                first,
                                            ) {
                                                let mut op =
                                                    owner_command("effect.add", owner, kind);
                                                if kind == "transition" {
                                                    let length =
                                                        room.min(transition_default(schema));
                                                    op.push((
                                                        "params".into(),
                                                        param(
                                                            "length",
                                                            Json::Signed(i64::from(length)),
                                                        ),
                                                    ));
                                                }
                                                intents.op = Some(Json::Object(op));
                                                picked = true;
                                            }
                                        });
                                    }
                                    picked
                                },
                            );
                        });
                    }
                },
            );
            let mut shown = false;
            for (kind, schema) in types {
                let card = sources(kind, permitted(kind));
                if multiple(schema) {
                    let mut group = local
                        .iter()
                        .filter(|effect| effect.kind == *kind)
                        .peekable();
                    if group.peek().is_some() {
                        let label = ui.label(title(kind, schema), CAPTION);
                        ink(ui, label, QUIET);
                        shown = true;
                    }
                    for effect in group {
                        ui.id(effect.id.serial(), |ui| {
                            paint_card(
                                ui,
                                project,
                                owner,
                                kind,
                                schema,
                                [&[], &[], std::slice::from_ref(effect)],
                                (names, lane),
                                (card_width, room),
                                intents,
                            );
                        });
                    }
                } else if card
                    .iter()
                    .any(|entries| entries.iter().any(|effect| effect.kind == *kind))
                {
                    shown = true;
                    ui.id(kind.as_str(), |ui| {
                        paint_card(
                            ui,
                            project,
                            owner,
                            kind,
                            schema,
                            card,
                            (names, lane),
                            (card_width, room),
                            intents,
                        );
                    });
                }
            }
            for effect in local
                .iter()
                .filter(|effect| schemas.field(&effect.kind).is_none())
            {
                shown = true;
                ui.id(effect.id.serial(), |ui| {
                    paint_card(
                        ui,
                        project,
                        &owner,
                        &effect.kind,
                        &Json::Null,
                        [&[], &[], std::slice::from_ref(effect)],
                        (names, lane),
                        (card_width, room),
                        intents,
                    );
                });
            }
            if !shown {
                let hint = ui.label("No effects reach this selection.", CAPTION.wrap());
                ink(ui, hint, QUIET);
            }
        },
    );
}

fn multiple(schema: &Json) -> bool {
    schema.field("multiple").and_then(Json::as_bool) == Some(true)
}

// The native evaluator uses these effects with a scope chain. Crop, mask, cutout,
// transition, and other item-specific effects read only the owner's definition.
fn inherits_lane(kind: &str) -> bool {
    matches!(
        kind,
        "window"
            | "cursor"
            | "camera"
            | "keystrokes"
            | "color"
            | "volume"
            | "eq"
            | "compressor"
            | "gate"
            | "voiceIsolation"
            | "captions"
            | "text"
            | "animate"
            | "layout"
    )
}

fn inherits_project(owner: &Target, kind: &str) -> bool {
    if matches!(owner, Target::Project) {
        return false;
    }
    kind != "window" && inherits_lane(kind)
        || kind == "window"
            && matches!(
                owner,
                Target::Lane(LaneId::SPINE) | Target::Timeline(TimelineId::Clip(_))
            )
}

fn paint_card(
    ui: &mut Ui,
    project: &Project,
    owner: &Target,
    kind: &str,
    schema: &Json,
    scopes: [&[Effect]; 3],
    (names, lane): ([&str; 3], Option<LaneId>),
    (width, room): (f32, i32),
    intents: &mut Intents,
) {
    let layers = scopes.map(|scope| scope.iter().rev().find(|effect| effect.kind == kind));
    let local = layers[2];
    let (enabled, enabled_at) = scopes
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, scope)| {
            scope
                .iter()
                .rev()
                .filter(|effect| effect.kind == kind)
                .find_map(|effect| effect.enabled.map(|on| (on, index)))
        })
        .unwrap_or((true, 3));
    let title = schema.field("title").and_then(Json::as_str).unwrap_or(kind);
    let fields = schema
        .field("params")
        .and_then(Json::as_object)
        .unwrap_or_default();
    let transition_min = schema
        .field("params")
        .and_then(|p| p.field("length"))
        .and_then(|p| p.field("min"))
        .and_then(Json::as_i64)
        .unwrap_or(0);
    let editable = !matches!(schema, Json::Null)
        && (kind != "transition" || i64::from(room) >= transition_min);
    ui.column(
        Node::column()
            .w(Size::Grow)
            .padding(Edges::new(6.0, 4.0, 6.0, 6.0))
            .margin(Edges::new(0.0, 6.0, 0.0, 0.0))
            .look(Look::NONE.fill(INPUT).border(LINE, 1.0)),
        |ui| {
            ui.row(
                Node::row().w(Size::Grow).h_px(22.0).center().gap(6.0),
                |ui| {
                    ui.column(Node::column().w_px(26.0), |ui| {
                        let toggle = ui.toggle("enabled", "", enabled, &SWITCH.disabled(!editable));
                        ui.a11y_name(&toggle.handle, title);
                        if toggle.changed {
                            intents.op =
                                Some(write(owner, kind, local, "enabled", Json::Bool(!enabled)));
                        }
                    });
                    let name = ui.label(title, BODY.grow().ellipsis());
                    ink(ui, name, if enabled { INK } else { MUTED });
                    options(
                        ui,
                        project,
                        owner,
                        kind,
                        title,
                        layers,
                        (names, lane),
                        intents,
                    );
                },
            );
            ui.row(
                Node::row().w(Size::Grow).gap(4.0).row_gap(2.0).wrap(),
                |ui| {
                    for (index, entry) in layers.iter().enumerate() {
                        if entry.is_some() {
                            let chip = ui.label(names[index], CAPTION.pad(2.0));
                            let color = if index == 2 { ACCENT } else { QUIET };
                            ui.style(
                                &chip,
                                &States::of(
                                    Look::NONE
                                        .border(if index == 2 { ACCENT } else { STRONG_LINE }, 1.0)
                                        .text(color),
                                ),
                                Default::default(),
                            );
                        }
                    }
                },
            );
            if !editable && kind == "transition" {
                let hint = ui.label("No usable transition handles.", CAPTION.wrap());
                ink(ui, hint, QUIET);
            } else if !enabled {
                let from = names.get(enabled_at).copied().unwrap_or("Default");
                let hint =
                    ui.label_fmt(format_args!("Off \u{b7} {from}"), CAPTION.grow().ellipsis());
                ink(ui, hint, QUIET);
            } else {
                for (index, (key, definition)) in fields.iter().enumerate() {
                    ui.id(index as u64, |ui| {
                        let label = definition
                            .field("label")
                            .and_then(Json::as_str)
                            .unwrap_or(key);
                        let found = scopes
                            .iter()
                            .enumerate()
                            .rev()
                            .find_map(|(scope, effects)| {
                                effects
                                    .iter()
                                    .rev()
                                    .filter(|effect| effect.kind == kind)
                                    .find_map(|effect| {
                                        let value =
                                            effect.params.iter().rev().find(|p| p.name == *key)?;
                                        Some((scope, effect, &value.value))
                                    })
                            });
                        let field_owner = found
                            .filter(|(scope, _, _)| *scope == 2)
                            .map(|(_, effect, _)| effect)
                            .or(local);
                        property_row_at(ui, label, LABEL_WIDTH, &mut |ui| {
                            let field_width =
                                (width - LABEL_WIDTH - 2.0 * PROPERTY_GAP - SOURCE_WIDTH).max(0.0);
                            ui.row(Node::row().w_px(field_width).h_px(22.0).center(), |ui| {
                                if editable
                                    && let Some(value) = control(
                                        ui,
                                        label,
                                        definition,
                                        found.map(|(_, _, value)| value),
                                        field_width,
                                        (kind == "transition" && key == "length")
                                            .then_some(f64::from(room)),
                                    )
                                {
                                    intents.op = Some(write(
                                        owner,
                                        kind,
                                        field_owner,
                                        "params",
                                        param(key, value),
                                    ));
                                }
                            });
                            let scope = found.map_or(3, |(scope, _, _)| scope);
                            let can_reset = key != "length"
                                || kind != "transition"
                                || room >= transition_default(schema);
                            if scope == 2 && can_reset {
                                let name = with_name("Reset ", label);
                                let button = ui.button(
                                    "reset-param",
                                    "",
                                    &Button::sized(SOURCE_WIDTH, 20.0)
                                        .name(name.as_str())
                                        .look(&CHOICE_LOOK),
                                );
                                ui.canvas(&button.handle, |paint, rect| {
                                    paint.circle(
                                        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5),
                                        4.0,
                                        Some(Fill::new(ACCENT)),
                                        None,
                                    );
                                });
                                if button.clicked {
                                    intents.op = Some(write(
                                        owner,
                                        kind,
                                        field_owner,
                                        "reset",
                                        Json::Array(vec![Json::String(key.clone())]),
                                    ));
                                }
                            } else {
                                let label = ui.label(
                                    names.get(scope).copied().unwrap_or("Default"),
                                    CAPTION.size(9.0).w_px(SOURCE_WIDTH).ellipsis(),
                                );
                                ink(ui, label, QUIET);
                            }
                        });
                    });
                }
            }
        },
    );
}

#[derive(Clone, Copy)]
enum Action {
    Remove,
    Override,
    Off,
    Lane(bool),
    Project,
    Upstream,
}

fn options(
    ui: &mut Ui,
    project: &Project,
    owner: &Target,
    kind: &str,
    title: &str,
    layers: [Option<&Effect>; 3],
    (names, lane): ([&str; 3], Option<LaneId>),
    intents: &mut Intents,
) {
    let local = layers[2];
    let inherited = layers[0].is_some() || layers[1].is_some();
    let to_lane = inherits_lane(kind)
        && lane.is_some_and(|id| effect_target_allowed(project, &Target::Lane(id), kind));
    let to_project = matches!(owner, Target::Lane(_))
        && inherits_project(owner, kind)
        && effect_target_allowed(project, &Target::Project, kind);
    let upstream_name = with_name(
        "Edit on ",
        if layers[1].is_some() {
            names[1]
        } else {
            names[0]
        },
    );
    let rows = [
        (
            if inherited {
                "Reset to inherited"
            } else {
                "Remove effect"
            },
            Action::Remove,
            local.is_some(),
        ),
        ("Override here", Action::Override, local.is_none()),
        ("Turn off here", Action::Off, local.is_none()),
        (
            "Use for whole lane",
            Action::Lane(false),
            local.is_some() && to_lane,
        ),
        (
            "Use for whole lane, clear other overrides",
            Action::Lane(true),
            local.is_some() && to_lane,
        ),
        (
            "Use for project",
            Action::Project,
            local.is_some() && to_project,
        ),
        (upstream_name.as_str(), Action::Upstream, inherited),
    ];
    let accessible = with_name(title, " options");
    let trigger = ui.button(
        "effect-options",
        "\u{22ef}",
        &Button::sized(20.0, 20.0)
            .name(accessible.as_str())
            .label(BODY)
            .look(&CHOICE_LOOK),
    );
    let mut choice = None;
    ui.id("options-menu", |ui| {
        choice_popup(
            ui,
            trigger,
            (
                accessible.as_str(),
                256.0,
                rows.iter().filter(|row| row.2).count(),
            ),
            &mut |ui, opened| {
                let mut first = opened;
                for (index, &(label, action, _)) in rows.iter().enumerate().filter(|(_, row)| row.2)
                {
                    ui.id(index as u64, |ui| {
                        if choice_option(ui, label, first, true, first) {
                            choice = Some(action);
                        }
                    });
                    first = false;
                }
                choice.is_some()
            },
        );
    });
    if let Some(action) = choice {
        match action {
            Action::Remove => {
                let effect = local.expect("local option");
                intents.op = Some(Json::Object(vec![
                    ("cmd".into(), Json::String("effect.remove".into())),
                    (
                        "effects".into(),
                        Json::Array(vec![Json::String(effect.id.to_string())]),
                    ),
                ]));
            }
            Action::Override | Action::Off => {
                let mut op = owner_command("effect.add", owner, kind);
                if matches!(action, Action::Off) {
                    op.push(("enabled".into(), Json::Bool(false)));
                }
                intents.op = Some(Json::Object(op));
            }
            Action::Lane(_) | Action::Project => {
                let effect = local.expect("local option");
                let to = if matches!(action, Action::Project) {
                    "project"
                } else {
                    "lane"
                };
                intents.op = Some(Json::Object(vec![
                    ("cmd".into(), Json::String("effect.promote".into())),
                    ("effect".into(), Json::String(effect.id.to_string())),
                    ("to".into(), Json::String(to.into())),
                    (
                        "clearOverrides".into(),
                        Json::Bool(matches!(action, Action::Lane(true))),
                    ),
                ]));
            }
            Action::Upstream => {
                if layers[1].is_some() {
                    intents.select_lane = lane;
                } else {
                    intents.scope = Some(Scope::Canvas);
                }
            }
        }
    }
}

fn control(
    ui: &mut Ui,
    label: &str,
    schema: &Json,
    value: Option<&EffectValue>,
    width: f32,
    upper: Option<f64>,
) -> Option<Json> {
    if let Some(EffectValue::Keys(keys)) = value {
        let label = ui.label_fmt(
            format_args!("Animated \u{b7} {}", keys.len()),
            CAPTION.grow().ellipsis(),
        );
        ink(ui, label, QUIET);
        return None;
    }
    let default = schema.field("def").unwrap_or(&Json::Null);
    let text = match value {
        Some(EffectValue::Text(text)) => Some(text.as_str()),
        _ => default.as_str(),
    };
    match schema.field("type").and_then(Json::as_str) {
        Some("bool") => {
            let on = match value {
                Some(EffectValue::Bool(on)) => *on,
                _ => default.as_bool().unwrap_or(false),
            };
            let button = ui.toggle("parameter-value", "", on, &SWITCH);
            ui.a11y_name(&button.handle, label);
            button.changed.then_some(Json::Bool(!on))
        }
        Some("number") => {
            let number = match value {
                Some(EffectValue::Number(n)) => Some(*n),
                _ => default.as_f64(),
            };
            let min = schema.field("min").and_then(Json::as_f64)?;
            let max = schema.field("max").and_then(Json::as_f64)?;
            let step = schema.field("step").and_then(Json::as_f64).unwrap_or(0.01);
            let max = upper.map_or(max, |cap| max.min(cap));
            number_input(ui, label, number, (min, max, step), width).map(Json::Float)
        }
        Some("enum") => {
            let values = schema
                .field("options")
                .and_then(Json::as_array)
                .unwrap_or_default();
            let mut chosen = None;
            choice_input(
                ui,
                label,
                text.unwrap_or("Unset"),
                width,
                values.len(),
                &mut |ui, opened| {
                    for (index, option) in values.iter().filter_map(Json::as_str).enumerate() {
                        ui.id(index as u64, |ui| {
                            if choice_option(ui, option, text == Some(option), true, opened) {
                                chosen = Some(option);
                            }
                        });
                    }
                    chosen.is_some()
                },
            );
            chosen
                .filter(|&option| Some(option) != text)
                .map(|option| Json::String(option.into()))
        }
        Some("color") => {
            let text = text.unwrap_or("");
            let color = hex(text).unwrap_or(0);
            let swatch = ui.node("swatch", Node::DEFAULT.w_px(14.0).h_px(14.0));
            ui.canvas(&swatch, move |paint, rect| {
                paint.fill(
                    rect,
                    Fill::new(Rgba::rgb(
                        (color >> 16) as u8,
                        (color >> 8) as u8,
                        color as u8,
                    )),
                );
            });
            let pos = 18.0_f32.min(width);
            let valid = |text: &str| hex(text).is_some();
            let response = pxui::ext::value_field(
                ui,
                "parameter-value",
                text,
                Rect::new(pos, 0.0, width - pos, 22.0),
                &FIELD.name(label),
                &ValueCallbacks::DEFAULT.valid(&valid),
            );
            let changed = response
                .changed
                .filter(|&value| value != text)
                .map(|value| Json::String(value.into()));
            let (signal, editing) = (response.response, response.editing);
            style_field(ui, signal, editing, text, "Unset");
            changed
        }
        _ => {
            let label = ui.label(text.unwrap_or("Unset"), CAPTION.grow().ellipsis());
            ink(ui, label, QUIET);
            None
        }
    }
}

fn number_input(
    ui: &mut Ui,
    label: &str,
    stored: Option<f64>,
    (min, max, step): (f64, f64, f64),
    width: f32,
) -> Option<f64> {
    if min > max {
        return None;
    }
    let box_width = width.min(52.0);
    let slider = slider_field(
        ui,
        ("parameter-slider", label),
        stored.unwrap_or(min).clamp(min, max) as f32,
        (min as f32, max as f32, step as f32),
        (width - box_width - 6.0).max(0.0),
    );
    let display_value = |value: f32| {
        let n = min + ((f64::from(value) - min) / step).round() * step;
        ((n * 1e6).round() / 1e6).clamp(min, max)
    };
    let shown = slider.preview.map(display_value).or(stored);
    let mut value_text = [0u8; 48];
    let mut rest = value_text.as_mut_slice();
    use std::io::Write;
    if let Some(value) = shown {
        if value != 0.0 && !(1e-6..1e12).contains(&value.abs()) {
            write!(rest, "{value:e}").expect("finite value fits");
        } else {
            write!(rest, "{value}").expect("finite value fits");
        }
    }
    let len = 48 - rest.len();
    let text = std::str::from_utf8(&value_text[..len]).expect("decimal digits");
    let parse = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|n| (min..=max).contains(n))
    };
    let valid = |text: &str| parse(text).is_some();
    let response = pxui::ext::value_field(
        ui,
        "parameter-value",
        text,
        Rect::new(width - box_width, 0.0, box_width, 22.0),
        &FIELD.name(label),
        &ValueCallbacks::DEFAULT.valid(&valid),
    );
    let change = response.changed.and_then(parse);
    let (signal, editing) = (response.response, response.editing);
    style_field(ui, signal, editing, text, "Unset");
    slider
        .commit
        .map(display_value)
        .or(change)
        .filter(|&v| stored != Some(v))
}

fn hex(raw: &str) -> Option<u32> {
    let hex = raw.strip_prefix('#')?;
    (hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

fn transition_default(schema: &Json) -> i32 {
    schema
        .field("params")
        .and_then(|p| p.field("length"))
        .and_then(|p| p.field("def"))
        .and_then(Json::as_i64)
        .unwrap_or(15) as i32
}

fn title(kind: &'static str, schema: &'static Json) -> &'static str {
    schema.field("title").and_then(Json::as_str).unwrap_or(kind)
}

// Small strings borrowed by a single PXUI call, including accessible parameter names.
struct Name {
    bytes: [u8; 256],
    used: usize,
}

impl Name {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.used]).expect("UI name")
    }
}

fn with_name(before: &str, after: &str) -> Name {
    let mut name = Name {
        bytes: [0; 256],
        used: 0,
    };
    for part in [before, after] {
        let mut len = part.len().min(name.bytes.len() - name.used);
        while !part.is_char_boundary(len) {
            len -= 1;
        }
        name.bytes[name.used..name.used + len].copy_from_slice(&part.as_bytes()[..len]);
        name.used += len;
    }
    name
}

fn owner_command(cmd: &str, owner: &Target, kind: &str) -> Vec<(String, Json)> {
    let target = match *owner {
        Target::Timeline(id) => id.to_string(),
        Target::Lane(id) => id.to_string(),
        Target::Project => "project".into(),
        _ => unreachable!("Inspector effect owner"),
    };
    vec![
        ("cmd".into(), Json::String(cmd.into())),
        ("target".into(), Json::String(target)),
        ("type".into(), Json::String(kind.into())),
    ]
}

fn param(key: &str, value: Json) -> Json {
    Json::Object(vec![(key.into(), value)])
}

fn write(owner: &Target, kind: &str, local: Option<&Effect>, key: &str, value: Json) -> Json {
    let mut op = match local {
        Some(effect) => vec![
            ("cmd".into(), Json::String("effect.set".into())),
            ("effect".into(), Json::String(effect.id.to_string())),
        ],
        None => owner_command("effect.set", owner, kind),
    };
    op.push((key.into(), value));
    Json::Object(op)
}
