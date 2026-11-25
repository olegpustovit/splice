use pxui::{
    A11yStates, Button, Fill, Flags, Handle, Label, Node, Overlay, Point, Rect, Response, Role,
    Scrubber, Stroke, Toggle, Ui, ValueCallbacks, ValueField,
    geom::{Edges, Rgba},
    ui::{Align, Along, Axis, Bar, Look, SectionList, SectionPart, Size, State, States},
};
use splice_av::{SourceSpan, TimelineEvents, TimelineKey};
use splice_core::{
    commands::{Component, Target},
    edit::{Span, span},
};
use splice_json::Value as Json;
use splice_model::{
    Access, Anchor, Appearance, Aspect, ClipData, ClipId, ClipTiming, EffectValue, Fit, Framing,
    ItemId, ItemKind, Lane, LaneId, LaneKind, Marker, MarkerId, MediaId, MediaKind, Placement,
    Project, Slot, Spring, TimelineId, Zoom, timing,
};

pub(super) mod captions;
#[path = "inspector/effects.rs"]
mod effects;

const PANEL: Rgba = Rgba::rgb(15, 15, 16);
const INPUT: Rgba = Rgba::rgb(11, 11, 12);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const STRONG_LINE: Rgba = Rgba::rgb(56, 56, 59);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const HOVER: Rgba = Rgba::rgb(34, 34, 36);
const SELECTED: Rgba = Rgba::rgb(42, 42, 45);
const CONTROL: Look = Look::NONE.text(QUIET);
const SEGMENT: States = States::of(CONTROL)
    .hover(CONTROL.fill(HOVER).text(INK))
    .checked(CONTROL.fill(SELECTED).text(INK));
const CAPTION: Label = Label::MONO.size(10.0).line_height(14.0);
const BODY: Label = Label::MONO.size(11.0).line_height(15.4);
const SECTION_PAD: f32 = 12.0;
const PROPERTY_LABEL: f32 = 78.0;
const PROPERTY_GAP: f32 = 8.0;
const FIELD: ValueField<'static> = ValueField::DEFAULT.size(11).left(true);
const CHOICE_FIELD: Look = Look::NONE.fill(INPUT).border(LINE, 1.0).text(INK);
const CHOICE_LOOK: States = States::of(CHOICE_FIELD)
    .hover(CHOICE_FIELD.border(STRONG_LINE, 1.0))
    .focus(CHOICE_FIELD.border(MUTED, 1.0))
    .disabled(CHOICE_FIELD.text(QUIET));
const PLACEMENTS: [(Placement, &str); 6] = [
    (Placement::Full, "full"),
    (Placement::Card, "card"),
    (Placement::TopLeft, "corner-tl"),
    (Placement::TopRight, "corner-tr"),
    (Placement::BottomLeft, "corner-bl"),
    (Placement::BottomRight, "corner-br"),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Selection {
    #[default]
    None,
    Clip(ClipId),
    Component(ClipId, Component, Option<u32>),
    Item(ItemId),
    Lane(LaneId),
    Marker(MarkerId),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Scope {
    #[default]
    Selection,
    Canvas,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Focus {
    Speed(ClipId),
    Text(ItemId),
}

#[derive(Default)]
pub(super) struct Intents {
    pub scope: Option<Scope>,
    pub op: Option<Json>,
    pub seek: Option<i32>,
    pub select_parent: Option<ClipId>,
    pub crop: Option<TimelineId>,
    pub select_lane: Option<LaneId>,
    pub copy_srt: bool,
    pub focused_field: Option<Handle>,
    pub focus_scroll: Option<Handle>,
}

enum Content {
    Sequence,
    Group,
    Canvas,
    Clip(ClipId, Slot),
    Component(ClipId, Slot, Component, Option<u32>),
    Item(Slot),
    Spine,
    Lane(usize),
    Marker(usize),
}
pub(super) fn paint(
    ui: &mut Ui,
    project: Option<&Project>,
    selection: Selection,
    scope: Scope,
    end: i32,
    (width, inline): (f32, bool),
    (group, ripple, events, caption, focus): (
        &[TimelineId],
        bool,
        Option<(&TimelineEvents, (SourceSpan, &[f64]))>,
        Option<(&captions::Cache, i32)>,
        Option<Focus>,
    ),
) -> Intents {
    let (content, title) = project.map_or((Content::Sequence, "Sequence"), |project| {
        if scope == Scope::Canvas {
            return (Content::Canvas, "Project");
        }
        if group.len() > 1 {
            return (Content::Group, "Multiple");
        }
        match selection {
            Selection::None => None,
            Selection::Clip(id) => project.clips.find(id).and_then(|slot| {
                let title = if project.clips.data(slot)?.media.is_none() {
                    "Gap clip"
                } else {
                    "Spine clip"
                };
                Some((Content::Clip(id, slot), title))
            }),
            Selection::Component(id, component, key) => project.clips.find(id).and_then(|slot| {
                component_info(project.clips.data(slot)?, component)
                    .map(|_| (Content::Component(id, slot, component, key), "Component"))
            }),
            Selection::Item(id) => project.items.find(id).and_then(|slot| {
                let title = match project.items.data(slot)?.kind {
                    ItemKind::Title { .. } => "Title",
                    ItemKind::Media(_) => "Overlay",
                    ItemKind::Motion(_) | ItemKind::Zoom(_) => "Scale animation",
                    ItemKind::Sound(_) => "Audio",
                    ItemKind::Shape { .. } => "Shape",
                    ItemKind::Caption { .. } => "Caption",
                };
                Some((Content::Item(slot), title))
            }),
            Selection::Lane(id) if id == project.spine.id => Some((Content::Spine, "Lane")),
            Selection::Lane(id) => project
                .lanes
                .iter()
                .position(|lane| lane.id == id)
                .map(|index| (Content::Lane(index), "Lane")),
            Selection::Marker(id) => project
                .markers
                .iter()
                .position(|marker| marker.id == id)
                .map(|index| (Content::Marker(index), "Marker")),
        }
        .unwrap_or((Content::Sequence, "Sequence"))
    });
    let focus = focus.filter(|_| {
        !ui.captured()
            && !ui.text_input_focused()
            && !ui.key_pressed(pxui::KeyCode::Escape, pxui::Mods::ANY)
    });
    let mut intents = Intents::default();
    ui.column(
        Node {
            w: Size::Px(width),
            h: if inline { Size::Fit } else { Size::Grow },
            look: Look::NONE.fill(PANEL),
            ..Node::DEFAULT
        },
        |ui| {
            if inline {
                rule(ui);
            }
            ui.row(
                Node {
                    h: Size::Px(29.0),
                    align: Align::Center,
                    padding: Edges::new(8.0, 0.0, 8.0, 0.0),
                    ..Node::DEFAULT
                },
                |ui| {
                    ui.row(
                        Node::row()
                            .w_px(133.0)
                            .h_px(22.0)
                            .look(Look::NONE.border(LINE, 1.0))
                            .padding(Edges::all(1.0)),
                        |ui| {
                            for (value, label, width) in [
                                (Scope::Selection, "Selection", 75.0),
                                (Scope::Canvas, "Canvas", 56.0),
                            ] {
                                if segment(ui, label, width, scope == value) {
                                    intents.scope = Some(value);
                                }
                            }
                        },
                    );
                    ui.node("scope-space", Node::DEFAULT);
                    let caption = ui.label(title, CAPTION);
                    ink(ui, caption, QUIET);
                },
            );
            rule(ui);
            let body = if inline {
                Node::column().w(Size::Grow).h(Size::Fit).role(Role::Group)
            } else {
                Node::column()
                    .h(Size::Grow)
                    .flags(Flags::SCROLL | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
                    .role(Role::ScrollArea)
                    .bar(Bar::new(STRONG_LINE, 6.0).gap(2.0))
            };
            let body = ui.node(title, body);
            ui.a11y_name(&body, "Inspector");
            ui.within(&body, &mut |ui| {
                let Some(project) = project else { return };
                ui.node("top-space", Node::DEFAULT.h_px(8.0));
                match content {
                    Content::Sequence => sequence(ui, project, end),
                    Content::Group => group_selection(ui, project, group, ripple, &mut intents),
                    Content::Canvas => canvas(ui, project, end, width, &mut intents),
                    Content::Clip(id, slot) => {
                        let (Some(clip), Some(data)) =
                            (project.clips.timing(slot), project.clips.data(slot))
                        else {
                            return;
                        };
                        let media = data
                            .media
                            .and_then(|id| project.media.iter().find(|media| media.id == id));
                        let custom_name = data.appearance.name.as_deref();
                        let media_name = media
                            .and_then(|media| std::path::Path::new(&media.path).file_stem())
                            .and_then(std::ffi::OsStr::to_str)
                            .unwrap_or("Gap");
                        heading(
                            ui,
                            format_args!("{title} · {id}"),
                            custom_name.unwrap_or(media_name),
                        );
                        let field = property_field(width);
                        section(ui, "CLIP", &mut |ui| {
                            ui.id(id.serial(), |ui| {
                                clip_name(
                                    ui,
                                    id,
                                    custom_name.unwrap_or(""),
                                    media_name,
                                    field,
                                    &mut intents,
                                );
                                let fps = project.fps.max(1);
                                let start = clip.start.max(0) as u32;
                                let seconds = start / fps;
                                numeric(
                                    ui,
                                    "Start",
                                    format_args!(
                                        "{} f · {:02}:{:02}:{:02}",
                                        clip.start,
                                        seconds / 60,
                                        seconds % 60,
                                        start % fps
                                    ),
                                );
                                let out = clip.start.saturating_add(clip.length);
                                if let Some(at) = frame_field(
                                    ui,
                                    "In",
                                    clip.start,
                                    0,
                                    out.saturating_sub(3),
                                    field,
                                ) {
                                    intents.op = Some(trim_op(TimelineId::Clip(id), "in", at));
                                }
                                if let Some(at) = frame_field(
                                    ui,
                                    "Out",
                                    out,
                                    clip.start.saturating_add(3),
                                    i32::MAX,
                                    field,
                                ) {
                                    intents.op = Some(trim_op(TimelineId::Clip(id), "out", at));
                                }
                            });
                            numeric(
                                ui,
                                "Source",
                                format_args!("from {} f · {} f long", clip.source, clip.length),
                            );
                        });
                        if data.media.is_some() {
                            section(ui, "SPEED", &mut |ui| {
                                segment_group(ui, &mut |ui| {
                                    for (speed, label) in [
                                        (0.5, "0.5×"),
                                        (1.0, "1×"),
                                        (1.5, "1.5×"),
                                        (2.0, "2×"),
                                        (4.0, "4×"),
                                        (8.0, "8×"),
                                    ] {
                                        if segment(ui, label, 37.0, clip.speed == speed)
                                            && clip.speed != speed
                                        {
                                            intents.op = Some(speed_op(id, speed, ripple));
                                        }
                                    }
                                });
                                ui.id(id.serial(), |ui| {
                                    if let Some(speed) = level_control(
                                        ui,
                                        "Custom",
                                        (clip.speed, 0.25, 16.0, 2),
                                        (
                                            width,
                                            (focus == Some(Focus::Speed(id)))
                                                .then_some(&mut intents.focused_field),
                                        ),
                                    ) {
                                        intents.op = Some(speed_op(id, speed, ripple));
                                    }
                                });
                            });
                        }
                        if let Some(media) = media.filter(|media| media.kind != MediaKind::Audio) {
                            framing(
                                ui,
                                TimelineId::Clip(id),
                                data.framing,
                                [media.probe.width, media.probe.height],
                                &mut intents,
                            );
                        }
                        item_fades(
                            ui,
                            TimelineId::Clip(id),
                            &data.appearance,
                            (clip.length, 150),
                            property_field(width),
                            &mut intents,
                        );
                        clip_details(
                            ui,
                            project,
                            id,
                            data,
                            clip.speed == 1.0,
                            width,
                            &mut intents,
                        );
                    }
                    Content::Component(id, slot, component, key) => selected_component(
                        ui,
                        project,
                        (id, slot, component, key),
                        events,
                        width,
                        &mut intents,
                    ),
                    Content::Item(slot) => selected_item(
                        ui,
                        project,
                        slot,
                        title,
                        width,
                        (
                            caption.and_then(|(cache, at)| cache.cue.as_ref().map(|cue| (cue, at))),
                            focus,
                        ),
                        &mut intents,
                    ),
                    Content::Spine => {
                        let spine = &project.spine;
                        heading(ui, format_args!("Lane · {}", spine.id), &spine.name);
                        section(ui, "LANE", &mut |ui| {
                            numeric(ui, "Name", format_args!("{} · magnetic spine", spine.name));
                            property(ui, "Kind", "spine");
                            numeric(ui, "Items", format_args!("{}", project.clips.order().len()));
                        });
                        effects::paint(ui, project, Target::Lane(spine.id), width, &mut intents);
                    }
                    Content::Lane(index) => {
                        selected_lane(
                            ui,
                            project,
                            &project.lanes[index],
                            width,
                            caption.and_then(|(cache, _)| cache.lane.as_ref()),
                            &mut intents,
                        );
                    }
                    Content::Marker(index) => {
                        selected_marker(ui, &project.markers[index], width, ripple, &mut intents);
                    }
                }
                ui.node("bottom-space", Node::DEFAULT.h_px(20.0));
            });
            if !inline && intents.focused_field.is_some() {
                intents.focus_scroll = Some(body);
            }
        },
    );
    intents
}

/// Reveals the focused control once PXUI has laid out the Inspector for its new owner.
pub(super) fn reveal_focus(ui: &mut Ui, intents: &Intents) -> bool {
    let Some((scroll, field)) = intents.focus_scroll.zip(intents.focused_field) else {
        return false;
    };
    let viewport = ui.rect(&scroll);
    let field = ui.rect(&field);
    if viewport.h <= 0.0 || field.h <= 0.0 {
        return false;
    }
    let top = field.y - viewport.y;
    let bottom = field.y + field.h - viewport.y - viewport.h;
    let delta = if top < 0.0 { top } else { bottom.max(0.0) };
    if delta == 0.0 {
        return false;
    }
    let offset = (ui.scroll(&scroll).painted + delta).max(0.0);
    ui.set_scroll_offset(&scroll, offset);
    true
}

fn component_info(
    data: &ClipData,
    component: Component,
) -> Option<(&'static str, &'static str, bool)> {
    let enabled = &data.enabled;
    let (key, name, on) = match component {
        Component::Screen => ("screen", "Screen video", enabled.screen),
        Component::Camera => ("camera", "Camera video", enabled.camera),
        Component::Mic => ("mic", "Microphone", enabled.mic),
        Component::System => ("system", "System audio", enabled.system),
        Component::Pointer => ("pointer", "Cursor", enabled.pointer),
        Component::Keys => ("keys", "Keystrokes", enabled.keys),
    };
    Some((key, name, on?))
}

fn selected_component(
    ui: &mut Ui,
    project: &Project,
    (id, slot, component, key): (ClipId, Slot, Component, Option<u32>),
    events: Option<(&TimelineEvents, (SourceSpan, &[f64]))>,
    width: f32,
    intents: &mut Intents,
) {
    let (Some(data), Some(clip)) = (project.clips.data(slot), project.clips.timing(slot)) else {
        return;
    };
    let Some((part, title, on)) = component_info(data, component) else {
        return;
    };
    let name = data
        .appearance
        .name
        .as_deref()
        .or_else(|| group_media_name(project, data.media?))
        .unwrap_or("Take");
    let section_title = match component {
        Component::Screen | Component::Camera => "PICTURE",
        Component::Mic | Component::System => "AUDIO",
        Component::Pointer => "CURSOR",
        Component::Keys => "KEYSTROKES",
    };

    ui.id(id.serial(), |ui| {
        ui.id(part, |ui| {
            heading(ui, format_args!("Component · {name}"), title);
            section(ui, section_title, &mut |ui| {
                let toggle = Toggle::DEFAULT.row_h(26.0).label(BODY);
                if ui
                    .toggle("visible", "Visible on this clip", on, &toggle)
                    .changed
                {
                    intents.op = Some(component_op(
                        id,
                        "component.set",
                        "enabled",
                        Json::Bool(!on),
                        Some(part),
                    ));
                }
                match component {
                    Component::Screen | Component::Camera => {}
                    Component::Mic | Component::System => {
                        let gain = if component == Component::Mic {
                            data.volume.mic
                        } else {
                            data.volume.system
                        };
                        if let Some(percent) = level_control(
                            ui,
                            "Volume",
                            (gain * 100.0, 0.0, 200.0, 0),
                            (width, None),
                        ) {
                            intents.op = Some(component_op(
                                id,
                                "audio.set",
                                "volume",
                                Json::Float(percent / 100.0),
                                Some(part),
                            ));
                        }
                    }
                    Component::Pointer => {
                        let (size, rings) = component_style(project, data, "cursor");
                        if let Some(size) =
                            level_control(ui, "Size", (size, 0.5, 4.0, 1), (width, None))
                        {
                            intents.op = Some(component_op(
                                id,
                                "cursor.set",
                                "size",
                                Json::Float(size),
                                None,
                            ));
                        }
                        if ui
                            .toggle("click-rings", "Click rings", rings, &toggle)
                            .changed
                        {
                            intents.op = Some(component_op(
                                id,
                                "cursor.set",
                                "clickRings",
                                Json::Bool(!rings),
                                None,
                            ));
                        }
                    }
                    Component::Keys => {
                        let (size, _) = component_style(project, data, "keystrokes");
                        if let Some(size) =
                            level_control(ui, "Chip size", (size, 0.5, 3.0, 1), (width, None))
                        {
                            intents.op = Some(component_op(
                                id,
                                "keys.set",
                                "size",
                                Json::Float(size),
                                None,
                            ));
                        }
                    }
                }
            });
            if component == Component::Keys {
                recorded_keys(ui, project, (id, clip, data, key), events, width, intents);
            }
            rule(ui);
            ui.row(
                Node::DEFAULT.padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0)),
                |ui| {
                    if segment(ui, "Select parent clip", 144.0, false) {
                        intents.select_parent = Some(id);
                    }
                },
            );
        })
    });
}

/// The controls only need scalar parameters. Read them in the evaluator's scope/order without
/// cloning an entire effect or its animated params during a frame.
fn component_style(project: &Project, data: &ClipData, kind: &str) -> (f64, bool) {
    let (mut enabled, mut size, mut rings) = (None, None, None);
    for scope in [
        &project.effects,
        &project.spine.effects,
        &data.appearance.effects,
    ] {
        for effect in scope.iter().filter(|effect| effect.kind == kind) {
            enabled = effect.enabled.or(enabled).or(Some(true));
            for param in &effect.params {
                match param.name.as_str() {
                    "size" => {
                        size = match param.value {
                            EffectValue::Number(n) if n.is_finite() => Some(n),
                            _ => None,
                        };
                    }
                    "clickRings" => {
                        rings = match param.value {
                            EffectValue::Bool(on) => Some(on),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    match (kind, enabled) {
        ("cursor", Some(true)) => (size.unwrap_or(1.5), rings.unwrap_or(true)),
        ("cursor", _) | (_, Some(false)) => (1.0, false),
        _ => (size.unwrap_or(1.0), false),
    }
}

fn component_op(clip: ClipId, command: &str, field: &str, value: Json, part: Option<&str>) -> Json {
    let mut fields = vec![
        ("cmd".into(), Json::String(command.into())),
        ("clip".into(), Json::String(clip.to_string())),
        (field.into(), value),
    ];
    if let Some(part) = part {
        fields.push(("component".into(), Json::String(part.into())));
    }
    Json::Object(fields)
}

fn keys_visibility(clip: ClipId, indexes: &[u32], hidden: bool) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("keys.hide".into())),
        ("clip".into(), Json::String(clip.to_string())),
        (
            "events".into(),
            Json::Array(
                indexes
                    .iter()
                    .map(|&index| Json::Unsigned(u64::from(index)))
                    .collect(),
            ),
        ),
        ("hidden".into(), Json::Bool(hidden)),
    ])
}

fn level_control(
    ui: &mut Ui,
    label: &'static str,
    (value, min, max, decimals): (f64, f64, f64, usize),
    (width, mut focus): (f32, Option<&mut Option<Handle>>),
) -> Option<f64> {
    let custom_speed = label == "Custom";
    let scale = if custom_speed {
        20.0
    } else if decimals == 0 {
        1.0
    } else {
        10.0
    };
    let rounded = |v: f64| (v * scale).round() / scale;
    let shown = if custom_speed {
        value
    } else {
        rounded(value.clamp(min, max))
    };
    let step = (1.0 / scale) as f32;
    let field = property_field(width);
    let field_w = field.w.min(52.0);
    let mut change = None;
    let row = match label {
        "Volume" => "Volume (%)",
        "Custom" => "Custom (×)",
        _ => label,
    };
    property_row(ui, row, &mut |ui| {
        let slider = stepped_slider(
            ui,
            label,
            shown.clamp(min, max) as f32,
            (min as f32, max as f32, step),
            (field.w - field_w - 6.0).max(0.0),
        );
        change = slider.commit.map(|v| rounded(f64::from(v)));
        use std::io::Write;
        let mut bytes = [0; 24];
        let mut buffer = bytes.as_mut_slice();
        if custom_speed && slider.preview.is_none() {
            // Preserve the exact Core speed, including a value authored between slider steps.
            if (min..=max).contains(&shown) {
                write!(buffer, "{shown}")
            } else {
                write!(buffer, "{shown:e}")
            }
        } else {
            write!(buffer, "{:.*}", decimals, rounded(f64::from(slider.value)))
        }
        .expect("a compact numeric value fits in 24 bytes");
        let len = 24 - buffer.len();
        let text = std::str::from_utf8(&bytes[..len]).expect("decimal digits");
        let valid = |text: &str| {
            text.trim()
                .parse::<f64>()
                .is_ok_and(|v| (min..=max).contains(&v))
        };
        let name = match label {
            "Volume" => "Volume percent",
            "Size" => "Cursor size value",
            "Custom" => "Custom speed",
            _ => "Chip size value",
        };
        let field = Rect::new(field.x + field.w - field_w, field.y, field_w, field.h);
        let response = pxui::ext::value_field(
            ui,
            name,
            text,
            field,
            &FIELD.name(name).edit(focus.is_some()),
            &ValueCallbacks::DEFAULT.valid(&valid),
        );
        if let Some(value) = response.changed.and_then(|v| v.trim().parse().ok()) {
            change = Some(if custom_speed { value } else { rounded(value) });
        }
        let (signal, editing) = (response.response, response.editing);
        if editing && let Some(focus) = focus.as_deref_mut() {
            *focus = Some(signal.handle);
        }
        style_field(ui, signal, editing, text, "");
    });
    change
        .map(|value| value.clamp(min, max))
        .filter(|&value| value != shown)
}

fn recorded_keys(
    ui: &mut Ui,
    project: &Project,
    (id, clip, data, selected): (ClipId, ClipTiming, &ClipData, Option<u32>),
    events: Option<(&TimelineEvents, (SourceSpan, &[f64]))>,
    width: f32,
    intents: &mut Intents,
) {
    rule(ui);
    ui.column(
        Node::DEFAULT.padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0)),
        |ui| {
            ui.row(Node::DEFAULT.h_px(23.0).center(), |ui| {
                let heading = ui.label("Recorded keystrokes", CAPTION.grow().ellipsis());
                ink(ui, heading, QUIET);
                ui.disabled(data.hidden_keys.is_empty(), |ui| {
                    if segment(ui, "Show all", 66.0, false) {
                        intents.op = Some(keys_visibility(id, &data.hidden_keys, false));
                    }
                });
            });
            let Some((events, (span, map))) =
                events.filter(|(events, _)| events.fps.is_finite() && events.fps > 0.0)
            else {
                ui.label("No recorded keystrokes", CAPTION);
                return;
            };
            let start = span.source_at(f64::from(clip.start), map);
            let end = span.source_at(f64::from(clip.end().unwrap_or(clip.start)), map);
            let from = events
                .keys
                .partition_point(|key| key.t * events.fps < start);
            let to = events
                .keys
                .partition_point(|key| key.t * events.fps < end)
                .max(from);
            // ponytail: PXUI caps one list at a million rows. Page if takes exceed that limit.
            let keys = &events.keys[from..to.min(from.saturating_add(1_000_000))];
            if keys.is_empty() {
                ui.label("No recorded keystrokes in this clip", CAPTION);
                return;
            }
            const ROW: f32 = 22.0;
            ui.column(
                Node::DEFAULT.h_px((keys.len() as f32 * ROW).min(132.0)),
                |ui| {
                    let list = ui.section_list(
                        "recorded-keys",
                        &SectionList::new(keys.len()).row_h(ROW).overlay_scrollbar(),
                        &|row| keys[row].index.map_or(-1 - (from + row) as i64, i64::from),
                        &mut |ui, part, _| {
                            let SectionPart::Row(row) = part else { return };
                            let key = &keys[row];
                            let picked = selected.is_some() && selected == key.index;
                            let mut line = Node::DEFAULT.h_px(ROW).center().gap(4.0);
                            if picked {
                                line = line.look(Look::NONE.fill(SELECTED));
                            }
                            ui.row(line, |ui| {
                                ui.column(
                                    Node::DEFAULT.w_px((width - 2.0 * SECTION_PAD - 66.0).max(0.0)),
                                    |ui| {
                                        key_visibility(ui, id, data, key, intents);
                                    },
                                );
                                let Some(frame) = span.seek_at(key.t * events.fps, map) else {
                                    return;
                                };
                                let fps = project.fps.max(1);
                                let frame = frame.max(0) as u32;
                                let seconds = frame / fps;
                                ui.label_digits(
                                    format_args!(
                                        "{:02}:{:02}:{:02}",
                                        seconds / 60,
                                        seconds % 60,
                                        frame % fps
                                    ),
                                    CAPTION,
                                );
                            });
                        },
                    );
                    let last = selected.map_or(-1, i64::from);
                    if ui.state(&list.handle, -1) != last {
                        if let Some(index) = selected
                            && let Some(row) = keys.iter().position(|key| key.index == Some(index))
                        {
                            ui.reveal(&list.handle, row);
                        }
                        ui.set_state(&list.handle, last);
                    }
                },
            );
        },
    );
}

fn key_visibility(
    ui: &mut Ui,
    id: ClipId,
    data: &ClipData,
    key: &TimelineKey,
    intents: &mut Intents,
) {
    if let Some(index) = key.index {
        let hidden = data.hidden_keys.binary_search(&index).is_ok();
        let toggle = Toggle::DEFAULT
            .row_h(22.0)
            .switch(22.0, 12.0)
            .label(BODY)
            .press_row(true);
        if ui
            .toggle("key-visible", &key.label, !hidden, &toggle)
            .changed
        {
            intents.op = Some(keys_visibility(id, &[index], !hidden));
        }
    } else {
        let label = ui.label(&key.label, BODY.grow().ellipsis());
        ink(ui, label, MUTED);
    }
}

fn selected_item(
    ui: &mut Ui,
    project: &Project,
    slot: Slot,
    kind: &str,
    width: f32,
    (caption, focus): (Option<(&captions::Cue, i32)>, Option<Focus>),
    intents: &mut Intents,
) {
    let (Some(id), Some(item), Some(data)) = (
        project.items.id(slot),
        project.items.timing(slot),
        project.items.data(slot),
    ) else {
        return;
    };
    heading(
        ui,
        format_args!("{kind} · {id}"),
        caption.map_or_else(
            || group_name(project, TimelineId::Item(id)).0,
            |(cue, _)| cue.title(),
        ),
    );
    let field = property_field(width);
    let target = TimelineId::Item(id);
    let parent = match item.anchor {
        Anchor::ClipSource { clip, .. } if project.clips.find(clip).is_some() => Some(clip),
        _ => None,
    };
    let lane = project.lanes.iter().find(|lane| lane.id == data.lane);
    let edit = focus == Some(Focus::Text(id));
    ui.id(id.serial(), |ui| {
        if let Some(caption) = caption {
            captions::cue_fields(ui, project, (id, lane), caption, (field, edit), intents);
        }
        if caption.is_none_or(|(cue, _)| !cue.uses_words()) {
            section(ui, "TIMING AND ANCHOR", &mut |ui| {
                match item.anchor {
                    Anchor::Pinned(at) => numeric(ui, "Anchor", format_args!("Pinned at {at} f")),
                    Anchor::ClipSource { clip, source } => numeric(
                        ui,
                        "Anchor",
                        format_args!(
                            "{} · source {source} f",
                            group_name(project, TimelineId::Clip(clip)).0,
                        ),
                    ),
                    Anchor::Item { item, edge, offset } => {
                        let edge = match edge {
                            splice_model::Edge::Start => "start",
                            splice_model::Edge::End => "end",
                        };
                        numeric(ui, "Anchor", format_args!("{item} {edge} {offset:+} f"));
                    }
                    Anchor::Marker { marker, offset } => {
                        numeric(ui, "Anchor", format_args!("{marker} {offset:+} f"))
                    }
                    Anchor::Word { word, .. } => numeric(ui, "Anchor", format_args!("Word {word}")),
                }
                let start = timing::anchor_start(project, item.anchor).ok();
                let word_derived = matches!(data.kind, ItemKind::Caption { text: None, .. });
                if !word_derived && start.is_some() {
                    let pinned = matches!(item.anchor, Anchor::Pinned(_));
                    let toggle = Toggle::DEFAULT
                        .row_h(26.0)
                        .label(BODY)
                        .disabled(pinned && project.clips.order().is_empty());
                    if ui
                        .toggle("pinned", "Pinned to timeline", pinned, &toggle)
                        .changed
                    {
                        intents.op = Some(item_setting(
                            id,
                            "item.anchor",
                            "pinned",
                            Json::Bool(!pinned),
                        ));
                    }
                }
                if let Some(lane) = lane
                    && matches!(
                        data.kind,
                        ItemKind::Title { .. }
                            | ItemKind::Media(_)
                            | ItemKind::Motion(_)
                            | ItemKind::Zoom(_)
                            | ItemKind::Caption { .. }
                    )
                {
                    lane_choice(ui, project, id, lane, field.w, intents);
                } else {
                    property(ui, "Lane", lane.map_or("", |lane| lane.name.as_str()));
                }
                if let Some(start) = start.filter(|_| !word_derived) {
                    if let Some(to) = frame_field(
                        ui,
                        "Start",
                        start,
                        0,
                        i32::MAX.saturating_sub(item.length),
                        field,
                    ) {
                        let mut op = group_command("item.move", &[TimelineId::Item(id)]);
                        op.push(("to".into(), Json::Signed(i64::from(to))));
                        intents.op = Some(Json::Object(op));
                    }
                    if let Some(length) = frame_field(
                        ui,
                        "Length",
                        item.length,
                        3,
                        i32::MAX.saturating_sub(start),
                        field,
                    ) && let Some(end) = start.checked_add(length)
                    {
                        intents.op = Some(trim_op(TimelineId::Item(id), "out", end));
                    }
                } else {
                    if let Some(start) = start {
                        numeric(ui, "Start", format_args!("{start} f"));
                    }
                    numeric(ui, "Length", format_args!("{} f", item.length));
                }
            });
        }
        match &data.kind {
            ItemKind::Title { text } => section(ui, "TEXT", &mut |ui| {
                text_field(
                    ui,
                    ("Text", "Title text"),
                    text,
                    "",
                    field,
                    (50, edit.then_some(&mut intents.focused_field)),
                    &mut |text| {
                        intents.op = Some(item_setting(
                            id,
                            "title.set",
                            "text",
                            Json::String(text.into()),
                        ));
                    },
                );
            }),
            ItemKind::Motion(motion) => section(ui, "KEYFRAMES", &mut |ui| {
                let [min, max] = splice_model::ScaleMotion::EDIT_RANGE;
                for (key, label, value) in
                    [("from", "◆ Start", motion.from), ("to", "◆ End", motion.to)]
                {
                    if let Some(scale) = decimal_field(ui, label, (value, min, max), field) {
                        intents.op = Some(item_setting(id, "motion.set", key, Json::Float(scale)));
                    }
                }
                property(ui, "Curve", "Linear");
            }),
            ItemKind::Zoom(zoom) => section(ui, "ZOOM", &mut |ui| {
                let [min, max] = Zoom::AMOUNT;
                if let Some(amount) = decimal_field(ui, "Amount", (zoom.amount, min, max), field) {
                    intents.op = Some(zoom_setting(id, "amount", Json::Float(amount)));
                }
                let springs = Spring::NAMES.map(|(_, name)| name);
                if let Some(spring) =
                    static_choice(ui, "Spring", zoom.spring.name(), field.w, &springs)
                {
                    intents.op = Some(zoom_setting(id, "spring", Json::String(spring.into())));
                }
            }),
            ItemKind::Media(overlay) => {
                let media = project.media.iter().find(|media| media.id == overlay.media);
                section(ui, "OVERLAY", &mut |ui| {
                    property(
                        ui,
                        "Media",
                        group_media_name(project, overlay.media).unwrap_or("Media unavailable"),
                    );
                    let placement = PLACEMENTS
                        .iter()
                        .find(|(placement, _)| *placement == overlay.placement)
                        .expect("placement wire name")
                        .1;
                    let placements = PLACEMENTS.map(|(_, name)| name);
                    if let Some(placement) =
                        static_choice(ui, "Placement", placement, field.w, &placements)
                    {
                        intents.op = Some(item_setting(
                            id,
                            "overlay.set",
                            "placement",
                            Json::String(placement.into()),
                        ));
                    }
                    let max_source = media.and_then(|media| {
                        // item.slip consumes this rounded length on the media clock.
                        let needed = (f64::from(item.length) * media.probe.fps
                            / f64::from(project.fps))
                        .round();
                        (0.0..=f64::from(media.probe.frames))
                            .contains(&needed)
                            .then(|| media.probe.frames - needed as i32)
                    });
                    if let Some(max) = max_source {
                        if let Some(source) =
                            frame_field(ui, "Source", overlay.source, 0, max, field)
                        {
                            let by = i64::from(source) - i64::from(overlay.source);
                            intents.op =
                                Some(item_setting(id, "item.slip", "by", Json::Signed(by)));
                        }
                    } else {
                        numeric(ui, "Source", format_args!("{} f", overlay.source));
                    }
                    let toggle = Toggle::DEFAULT.row_h(26.0).label(BODY);
                    for (key, label, on) in [
                        (
                            "behindPresenter",
                            "Behind presenter",
                            overlay.behind_presenter,
                        ),
                        ("muted", "Audio muted", overlay.muted),
                    ] {
                        if ui.toggle(key, label, on, &toggle).changed {
                            intents.op =
                                Some(item_setting(id, "overlay.set", key, Json::Bool(!on)));
                        }
                    }
                    ui.node("replace-spine-space", Node::DEFAULT.h_px(6.0));
                    if ui
                        .button(
                            "replace-spine",
                            "Replace Spine Clip",
                            &Button::sized(170.0, 24.0)
                                .label(BODY)
                                .look(&CHOICE_LOOK)
                                .disabled(parent.is_none() || media.is_none()),
                        )
                        .clicked
                    {
                        intents.op = Some(Json::Object(vec![
                            ("cmd".into(), Json::String("overlay.replaceSpine".into())),
                            ("item".into(), Json::String(id.to_string())),
                        ]));
                    }
                });
                if let Some(size) = media
                    .filter(|media| !matches!(media.kind, MediaKind::Audio | MediaKind::Compound))
                    .map(|media| [media.probe.width, media.probe.height])
                    .filter(|size| !size.contains(&0))
                {
                    framing(ui, target, overlay.framing, size, intents);
                }
            }
            _ => {}
        }
        let fade_limit = match data.kind {
            ItemKind::Caption { .. } => 30,
            ItemKind::Media(_) | ItemKind::Title { .. } => 90,
            _ => 0,
        };
        if fade_limit > 0 {
            item_fades(
                ui,
                target,
                &data.appearance,
                (item.length, fade_limit),
                field,
                intents,
            );
        }
        if !matches!(data.kind, ItemKind::Motion(_)) {
            effects::paint(ui, project, Target::Timeline(target), width, intents);
        }
        if matches!(
            data.kind,
            ItemKind::Media(_)
                | ItemKind::Title { .. }
                | ItemKind::Motion(_)
                | ItemKind::Zoom(_)
                | ItemKind::Caption { .. }
        ) {
            section(ui, "COLOR", &mut |ui| {
                let active = data.appearance.color.as_deref().unwrap_or("");
                color_swatches(ui, Some(active), &mut |color| {
                    intents.op = Some(selection_setting(
                        target,
                        "item.set",
                        "color",
                        color.map_or(Json::Null, |color| Json::String(color.into())),
                    ));
                });
            });
            if let Some(parent) = parent.filter(|_| caption.is_none()) {
                rule(ui);
                ui.row(
                    Node::row().padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0)),
                    |ui| {
                        if ui
                            .button(
                                "select-anchor",
                                "Select anchor clip",
                                &Button::sized(132.0, 24.0).label(BODY).look(&CHOICE_LOOK),
                            )
                            .clicked
                        {
                            intents.select_parent = Some(parent);
                        }
                    },
                );
            }
        }
    });
}

fn lane_choice(
    ui: &mut Ui,
    project: &Project,
    id: ItemId,
    current: &Lane,
    width: f32,
    intents: &mut Intents,
) {
    let lanes = || {
        project
            .lanes
            .iter()
            .filter(|lane| lane.kind == current.kind)
    };
    let mut chosen = None;
    choice_field(
        ui,
        "Lane",
        &current.name,
        width,
        lanes().count(),
        &mut |ui, opened| {
            let current_span = span(project, TimelineId::Item(id)).ok();
            for lane in lanes() {
                let selected = lane.id == current.id;
                let enabled = selected
                    || current_span.is_some_and(|span| !lane_occupied(project, lane.id, span));
                ui.id(lane.id.serial(), |ui| {
                    if choice_option(ui, &lane.name, selected, enabled, opened) {
                        chosen = Some(lane.id);
                    }
                });
            }
            chosen.is_some()
        },
    );
    if let Some(lane) = chosen.filter(|&lane| lane != current.id) {
        let mut op = group_command("item.lane", &[TimelineId::Item(id)]);
        op.push(("lane".into(), Json::String(lane.to_string())));
        intents.op = Some(Json::Object(op));
    }
}

fn lane_occupied(project: &Project, lane: LaneId, selected: Span) -> bool {
    project.items.order().iter().copied().any(|slot| {
        if project.items.data(slot).expect("live item").lane != lane {
            return false;
        }
        let id = project.items.id(slot).expect("live item");
        span(project, TimelineId::Item(id)).map_or(true, |other| {
            other.start < selected.end && selected.start < other.end
        })
    })
}

fn static_choice(
    ui: &mut Ui,
    label: &'static str,
    current: &'static str,
    width: f32,
    options: &[&'static str],
) -> Option<&'static str> {
    let mut chosen = None;
    choice_field(
        ui,
        label,
        current,
        width,
        options.len(),
        &mut |ui, opened| {
            for &option in options {
                ui.id(option, |ui| {
                    if choice_option(ui, option, option == current, true, opened) {
                        chosen = Some(option);
                    }
                });
            }
            chosen.is_some()
        },
    );
    chosen.filter(|&value| value != current)
}

fn choice_field(
    ui: &mut Ui,
    name: &'static str,
    value: &str,
    width: f32,
    count: usize,
    options: &mut dyn FnMut(&mut Ui, bool) -> bool,
) {
    property_row(ui, name, &mut |ui| {
        ui.id(name, |ui| {
            choice_input(ui, name, value, width, count, options)
        });
    });
}

fn choice_input(
    ui: &mut Ui,
    name: &str,
    value: &str,
    width: f32,
    count: usize,
    options: &mut dyn FnMut(&mut Ui, bool) -> bool,
) {
    let button = ui.button(
        "value",
        value,
        &Button::sized(width, 22.0)
            .look(&CHOICE_LOOK)
            .content_align(Along::Start)
            .label(BODY.grow().ellipsis())
            .accessory("\u{25be}")
            .accessory_label(CAPTION)
            .focus_on_click(true)
            .name(name)
            .disabled(count < 2),
    );
    let open = choice_popup(
        ui,
        button,
        (name, width, if count < 2 { 0 } else { count }),
        options,
    );
    let states = if count < 2 {
        A11yStates::NONE
    } else if open {
        A11yStates::EXPANDABLE.with(A11yStates::EXPANDED)
    } else {
        A11yStates::EXPANDABLE
    };
    ui.set_role(&button.handle, Role::ComboBox, states);
    let _ = ui.a11y_value(&button.handle, value, (0, 0));
}

fn choice_popup(
    ui: &mut Ui,
    button: Response,
    (name, width, count): (&str, f32, usize),
    options: &mut dyn FnMut(&mut Ui, bool) -> bool,
) -> bool {
    let handle = button.handle;
    let was_open = ui.state(&handle, 0) != 0;
    let open = count > 0 && (was_open != button.clicked);
    let mut still_open = open;
    if open {
        let rect = ui.rect(&handle);
        let content = ui.content_rect();
        let w = width.max(180.0);
        let h = (count.min(6) as f32 * 26.0 + 16.0).min(content.h.max(1.0));
        let x = (rect.x + rect.w - w).clamp(content.x, (content.x + content.w - w).max(content.x));
        let below = rect.y + rect.h;
        let y = if below + h <= content.y + content.h {
            below
        } else {
            (rect.y - h).max(content.y)
        };
        let mut picked = false;
        let popup = ui.popup(
            "choices",
            &Overlay::DEFAULT
                .at(Point::new(x, y))
                .w_px(w)
                .h_px(h)
                .max_h_px(h)
                .stroke(STRONG_LINE)
                .name(name),
            |ui| {
                ui.column(Node::column().w(Size::Grow), |ui| {
                    picked = options(ui, !was_open);
                });
            },
        );
        still_open = popup.open && !picked;
        if picked {
            ui.dismiss_popup();
        }
        if !still_open {
            ui.focus_box(&handle);
        }
    }
    if still_open != was_open {
        ui.set_state(&handle, i64::from(still_open));
    }
    still_open
}

fn choice_option(ui: &mut Ui, value: &str, selected: bool, enabled: bool, opened: bool) -> bool {
    let button = ui.button(
        "option",
        value,
        &Button::DEFAULT
            .size(Size::Grow, Size::Px(22.0))
            .look(&SEGMENT)
            .label(BODY.grow().ellipsis())
            .content_align(Along::Start)
            .disabled(!enabled)
            .selected(selected),
    );
    if selected && opened {
        ui.focus_box(&button.handle);
    }
    button.clicked
}

fn zoom_setting(id: ItemId, key: &str, value: Json) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("zoom.set".into())),
        ("zoom".into(), Json::String(id.to_string())),
        (key.into(), value),
    ])
}

fn item_setting(id: ItemId, command: &str, key: &str, value: Json) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String(command.into())),
        ("item".into(), Json::String(id.to_string())),
        (key.into(), value),
    ])
}

fn selected_lane(
    ui: &mut Ui,
    project: &Project,
    lane: &Lane,
    width: f32,
    caption: Option<&captions::LaneDetails>,
    intents: &mut Intents,
) {
    heading(ui, format_args!("Lane · {}", lane.id), &lane.name);
    section(ui, "LANE", &mut |ui| {
        ui.id(lane.id.serial(), |ui| {
            name_field(
                ui,
                "Lane name",
                &lane.name,
                "",
                property_field(width),
                30,
                &mut |value| {
                    let name = value.trim();
                    let name = if name.is_empty() { "Lane" } else { name };
                    if name != lane.name {
                        intents.op = Some(Json::Object(vec![
                            ("cmd".into(), Json::String("lane.rename".into())),
                            ("lane".into(), Json::String(lane.id.to_string())),
                            ("name".into(), Json::String(name.into())),
                        ]));
                    }
                },
            );
        });
        let kind = match lane.kind {
            LaneKind::Graphics => "graphics",
            LaneKind::Overlay => "overlay",
            LaneKind::Camera => "motion",
            LaneKind::Captions => "captions",
            LaneKind::Audio => "audio",
        };
        property(ui, "Kind", kind);
        if caption.is_none() {
            let count = project
                .items
                .order()
                .iter()
                .filter(|&&slot| {
                    project
                        .items
                        .data(slot)
                        .is_some_and(|item| item.lane == lane.id)
                })
                .count();
            numeric(ui, "Items", format_args!("{count}"));
        }
    });
    if let Some(caption) = caption {
        ui.id(lane.id.serial(), |ui| {
            captions::lane_fields(ui, project, lane, caption, intents);
        });
    }
    effects::paint(ui, project, Target::Lane(lane.id), width, intents);
}

fn selected_marker(ui: &mut Ui, marker: &Marker, width: f32, ripple: bool, intents: &mut Intents) {
    heading(ui, format_args!("Marker · {}", marker.id), &marker.name);
    section(ui, "MARKER", &mut |ui| {
        ui.id(marker.id.serial(), |ui| {
            let field = property_field(width);
            name_field(
                ui,
                "Marker name",
                &marker.name,
                "",
                field,
                40,
                &mut |name| {
                    intents.op = Some(marker_setting(marker.id, "name", Json::String(name.into())));
                },
            );
            if let Some(at) = frame_field(ui, "Frame", marker.at, 0, i32::MAX, field) {
                intents.op = Some(marker_setting(marker.id, "at", Json::Signed(i64::from(at))));
            }
        });
    });
    rule(ui);
    ui.row(
        Node::row()
            .padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0))
            .gap(8.0),
        |ui| {
            if ui
                .button("jump-marker", "Jump to marker", &Button::DEFAULT)
                .clicked
            {
                intents.seek = Some(marker.at);
            }
            if ui
                .button(
                    "delete-marker",
                    "Delete",
                    &Button::DEFAULT.name("Delete marker"),
                )
                .clicked
            {
                let mut op = group_command("item.delete", &[TimelineId::Marker(marker.id)]);
                op.push(("ripple".into(), Json::Bool(ripple)));
                intents.op = Some(Json::Object(op));
            }
        },
    );
}

fn marker_setting(id: MarkerId, key: &str, value: Json) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("marker.set".into())),
        ("marker".into(), Json::String(id.to_string())),
        (key.into(), value),
    ])
}

fn group_selection(
    ui: &mut Ui,
    project: &Project,
    group: &[TimelineId],
    ripple: bool,
    intents: &mut Intents,
) {
    let count = group.len();
    let shared = group_color(project, group[0]);
    let uniform = group
        .iter()
        .skip(1)
        .all(|&id| group_color(project, id) == shared);
    let all_clips = group.iter().all(|id| matches!(id, TimelineId::Clip(_)));
    ui.column(
        Node::DEFAULT.padding(Edges::new(12.0, 4.0, 12.0, 8.0)),
        |ui| {
            ui.label("Selection", CAPTION);
            ui.label_fmt(format_args!("{count} items"), Label::BODY.size(20.0));
        },
    );
    section(ui, "ITEMS", &mut |ui| {
        let rows = count.min(1_000_000);
        let height = (rows as f32 * 26.0).min(156.0);
        ui.column(Node::DEFAULT.h_px(height), |ui| {
            ui.section_list(
                "selected-items",
                &SectionList::new(rows).row_h(26.0).overlay_scrollbar(),
                &|index| index as i64,
                &mut |ui, part, _| {
                    let SectionPart::Row(index) = part else {
                        return;
                    };
                    let id = group[index];
                    let (name, tone) = group_name(project, id);
                    ui.row(Node::DEFAULT.h_px(26.0).center().gap(6.0), |ui| {
                        ui.node(
                            "item-kind",
                            Node::DEFAULT
                                .w_px(3.0)
                                .h_px(10.0)
                                .look(Look::NONE.fill(tone)),
                        );
                        ui.label(name, BODY.w(Size::Grow).ellipsis());
                        ui.label_fmt(format_args!("{id}"), CAPTION);
                    });
                },
            );
        });
    });
    if all_clips {
        section(ui, "SPEED", &mut |ui| {
            segment_group(ui, &mut |ui| {
                for (speed, label) in [
                    (0.5, "0.5×"),
                    (1.0, "1×"),
                    (1.5, "1.5×"),
                    (2.0, "2×"),
                    (4.0, "4×"),
                    (8.0, "8×"),
                ] {
                    if segment(ui, label, 37.0, false) {
                        let mut op = group_command("clip.speed", group);
                        op.push(("speed".into(), Json::Float(speed)));
                        op.push(("ripple".into(), Json::Bool(ripple)));
                        intents.op = Some(Json::Object(op));
                    }
                }
            });
        });
    }
    section(ui, "COLOR", &mut |ui| {
        let active = if uniform {
            Some(shared.unwrap_or(""))
        } else {
            None
        };
        color_swatches(ui, active, &mut |color| {
            let mut op = group_command("item.set", group);
            op.push((
                "color".into(),
                color.map_or(Json::Null, |color| Json::String(color.into())),
            ));
            intents.op = Some(Json::Object(op));
        });
    });
    rule(ui);
    ui.row(
        Node::DEFAULT
            .padding(Edges::new(12.0, 6.0, 12.0, 8.0))
            .gap(6.0),
        |ui| {
            if ui
                .button("duplicate-group", "Duplicate", &Button::DEFAULT)
                .clicked
            {
                intents.op = Some(Json::Object(group_command("item.duplicate", group)));
            }
            let button = Button::sized(98.0, 20.0).name("Delete selected items");
            if ui
                .button_content("delete-group", &button, |ui, _| {
                    ui.label_fmt(format_args!("Delete {count}"), BODY);
                })
                .clicked
            {
                let mut op = group_command("item.delete", group);
                op.push(("ripple".into(), Json::Bool(ripple)));
                intents.op = Some(Json::Object(op));
            }
        },
    );
}

fn group_color(project: &Project, id: TimelineId) -> Option<&str> {
    match id {
        TimelineId::Clip(id) => project
            .clips
            .data(project.clips.find(id)?)?
            .appearance
            .color
            .as_deref(),
        TimelineId::Item(id) => project
            .items
            .data(project.items.find(id)?)?
            .appearance
            .color
            .as_deref(),
        TimelineId::Marker(id) => project
            .markers
            .iter()
            .find(|marker| marker.id == id)?
            .color
            .as_deref(),
    }
}

fn group_name(project: &Project, id: TimelineId) -> (&str, Rgba) {
    match id {
        TimelineId::Clip(id) => {
            let clip = project
                .clips
                .find(id)
                .and_then(|slot| project.clips.data(slot));
            let name = clip
                .and_then(|clip| {
                    clip.appearance.name.as_deref().or_else(|| {
                        let media = clip
                            .media
                            .and_then(|id| project.media.iter().find(|media| media.id == id))?;
                        std::path::Path::new(&media.path).file_stem()?.to_str()
                    })
                })
                .unwrap_or(if clip.is_some_and(|clip| clip.media.is_none()) {
                    "Gap"
                } else {
                    "Clip"
                });
            (name, Rgba::rgb(236, 230, 206))
        }
        TimelineId::Item(id) => {
            let Some(data) = project
                .items
                .find(id)
                .and_then(|slot| project.items.data(slot))
            else {
                return ("Overlay", INK);
            };
            let (name, tone) = match &data.kind {
                ItemKind::Title { text } => (text.as_str(), Rgba::rgb(240, 214, 196)),
                ItemKind::Media(overlay) => (
                    group_media_name(project, overlay.media).unwrap_or("Overlay"),
                    Rgba::rgb(211, 228, 205),
                ),
                ItemKind::Motion(_) | ItemKind::Zoom(_) => {
                    ("Scale animation", Rgba::rgb(191, 224, 234))
                }
                ItemKind::Sound(audio) => (
                    group_media_name(project, audio.media).unwrap_or("Audio"),
                    Rgba::rgb(198, 214, 230),
                ),
                ItemKind::Shape { .. } => ("Shape", Rgba::rgb(240, 214, 196)),
                ItemKind::Caption { text, .. } => (
                    text.as_deref().unwrap_or("Caption"),
                    Rgba::rgb(225, 212, 236),
                ),
            };
            (data.appearance.name.as_deref().unwrap_or(name), tone)
        }
        TimelineId::Marker(id) => (
            project
                .markers
                .iter()
                .find(|marker| marker.id == id)
                .map_or("Marker", |marker| marker.name.as_str()),
            Rgba::rgb(216, 200, 106),
        ),
    }
}

fn group_media_name(project: &Project, id: MediaId) -> Option<&str> {
    let media = project.media.iter().find(|media| media.id == id)?;
    std::path::Path::new(&media.path).file_stem()?.to_str()
}
fn group_command(command: &str, group: &[TimelineId]) -> Vec<(String, Json)> {
    vec![
        ("cmd".into(), Json::String(command.into())),
        (
            "items".into(),
            Json::Array(
                group
                    .iter()
                    .map(|id| Json::String(id.to_string()))
                    .collect(),
            ),
        ),
    ]
}
fn clip_details(
    ui: &mut Ui,
    project: &Project,
    id: ClipId,
    data: &splice_model::ClipData,
    can_lift: bool,
    width: f32,
    intents: &mut Intents,
) {
    section(ui, "ANCHORED OVERLAYS", &mut |ui| {
        let mut found = false;
        for &slot in project.items.order() {
            let (Some(item), Some(timing)) = (project.items.data(slot), project.items.timing(slot))
            else {
                continue;
            };
            let source = match timing.anchor {
                Anchor::ClipSource { clip, source } if clip == id => Some(source),
                Anchor::Item {
                    item: TimelineId::Clip(clip),
                    ..
                } if clip == id => None,
                _ => continue,
            };
            found = true;
            let name = item
                .appearance
                .name
                .as_deref()
                .unwrap_or_else(|| match &item.kind {
                    ItemKind::Title { text } => text,
                    ItemKind::Media(_) => "Overlay",
                    ItemKind::Motion(_) | ItemKind::Zoom(_) => "Scale animation",
                    ItemKind::Sound(_) => "Audio",
                    ItemKind::Shape { .. } => "Shape",
                    ItemKind::Caption { text, .. } => text.as_deref().unwrap_or("Caption"),
                });
            ui.row(
                Node {
                    h: Size::Px(26.0),
                    align: Align::Center,
                    gap: 8.0,
                    ..Node::DEFAULT
                },
                |ui| {
                    ui.label(name, BODY.w_px(154.0).ellipsis());
                    if let Some(source) = source {
                        ui.label_fmt(format_args!("src {source}"), CAPTION);
                    } else {
                        ui.label("Clip edge", CAPTION);
                    }
                },
            );
        }
        if !found {
            ui.label(
                "None. Overlays above attach to a source frame in this clip.",
                CAPTION.w_px(244.0).wrap(),
            );
        }
    });
    effects::paint(
        ui,
        project,
        Target::Timeline(TimelineId::Clip(id)),
        width,
        intents,
    );
    section(ui, "COLOR", &mut |ui| {
        color_swatches(
            ui,
            Some(data.appearance.color.as_deref().unwrap_or("")),
            &mut |color| {
                intents.op = Some(clip_setting(
                    id,
                    "item.set",
                    "color",
                    color.map_or(Json::Null, |color| Json::String(color.into())),
                ));
            },
        );
    });
    if data.media.is_some() {
        section(ui, "STRUCTURE", &mut |ui| {
            let button = Button::DEFAULT.disabled(!can_lift);
            if ui
                .button("clip-lift", "Lift to Overlay ⌥⌘↑", &button)
                .clicked
            {
                intents.op = Some(Json::Object(vec![
                    ("cmd".into(), Json::String("clip.lift".into())),
                    (
                        "items".into(),
                        Json::Array(vec![Json::String(id.to_string())]),
                    ),
                ]));
            }
            if !can_lift {
                ui.label("Set the clip speed to 1× to lift it.", CAPTION);
            }
        });
    }
}

fn color_swatches(ui: &mut Ui, active: Option<&str>, pick: &mut dyn FnMut(Option<&'static str>)) {
    const COLORS: [(&str, &str, Rgba); 8] = [
        ("", "No color", Rgba::rgb(15, 15, 16)),
        ("red", "Red", Rgba::rgb(224, 121, 108)),
        ("orange", "Orange", Rgba::rgb(229, 154, 91)),
        ("yellow", "Yellow", Rgba::rgb(217, 194, 90)),
        ("green", "Green", Rgba::rgb(134, 191, 126)),
        ("blue", "Blue", Rgba::rgb(120, 169, 219)),
        ("purple", "Purple", Rgba::rgb(174, 146, 216)),
        ("gray", "Gray", Rgba::rgb(154, 154, 148)),
    ];
    ui.row(
        Node {
            gap: 3.0,
            ..Node::DEFAULT
        },
        |ui| {
            for (value, label, fill) in COLORS {
                let selected = active == Some(value);
                let base = Look::NONE.fill(fill).border(Rgba::rgb(56, 56, 59), 1.0);
                let look = States::of(base)
                    .hover(base.border(INK, 1.0))
                    .checked(base.border(INK, 2.0));
                let button = Button::sized(16.0, 16.0)
                    .name(label)
                    .look(&look)
                    .selected(selected);
                let response = ui.button(label, "", &button);
                if value.is_empty() {
                    ui.canvas(&response.handle, |paint, rect| {
                        paint.line(
                            (rect.x + 3.0, rect.y + rect.h - 3.0),
                            (rect.x + rect.w - 3.0, rect.y + 3.0),
                            Stroke::new(Rgba::rgb(227, 130, 118)),
                        );
                    });
                }
                if response.clicked && !selected {
                    pick((!value.is_empty()).then_some(value));
                }
            }
        },
    );
}
fn clip_name(
    ui: &mut Ui,
    id: ClipId,
    name: &str,
    placeholder: &str,
    rect: Rect,
    intents: &mut Intents,
) {
    name_field(ui, "Clip name", name, placeholder, rect, 40, &mut |name| {
        let value = if name.is_empty() {
            Json::Null
        } else {
            Json::String(name.into())
        };
        intents.op = Some(clip_setting(id, "item.set", "name", value));
    });
}

fn name_field(
    ui: &mut Ui,
    label: &'static str,
    name: &str,
    placeholder: &str,
    rect: Rect,
    max_length: usize,
    commit: &mut dyn FnMut(&str),
) {
    text_field(
        ui,
        ("Name", label),
        name,
        placeholder,
        rect,
        (max_length, None),
        commit,
    );
}

fn text_field(
    ui: &mut Ui,
    (row, label): (&str, &'static str),
    name: &str,
    placeholder: &str,
    rect: Rect,
    (max_length, mut focus): (usize, Option<&mut Option<Handle>>),
    commit: &mut dyn FnMut(&str),
) {
    property_row(ui, row, &mut |ui| {
        let valid =
            |text: &str| text.chars().count() <= max_length && !text.chars().any(char::is_control);
        let response = pxui::ext::value_field(
            ui,
            label,
            name,
            rect,
            &FIELD.name(label).edit(focus.is_some()),
            &ValueCallbacks::DEFAULT.valid(&valid),
        );
        if let Some(value) = response.changed.filter(|&value| value != name) {
            commit(value);
        }
        let (response, editing) = (response.response, response.editing);
        if editing && let Some(focus) = focus.as_deref_mut() {
            *focus = Some(response.handle);
        }
        style_field(ui, response, editing, name, placeholder);
    });
}

fn frame_field(
    ui: &mut Ui,
    label: &'static str,
    frame: i32,
    min: i32,
    max: i32,
    rect: Rect,
) -> Option<i32> {
    let mut changed = None;
    property_row(ui, label, &mut |ui| {
        changed = frame_value(ui, label, frame, (min, max), rect);
    });
    changed
}

fn frame_value(
    ui: &mut Ui,
    label: &'static str,
    frame: i32,
    (min, max): (i32, i32),
    rect: Rect,
) -> Option<i32> {
    use std::io::Write;
    let mut bytes = [0; 11];
    let mut buffer = bytes.as_mut_slice();
    write!(buffer, "{frame}").expect("an i32 fits 11 bytes");
    let len = 11 - buffer.len();
    let text = std::str::from_utf8(&bytes[..len]).expect("signed decimal frame");
    let valid = |text: &str| {
        text.parse::<i32>()
            .is_ok_and(|frame| (min..=max).contains(&frame))
    };
    let response = pxui::ext::value_field(
        ui,
        label,
        text,
        rect,
        &FIELD.name(label),
        &ValueCallbacks::DEFAULT.valid(&valid),
    );
    let changed = response
        .changed
        .and_then(|value| value.parse().ok())
        .filter(|&value| value != frame);
    let (signal, editing) = (response.response, response.editing);
    style_field(ui, signal, editing, text, "");
    changed
}

fn decimal_field(
    ui: &mut Ui,
    label: &'static str,
    (value, min, max): (f64, f64, f64),
    rect: Rect,
) -> Option<f64> {
    use std::io::Write;
    let mut bytes = [0; 24];
    let mut buffer = bytes.as_mut_slice();
    if write!(buffer, "{value}").is_err() {
        buffer = bytes.as_mut_slice();
        write!(buffer, "{value:e}").expect("a scientific f64 fits in 24 bytes");
    }
    let len = 24 - buffer.len();
    let text = std::str::from_utf8(&bytes[..len]).expect("decimal digits");
    let parse = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| (min..=max).contains(v))
    };
    let mut changed = None;
    property_row(ui, label, &mut |ui| {
        let valid = |text: &str| parse(text).is_some();
        let response = pxui::ext::value_field(
            ui,
            label,
            text,
            rect,
            &FIELD.name(label),
            &ValueCallbacks::DEFAULT.valid(&valid),
        );
        changed = response.changed.and_then(parse).filter(|&v| v != value);
        let (signal, editing) = (response.response, response.editing);
        style_field(ui, signal, editing, text, "");
    });
    changed
}

fn property_field(width: f32) -> Rect {
    let offset = PROPERTY_LABEL + PROPERTY_GAP;
    Rect::new(
        offset,
        2.0,
        (width - 2.0 * SECTION_PAD - offset).max(0.0),
        22.0,
    )
}

fn style_field(ui: &mut Ui, response: Response, editing: bool, value: &str, placeholder: &str) {
    let handle = response.handle;
    ui.set_role(&handle, Role::TextField, A11yStates::EDITABLE);
    if !editing {
        let border = if response.focused { MUTED } else { LINE };
        let mut look = Look::NONE.fill(INPUT).border(border, 1.0).text(INK);
        let mut text = value;
        if value.is_empty() && !placeholder.is_empty() {
            // PXUI copies a ValueField placeholder on every build.
            text = placeholder;
            let _ = ui.text_line_box(&handle, text, BODY.line_height(22.0).ellipsis(), &[]);
            look = look.text(Rgba::gray(117));
        }
        ui.node_mut(&handle, &mut |fields| {
            fields.look = look;
            fields.inset = Edges::xy(8.0, 0.0);
        });
        let _ = ui.a11y_value(&handle, text, (0, 0));
    }
}

fn trim_op(id: TimelineId, edge: &str, at: i32) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("item.trim".into())),
        ("item".into(), Json::String(id.to_string())),
        ("edge".into(), Json::String(edge.into())),
        ("to".into(), Json::Signed(i64::from(at))),
    ])
}

fn speed_op(id: ClipId, speed: f64, ripple: bool) -> Json {
    let mut fields = group_command("clip.speed", &[TimelineId::Clip(id)]);
    fields.extend([
        ("speed".into(), Json::Float(speed)),
        ("ripple".into(), Json::Bool(ripple)),
    ]);
    Json::Object(fields)
}
fn framing(ui: &mut Ui, id: TimelineId, framing: Framing, size: [u32; 2], intents: &mut Intents) {
    section(ui, "FRAMING", &mut |ui| {
        if let Some(crop) = framing.crop {
            let [w, h] = size.map(f64::from);
            numeric(
                ui,
                "Crop",
                format_args!(
                    "{:.0}×{:.0} at {:.0}, {:.0}",
                    crop.w * w,
                    crop.h * h,
                    crop.x * w,
                    crop.y * h
                ),
            );
        } else {
            let [w, h] = size;
            numeric(ui, "Crop", format_args!("Full frame · {w}×{h} px"));
        }
        property_row(ui, "In canvas", &mut |ui| {
            segment_group(ui, &mut |ui| {
                for (fit, label, setting) in [(Fit::Fit, "Fit", "fit"), (Fit::Fill, "Fill", "fill")]
                {
                    if segment(ui, label, 42.0, framing.fit == fit) && framing.fit != fit {
                        intents.op = Some(selection_setting(
                            id,
                            "clip.crop",
                            "fit",
                            Json::String(setting.into()),
                        ));
                    }
                }
            });
        });
        let mut reset = false;
        if matches!(id, TimelineId::Item(_)) {
            ui.node("crop-actions-space", Node::DEFAULT.h_px(6.0));
            ui.row(
                Node::row().w(Size::Grow).gap(8.0).row_gap(4.0).wrap(),
                |ui| {
                    if ui
                        .button(
                            "edit-crop",
                            "Crop… ⇧C",
                            &Button::sized(85.0, 24.0)
                                .name("Crop overlay")
                                .label(BODY)
                                .look(&CHOICE_LOOK),
                        )
                        .clicked
                    {
                        intents.crop = Some(id);
                    }
                    reset = ui
                        .button(
                            "reset-crop",
                            "Reset crop",
                            &Button::sized(92.0, 24.0)
                                .label(BODY)
                                .look(&CHOICE_LOOK)
                                .disabled(framing.crop.is_none()),
                        )
                        .clicked;
                },
            );
        } else {
            reset = ui
                .button(
                    "reset-crop",
                    "Reset crop",
                    &Button::DEFAULT.disabled(framing.crop.is_none()),
                )
                .clicked;
        }
        if reset {
            intents.op = Some(selection_setting(id, "clip.crop", "rect", Json::Null));
        }
    });
}
fn sequence(ui: &mut Ui, project: &Project, end: i32) {
    let seconds = end.max(0) as f64 / f64::from(project.fps.max(1));
    let whole = end.max(0) as u32 / project.fps.max(1);
    let frame = end.max(0) as u32 % project.fps.max(1);
    heading(ui, format_args!("Nothing selected"), "Sequence");
    section(ui, "SUMMARY", &mut |ui| {
        numeric(
            ui,
            "Duration",
            format_args!(
                "{seconds:.2} s · {:02}:{:02}:{frame:02}",
                whole / 60,
                whole % 60
            ),
        );
        numeric(
            ui,
            "Spine",
            format_args!("{} clips", project.clips.order().len()),
        );
        numeric(
            ui,
            "Overlays",
            format_args!(
                "{} on {} lanes",
                project.items.order().len(),
                project.lanes.len()
            ),
        );
        numeric(ui, "Markers", format_args!("{}", project.markers.len()));
        ui.label(
            "Click a clip, overlay, chip or marker to edit it. Right-click anything for its commands. ⌘K opens every command; ? lists shortcuts.",
            CAPTION.w_px(244.0).wrap(),
        );
    });
}

fn canvas(ui: &mut Ui, project: &Project, end: i32, width: f32, intents: &mut Intents) {
    heading(ui, format_args!("Whole project"), "Canvas");
    section(ui, "OUTPUT SHAPE", &mut |ui| {
        let card_width = ((width - 26.0 - 4.0 * 4.0) / 5.0).max(0.0);
        const SHAPES: [(Aspect, &str, f32, f32); 5] = [
            (Aspect::Portrait, "9:16", 11.0, 19.0),
            (Aspect::Social, "4:5", 14.0, 17.0),
            (Aspect::Square, "1:1", 17.0, 17.0),
            (Aspect::Standard, "4:3", 20.0, 15.0),
            (Aspect::Landscape, "16:9", 24.0, 14.0),
        ];
        const SHAPE_BASE: Look = CONTROL.fill(Rgba::rgb(20, 20, 21)).border(LINE, 1.0);
        const SHAPE_LOOK: States = States::of(SHAPE_BASE)
            .hover(SHAPE_BASE.fill(HOVER).text(INK))
            .checked(SHAPE_BASE.fill(SELECTED).text(INK).border(INK, 1.0));

        ui.row(
            Node {
                gap: 4.0,
                margin: Edges::new(0.0, 0.0, 0.0, 4.0),
                ..Node::DEFAULT
            },
            |ui| {
                for (aspect, label, width, height) in SHAPES {
                    let selected = project.aspect == aspect;
                    let button = Button::sized(card_width, 56.0)
                        .name(label)
                        .axis(Axis::Column)
                        .look(&SHAPE_LOOK)
                        .selected(selected);
                    let response = ui.button_content(label, &button, |ui, _| {
                        ui.column(
                            Node {
                                h: Size::Px(26.0),
                                align: Align::Center,
                                ..Node::DEFAULT
                            },
                            |ui| {
                                ui.node(
                                    "shape",
                                    Node {
                                        w: Size::Px(width),
                                        h: Size::Px(height),
                                        look: Look::NONE
                                            .border(if selected { INK } else { QUIET }, 1.5),
                                        ..Node::DEFAULT
                                    },
                                );
                            },
                        );
                        ui.label(label, BODY);
                    });
                    if response.clicked && !selected {
                        intents.op = Some(project_setting("aspect", label));
                    }
                }
            },
        );
    });
    section(ui, "AGENT ACCESS", &mut |ui| {
        segment_group(ui, &mut |ui| {
            for (access, label) in [
                (Access::Off, "off"),
                (Access::Read, "read"),
                (Access::Live, "live"),
            ] {
                if segment(ui, label, 45.0, project.access == access) && project.access != access {
                    intents.op = Some(project_setting("access", label));
                }
            }
        });
        ui.label(
            "live: agent batches commit directly as undo steps. read: perception only. off: MCP refuses this project.",
            CAPTION.w_px(244.0).wrap(),
        );
    });
    section(ui, "SEQUENCE", &mut |ui| {
        numeric(
            ui,
            "Frame rate",
            format_args!("{} fps · integer frames", project.fps),
        );
        numeric(
            ui,
            "Duration",
            format_args!(
                "{end} f · {:.2} s",
                f64::from(end) / f64::from(project.fps.max(1))
            ),
        );
        numeric(
            ui,
            "Lanes",
            format_args!("{} overlay lanes + spine", project.lanes.len()),
        );
    });
    effects::paint(ui, project, Target::Project, width, intents);
}

fn heading(ui: &mut Ui, scope: std::fmt::Arguments<'_>, title: &str) {
    ui.column(
        Node::column().padding(Edges::new(SECTION_PAD, 4.0, SECTION_PAD, 8.0)),
        |ui| {
            let scope = ui.label_fmt(scope, CAPTION.grow().ellipsis());
            ink(ui, scope, QUIET);
            let title = ui.label(
                title,
                super::EDITOR_HEADING
                    .size(20.0)
                    .line_height(23.0)
                    .grow()
                    .wrap_glyph()
                    .role(Role::Heading),
            );
            ink(ui, title, INK);
        },
    );
}

fn section(ui: &mut Ui, title: &str, body: &mut dyn FnMut(&mut Ui)) {
    rule(ui);
    ui.column(
        Node::column().padding(Edges::new(SECTION_PAD, 6.0, SECTION_PAD, 8.0)),
        |ui| {
            let caption = ui.label(title, CAPTION.h_px(20.0).tracking(0.06));
            ink(ui, caption, QUIET);
            body(ui);
        },
    );
}

fn ink(ui: &mut Ui, handle: Handle, color: Rgba) {
    ui.style(
        &handle,
        &States::of(Look::NONE.text(color)),
        State::default(),
    );
}

fn rule(ui: &mut Ui) {
    ui.row(
        Node {
            h: Size::Px(1.0),
            w: Size::Grow,
            look: Look::NONE.fill(LINE),
            ..Node::DEFAULT
        },
        |_| {},
    );
}

fn property(ui: &mut Ui, name: &str, value: &str) {
    property_row(ui, name, &mut |ui| {
        let value = ui.label(value, CAPTION.grow().ellipsis());
        ink(ui, value, QUIET);
    });
}

fn numeric(ui: &mut Ui, name: &str, value: std::fmt::Arguments<'_>) {
    property_row(ui, name, &mut |ui| {
        let value = ui.label_fmt(value, CAPTION.grow().ellipsis());
        ink(ui, value, QUIET);
    });
}

fn stepped_slider(
    ui: &mut Ui,
    label: &'static str,
    value: f32,
    range: (f32, f32, f32),
    width: f32,
) -> pxui::SliderResponse {
    slider_field(ui, (label, label), value, range, width)
}

fn slider_field(
    ui: &mut Ui,
    (key, label): (&'static str, &str),
    value: f32,
    (min, max, step): (f32, f32, f32),
    width: f32,
) -> pxui::SliderResponse {
    let slider = ui.scrub(
        key,
        value,
        min..=max,
        &Scrubber::DEFAULT.step(step).name(label),
    );
    let handle = slider.response.handle;
    ui.set_node_layout(&handle, &Node::DEFAULT.w_px(width).h_px(22.0));
    let fraction = if max > min {
        (slider.value - min) / (max - min)
    } else {
        0.0
    };
    let focused = slider.response.focused;
    ui.canvas(&handle, move |paint, rect| {
        if focused {
            paint.stroke(rect, Stroke::new(INK));
        }
        let y = rect.y + 0.5 * rect.h;
        paint.fill(
            Rect::new(rect.x, y - 1.0, rect.w, 2.0),
            Fill::new(STRONG_LINE),
        );
        let thumb = rect.w.min(7.0);
        let x = rect.x + fraction.clamp(0.0, 1.0) * (rect.w - thumb);
        paint.fill(Rect::new(x, y - 6.0, thumb, 12.0), Fill::new(INK));
    });
    slider
}

fn item_fades(
    ui: &mut Ui,
    id: TimelineId,
    appearance: &Appearance,
    (length, limit): (i32, i32),
    field: Rect,
    intents: &mut Intents,
) {
    section(ui, "FADES", &mut |ui| {
        let box_width = field.w.min(52.0);
        let numeric = Rect::new(field.x + field.w - box_width, field.y, box_width, field.h);
        for (label, input, key, value, other) in [
            (
                "Fade in",
                "Fade in frames",
                "fadeIn",
                appearance.fade_in,
                appearance.fade_out,
            ),
            (
                "Fade out",
                "Fade out frames",
                "fadeOut",
                appearance.fade_out,
                appearance.fade_in,
            ),
        ] {
            let max = length.saturating_sub(other).clamp(0, value.max(limit));
            property_row(ui, label, &mut |ui| {
                let slider = stepped_slider(
                    ui,
                    label,
                    value as f32,
                    (0.0, max as f32, 1.0),
                    (field.w - box_width - 6.0).max(0.0),
                );
                let rounded = |frames: f32| (frames.round() as i32).clamp(0, max);
                let shown = slider.preview.map_or(value, rounded);
                let typed = frame_value(ui, input, shown, (0, max), numeric);
                if let Some(frames) = slider
                    .commit
                    .map(rounded)
                    .or(typed)
                    .filter(|&frames| frames != value)
                {
                    intents.op = Some(selection_setting(
                        id,
                        "item.set",
                        key,
                        Json::Signed(i64::from(frames)),
                    ));
                }
            });
        }
    });
}

fn property_row(ui: &mut Ui, name: &str, value: &mut dyn FnMut(&mut Ui)) {
    property_row_at(ui, name, PROPERTY_LABEL, value);
}

fn property_row_at(ui: &mut Ui, name: &str, label_width: f32, value: &mut dyn FnMut(&mut Ui)) {
    ui.row(
        Node {
            h: Size::Px(26.0),
            gap: PROPERTY_GAP,
            align: Align::Center,
            ..Node::DEFAULT
        },
        |ui| {
            let label = ui.label(name, BODY.w_px(label_width).ellipsis());
            ink(ui, label, MUTED);
            value(ui);
        },
    );
}

fn segment_group(ui: &mut Ui, body: &mut dyn FnMut(&mut Ui)) {
    ui.row(
        Node {
            look: Look::NONE.border(LINE, 1.0),
            padding: Edges::all(1.0),
            ..Node::DEFAULT
        },
        body,
    );
}

fn segment(ui: &mut Ui, name: &'static str, width: f32, selected: bool) -> bool {
    let button = Button::sized(width, 20.0)
        .look(&SEGMENT)
        .selected(selected)
        .label(BODY);
    ui.button(name, name, &button).clicked
}

fn clip_setting(id: ClipId, command: &str, key: &str, value: Json) -> Json {
    selection_setting(TimelineId::Clip(id), command, key, value)
}

fn selection_setting(id: TimelineId, command: &str, key: &str, value: Json) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String(command.into())),
        (
            "items".into(),
            Json::Array(vec![Json::String(id.to_string())]),
        ),
        (key.into(), value),
    ])
}
fn project_setting(key: &str, value: &str) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("project.set".into())),
        (key.into(), Json::String(value.into())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{FrameIn, testing};

    fn inspector_ui() -> Ui {
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        ui
    }

    fn component_fixture() -> (Project, ClipId, Slot) {
        use splice_model::{Clip, ComponentStates, Media, Probe, TakeComponents};
        let mut project = Project::default();
        let media = MediaId::from_serial(1);
        let clip = ClipId::from_serial(1);
        project.media.push(Media {
            id: media,
            kind: MediaKind::Take,
            path: "assets/Screen take".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 60.0,
                frames: 2000,
                width: 1920,
                height: 1080,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: Some(TakeComponents {
                screen: Some("screen.mov".into()),
                camera: Some("camera.mov".into()),
                mic: Some("mic.m4a".into()),
                system: Some("system.m4a".into()),
                events: Some("events.json".into()),
                ..TakeComponents::default()
            }),
        });
        project
            .clips
            .insert(Clip {
                id: clip,
                timing: ClipTiming {
                    start: 12,
                    source: 60,
                    length: 150,
                    speed: 2.0,
                },
                data: ClipData {
                    media: Some(media),
                    enabled: ComponentStates {
                        screen: Some(true),
                        camera: Some(true),
                        mic: Some(true),
                        system: Some(true),
                        pointer: Some(true),
                        keys: Some(false),
                    },
                    hidden_keys: vec![44],
                    ..ClipData::default()
                },
            })
            .unwrap();
        let slot = project.clips.find(clip).unwrap();
        (project, clip, slot)
    }

    #[test]
    fn component_controls_dispatch_decodable_edits() {
        use splice_core::{commands, edit, package::journal::Source};
        use std::cell::{Cell, RefCell};
        let (mut project, clip, slot) = component_fixture();
        let mutate = |project: &mut Project, op: Json| {
            commands::decode(&op).unwrap();
            edit::apply(project, &[op], Source::Agent, false).unwrap();
        };
        mutate(&mut project, splice_json::parse(
            r#"{"cmd":"effect.set","target":"project","type":"cursor","params":{"size":2.5,"clickRings":true}}"#,
        ).unwrap());
        let evaluator = splice_av::Evaluator::prepare(&project).unwrap();
        let clock = evaluator.clip_clock(clip, 12).unwrap();
        let project = RefCell::new(project);
        let events = TimelineEvents {
            fps: 60.0,
            keys: vec![
                TimelineKey {
                    t: 0.5,
                    index: Some(13),
                    category: 1,
                    label: "Trimmed earlier".into(),
                },
                TimelineKey {
                    t: 1.25,
                    index: Some(44),
                    category: 1,
                    label: "Redacted key".into(),
                },
                TimelineKey {
                    t: 11.0,
                    index: Some(51),
                    category: 1,
                    label: "Trimmed later".into(),
                },
            ],
            ..TimelineEvents::default()
        };
        let component = Cell::new(Component::Screen);
        let pending = RefCell::new(None);
        let parent = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let intents = paint(
                ui,
                Some(&project.borrow()),
                Selection::Component(clip, component.get(), Some(44)),
                Scope::Selection,
                162,
                (268.0, false),
                (&[], true, Some((&events, clock)), None, None),
            );
            if let Some(op) = intents.op {
                pending.replace(Some(op));
            }
            if let Some(parent_clip) = intents.select_parent {
                parent.set(Some(parent_clip));
            }
        };
        let mut ui = inspector_ui();
        let frame = |ui: &mut Ui, events: &[pxui::Event], build: &mut dyn FnMut(&mut Ui)| {
            ui.frame(&FrameIn::new(268.0, 390.0).events(events), build)
                .unwrap();
        };
        let click = |ui: &mut Ui, role: Role, label: &str, build: &mut dyn FnMut(&mut Ui)| {
            let rect =
                testing::find(ui, role, label).unwrap_or_else(|| panic!("{label} not found"));
            let x = if role == Role::Toggle {
                rect.x + rect.w - 20.0
            } else {
                rect.x + rect.w / 2.0
            };
            for kind in [pxui::Event::POINTER_PRESSED, pxui::Event::POINTER_RELEASED] {
                frame(
                    ui,
                    &[pxui::Event {
                        kind,
                        x: f64::from(x),
                        y: f64::from(rect.y + rect.h / 2.0),
                        ..pxui::Event::default()
                    }],
                    build,
                );
            }
        };
        for (part, visibility, field, text) in [
            (Component::Screen, true, "", ""),
            (Component::Camera, true, "", ""),
            (Component::Mic, false, "Volume percent", "125"),
            (Component::System, false, "Volume percent", "50"),
            (Component::Pointer, false, "Cursor size value", "3.1"),
            (Component::Keys, false, "Chip size value", "1.8"),
        ] {
            component.set(part);
            for _ in 0..2 {
                frame(&mut ui, &[], &mut draw);
            }
            if visibility {
                click(&mut ui, Role::Toggle, "Visible on this clip", &mut draw);
            } else {
                click(&mut ui, Role::TextField, field, &mut draw);
                frame(
                    &mut ui,
                    &[pxui::Event {
                        kind: pxui::Event::TEXT_INPUT,
                        text: pxui::Str {
                            ptr: text.as_ptr(),
                            len: text.len(),
                        },
                        ..pxui::Event::default()
                    }],
                    &mut draw,
                );
                assert!(pending.borrow().is_none());
                testing::key(&mut ui, pxui::KeyCode::Enter, &mut draw);
            }
            mutate(&mut project.borrow_mut(), pending.take().unwrap());
            if part == Component::Pointer {
                assert_eq!(
                    component_style(
                        &project.borrow(),
                        project.borrow().clips.data(slot).unwrap(),
                        "cursor"
                    ),
                    (3.1, true)
                );
                click(&mut ui, Role::Toggle, "Click rings", &mut draw);
                mutate(&mut project.borrow_mut(), pending.take().unwrap());
            }
        }
        assert!(testing::find(&ui, Role::Toggle, "Trimmed earlier").is_none());
        assert!(testing::find(&ui, Role::Toggle, "Trimmed later").is_none());
        click(&mut ui, Role::Toggle, "Redacted key", &mut draw);
        mutate(&mut project.borrow_mut(), pending.take().unwrap());
        assert!(
            project
                .borrow()
                .clips
                .data(slot)
                .unwrap()
                .hidden_keys
                .is_empty()
        );
        click(&mut ui, Role::Toggle, "Redacted key", &mut draw);
        mutate(&mut project.borrow_mut(), pending.take().unwrap());
        frame(&mut ui, &[], &mut draw);
        click(&mut ui, Role::Button, "Show all", &mut draw);
        mutate(&mut project.borrow_mut(), pending.take().unwrap());
        click(&mut ui, Role::Button, "Select parent clip", &mut draw);
        assert_eq!(parent.get(), Some(clip));

        let project = project.borrow();
        let data = project.clips.data(slot).unwrap();
        assert_eq!(
            (data.enabled.screen, data.enabled.camera, data.enabled.keys),
            (Some(false), Some(false), Some(false))
        );
        assert_eq!((data.volume.mic, data.volume.system), (1.25, 0.5));
        assert_eq!(component_style(&project, data, "cursor"), (3.1, false));
        assert_eq!(component_style(&project, data, "keystrokes").0, 1.8);
        assert!(data.hidden_keys.is_empty());
    }

    #[test]
    fn component_inspector_virtualizes_keys_without_warm_allocations() {
        mem_sys::set_peer(pxui::library_counter);
        use splice_model::{EffectId, EffectParam, keys::Key};
        let (mut project, clip, slot) = component_fixture();
        project
            .clips
            .data_mut(slot)
            .unwrap()
            .appearance
            .effects
            .push(splice_model::Effect {
                id: EffectId::from_serial(1),
                kind: "speed".into(),
                enabled: Some(true),
                params: vec![EffectParam {
                    name: "rate".into(),
                    value: EffectValue::Keys(vec![
                        Key {
                            at: 0,
                            value: 0.5,
                            ease: Default::default(),
                        },
                        Key {
                            at: 150,
                            value: 1.5,
                            ease: Default::default(),
                        },
                    ]),
                }],
            });
        let evaluator = splice_av::Evaluator::prepare(&project).unwrap();
        let clock = evaluator.clip_clock(clip, 12).unwrap();
        let events = TimelineEvents {
            fps: 60.0,
            keys: (0..10_000)
                .map(|index| TimelineKey {
                    t: 1.0 + f64::from(index) / 2000.0,
                    index: Some(index),
                    category: 1,
                    label: format!("Redacted {index}"),
                })
                .collect(),
            ..TimelineEvents::default()
        };
        let mut ui = inspector_ui();
        let frame = FrameIn::new(268.0, 390.0);
        let mut tick = || {
            for component in [Component::Mic, Component::Pointer, Component::Keys] {
                ui.frame(&frame, &mut |ui| {
                    paint(
                        ui,
                        Some(&project),
                        Selection::Component(clip, component, Some(9000)),
                        Scope::Selection,
                        162,
                        (268.0, false),
                        (&[], true, Some((&events, clock)), None, None),
                    );
                })
                .unwrap();
            }
        };
        for _ in 0..8 {
            tick();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut tick);
        assert!(testing::find(&ui, Role::Toggle, "Redacted 9000").is_some());
        assert!(testing::find(&ui, Role::Toggle, "Redacted 0").is_none());
        assert!(ui.a11y_nodes().len() < 90);
    }
    #[test]
    fn group_inspector_actions_use_all_selected_ids_and_magnetic_mode() {
        let mut project = Project::default();
        let ids = [ClipId::from_serial(1), ClipId::from_serial(2)];
        let group = ids.map(TimelineId::Clip);
        for (index, id) in ids.into_iter().enumerate() {
            project
                .clips
                .insert(splice_model::Clip {
                    id,
                    timing: splice_model::ClipTiming {
                        start: index as i32 * 20,
                        length: 20,
                        source: 0,
                        speed: 1.0,
                    },
                    data: splice_model::ClipData {
                        appearance: splice_model::Appearance {
                            name: Some(
                                if index == 0 {
                                    "First clip"
                                } else {
                                    "Second clip"
                                }
                                .into(),
                            ),
                            color: (index == 0).then(|| "red".into()),
                            ..splice_model::Appearance::default()
                        },
                        ..splice_model::ClipData::default()
                    },
                })
                .unwrap();
        }
        let intent = std::cell::Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            intent.set(
                paint(
                    ui,
                    Some(&project),
                    Selection::None,
                    Scope::Selection,
                    40,
                    (268.0, false),
                    (&group, false, None, None, None),
                )
                .op,
            );
        };
        let mut ui = inspector_ui();
        let input = pxui::FrameIn::new(320.0, 640.0);
        for _ in 0..2 {
            ui.frame(&input, &mut draw).unwrap();
        }
        for (label, command) in [
            ("2×", "clip.speed"),
            ("No color", "item.set"),
            ("Duplicate", "item.duplicate"),
            ("Delete selected items", "item.delete"),
        ] {
            let button = testing::find(&ui, Role::Button, label).unwrap();
            for kind in [pxui::Event::POINTER_PRESSED, pxui::Event::POINTER_RELEASED] {
                let events = [pxui::Event {
                    kind,
                    x: f64::from(button.x + 0.5 * button.w),
                    y: f64::from(button.y + 0.5 * button.h),
                    ..pxui::Event::default()
                }];
                ui.frame(&pxui::FrameIn::new(320.0, 640.0).events(&events), &mut draw)
                    .unwrap();
            }
            let op = intent.take().unwrap();
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            let items = op.field("items").and_then(Json::as_array).unwrap();
            assert_eq!(items, group.map(|id| Json::String(id.to_string())));
            if command == "clip.speed" || command == "item.delete" {
                assert_eq!(op.field("ripple").and_then(Json::as_bool), Some(false));
            }
            if command == "item.set" {
                assert_eq!(op.field("color"), Some(&Json::Null));
            }
        }
    }
    #[test]
    fn canvas_control_dispatches_project_set() {
        let project = Project::default();
        let intent = std::cell::Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            intent.set(
                paint(
                    ui,
                    Some(&project),
                    Selection::None,
                    Scope::Canvas,
                    0,
                    (268.0, false),
                    (&[], true, None, None, None),
                )
                .op,
            );
        };
        let mut ui = inspector_ui();
        for _ in 0..2 {
            testing::frame(&mut ui, &[], &mut draw);
        }
        let square = testing::find(&ui, Role::Button, "1:1").unwrap();
        for label in ["9:16", "4:5", "4:3", "16:9"] {
            assert_eq!(testing::find(&ui, Role::Button, label).unwrap().y, square.y);
        }
        testing::click(
            &mut ui,
            f64::from(square.x + square.w * 0.5),
            f64::from(square.y + square.h * 0.5),
            &mut draw,
        );
        let op = intent.take().unwrap();
        assert_eq!(op.field("cmd").and_then(Json::as_str), Some("project.set"));
        assert_eq!(op.field("aspect").and_then(Json::as_str), Some("1:1"));
        assert_eq!(project.aspect, Aspect::Portrait);
    }

    #[test]
    fn clip_color_and_lift_controls_use_the_existing_commands() {
        let clip = ClipId::from_serial(1);
        let project = Project::default();
        let data = splice_model::ClipData {
            media: Some(splice_model::MediaId::from_serial(2)),
            appearance: splice_model::Appearance {
                color: Some("red".into()),
                ..splice_model::Appearance::default()
            },
            ..splice_model::ClipData::default()
        };
        let intent = std::cell::Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let mut intents = Intents::default();
            clip_details(ui, &project, clip, &data, true, 268.0, &mut intents);
            intent.set(intents.op);
        };
        let mut ui = pxui::doc::frames(2, &mut draw);
        for (label, command, color) in [
            ("No color", "item.set", Some(Json::Null)),
            ("Blue", "item.set", Some(Json::String("blue".into()))),
            ("Lift to Overlay ⌥⌘↑", "clip.lift", None),
        ] {
            let button = testing::find(&ui, Role::Button, label).unwrap();
            testing::click(
                &mut ui,
                f64::from(button.x + button.w * 0.5),
                f64::from(button.y + button.h * 0.5),
                &mut draw,
            );
            let op = intent.take().unwrap();
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            assert_eq!(op.field("color"), color.as_ref());
            assert_eq!(
                op.field("items").and_then(Json::as_array),
                Some([Json::String(clip.to_string())].as_slice())
            );
        }
    }
    #[test]
    fn clip_framing_controls_dispatch_fit_and_crop_reset() {
        let clip = ClipId::from_serial(1);
        let framing_value = Framing {
            crop: Some(splice_model::Rect {
                x: 0.25,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            }),
            fit: Fit::Fit,
        };
        let intent = std::cell::Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let mut intents = Intents::default();
            framing(
                ui,
                TimelineId::Clip(clip),
                framing_value,
                [1920, 1080],
                &mut intents,
            );
            intent.set(intents.op);
        };
        let mut ui = pxui::doc::frames(2, &mut draw);
        for label in ["Fill", "Reset crop"] {
            let button = testing::find(&ui, Role::Button, label).unwrap();
            testing::click(
                &mut ui,
                f64::from(button.x + button.w * 0.5),
                f64::from(button.y + button.h * 0.5),
                &mut draw,
            );
            let op = intent.take().unwrap();
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some("clip.crop"));
            assert_eq!(
                op.field("items").and_then(Json::as_array),
                Some([Json::String(clip.to_string())].as_slice())
            );
            if label == "Fill" {
                assert_eq!(op.field("fit").and_then(Json::as_str), Some("fill"));
            } else {
                assert_eq!(op.field("rect"), Some(&Json::Null));
            }
        }
    }
    #[test]
    fn clip_name_and_trim_dispatch_only_when_committed() {
        let clip = ClipId::from_serial(1);
        let intent = std::cell::Cell::new(None);
        let field = property_field(268.0);
        let mut draw = |ui: &mut Ui| {
            let mut intents = Intents::default();
            clip_name(ui, clip, "", "Camera", field, &mut intents);
            if let Some(at) = frame_field(ui, "Out", 60, 3, i32::MAX, field) {
                intents.op = Some(trim_op(TimelineId::Clip(clip), "out", at));
            }
            intent.set(intents.op);
        };
        let mut ui = pxui::doc::frames(2, &mut draw);
        for (field, text, key) in [
            ("Clip name", "Discarded", pxui::KeyCode::Escape),
            ("Clip name", "Renamed", pxui::KeyCode::Enter),
            ("Out", "45", pxui::KeyCode::Enter),
        ] {
            let rect = testing::find(&ui, Role::TextField, field).unwrap();
            testing::click(
                &mut ui,
                f64::from(rect.x + rect.w * 0.5),
                f64::from(rect.y + rect.h * 0.5),
                &mut draw,
            );
            assert!(ui.text_input_focused());
            let event = pxui::Event {
                kind: pxui::Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: text.as_ptr(),
                    len: text.len(),
                },
                ..pxui::Event::default()
            };
            testing::frame(&mut ui, &[event], &mut draw);
            assert!(
                intent.take().is_none(),
                "typing must leave the project unchanged"
            );
            testing::key(&mut ui, key, &mut draw);
            assert!(!ui.text_input_focused());
            let op = intent.take();
            if key == pxui::KeyCode::Escape {
                assert!(op.is_none());
            } else if field == "Clip name" {
                let op = op.unwrap();
                assert_eq!(op.field("cmd").and_then(Json::as_str), Some("item.set"));
                assert_eq!(op.field("name").and_then(Json::as_str), Some(text));
            } else {
                let op = op.unwrap();
                assert_eq!(op.field("cmd").and_then(Json::as_str), Some("item.trim"));
                assert_eq!(op.field("edge").and_then(Json::as_str), Some("out"));
                assert_eq!(op.field("to").and_then(Json::as_i64), Some(45));
            }
        }
    }
    #[test]
    fn clip_fade_slider_dispatches_item_set_after_release() {
        let mut project = Project::default();
        let clip = ClipId::from_serial(1);
        project
            .clips
            .insert(splice_model::Clip {
                id: clip,
                timing: splice_model::ClipTiming {
                    start: 0,
                    length: 60,
                    source: 0,
                    speed: 1.0,
                },
                data: splice_model::ClipData::default(),
            })
            .unwrap();
        let intent = std::cell::Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            intent.set(
                paint(
                    ui,
                    Some(&project),
                    Selection::Clip(clip),
                    Scope::Selection,
                    60,
                    (268.0, false),
                    (&[], true, None, None, None),
                )
                .op,
            );
        };
        let mut ui = inspector_ui();
        let frame = pxui::FrameIn::new(320.0, 640.0);
        for _ in 0..2 {
            ui.frame(&frame, &mut draw).unwrap();
        }
        let fade = testing::find(&ui, Role::Slider, "Fade in").unwrap();
        for (kind, fraction) in [
            (pxui::Event::POINTER_PRESSED, 0.25),
            (pxui::Event::POINTER_MOVED, 0.5),
            (pxui::Event::POINTER_RELEASED, 0.5),
        ] {
            let events = [pxui::Event {
                kind,
                x: f64::from(fade.x + fade.w * fraction),
                y: f64::from(fade.y + fade.h * 0.5),
                ..pxui::Event::default()
            }];
            let input = pxui::FrameIn::new(320.0, 640.0).events(&events);
            ui.frame(&input, &mut draw).unwrap();
            if kind != pxui::Event::POINTER_RELEASED {
                assert!(intent.take().is_none());
            }
        }
        let op = intent.take().unwrap();
        assert_eq!(op.field("cmd").and_then(Json::as_str), Some("item.set"));
        let frames = op.field("fadeIn").and_then(Json::as_i64).unwrap();
        assert!((1..60).contains(&frames));
    }
    #[test]
    fn marker_and_lane_controls_apply_valid_commands() {
        use splice_core::{edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let mut project = Project::default();
        let setup = splice_json::parse(
            r#"[{"cmd":"marker.add","at":18,"name":"Beat"},
                {"cmd":"title.add","at":0,"length":30,"text":"Card"}]"#,
        )
        .unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
        let marker = project.markers[0].id;
        let lane = project.items.data(project.items.order()[0]).unwrap().lane;
        let spine = project.spine.id;
        let project = RefCell::new(project);
        let selection = Cell::new(Selection::Marker(marker));
        let last = RefCell::new(Intents::default());
        let mut draw = |ui: &mut Ui| {
            *last.borrow_mut() = paint(
                ui,
                Some(&project.borrow()),
                selection.get(),
                Scope::Selection,
                60,
                (268.0, false),
                (&[], false, None, None, None),
            );
        };
        let mut ui = inspector_ui();
        for _ in 0..2 {
            testing::frame(&mut ui, &[], &mut draw);
        }
        let click = |ui: &mut Ui, role, label, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, label).unwrap();
            testing::click(
                ui,
                f64::from(rect.x + rect.w / 2.0),
                f64::from(rect.y + rect.h / 2.0),
                draw,
            );
        };
        let apply_op = |command| {
            let op = last.borrow_mut().op.take().unwrap();
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        };
        for (label, text, key, command) in [
            ("Marker name", "Discard", pxui::KeyCode::Escape, None),
            (
                "Marker name",
                "Cut here",
                pxui::KeyCode::Enter,
                Some("marker.set"),
            ),
            ("Frame", "48", pxui::KeyCode::Enter, Some("marker.set")),
            (
                "Lane name",
                "  Callouts  ",
                pxui::KeyCode::Enter,
                Some("lane.rename"),
            ),
        ] {
            if label == "Lane name" {
                selection.set(Selection::Lane(lane));
                for _ in 0..2 {
                    testing::frame(&mut ui, &[], &mut draw);
                }
                assert!(testing::find(&ui, Role::Label, "graphics").is_some());
                assert!(testing::find(&ui, Role::Label, "1").is_some());
            }
            click(&mut ui, Role::TextField, label, &mut draw);
            assert!(ui.text_input_focused());
            let event = pxui::Event {
                kind: pxui::Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: text.as_ptr(),
                    len: text.len(),
                },
                ..pxui::Event::default()
            };
            testing::frame(&mut ui, &[event], &mut draw);
            assert!(last.borrow().op.is_none());
            testing::key(&mut ui, key, &mut draw);
            assert!(!ui.text_input_focused());
            if let Some(command) = command {
                apply_op(command);
            } else {
                assert!(last.borrow().op.is_none());
                assert_eq!(project.borrow().markers[0].name, "Beat");
            }
            testing::frame(&mut ui, &[], &mut draw);
        }
        assert_eq!(
            project
                .borrow()
                .lanes
                .iter()
                .find(|entry| entry.id == lane)
                .unwrap()
                .name,
            "Callouts"
        );
        assert_eq!(project.borrow().markers[0].name, "Cut here");
        assert_eq!(project.borrow().markers[0].at, 48);

        selection.set(Selection::Lane(spine));
        for _ in 0..2 {
            testing::frame(&mut ui, &[], &mut draw);
        }
        assert!(testing::find(&ui, Role::TextField, "Lane name").is_none());
        assert!(testing::find(&ui, Role::Label, "spine").is_some());
        assert!(testing::find(&ui, Role::Label, "0").is_some());

        selection.set(Selection::Marker(marker));
        for _ in 0..2 {
            testing::frame(&mut ui, &[], &mut draw);
        }
        click(&mut ui, Role::Button, "Jump to marker", &mut draw);
        assert_eq!(last.borrow().seek, Some(48));
        assert!(last.borrow().op.is_none());
        click(&mut ui, Role::Button, "Delete marker", &mut draw);
        assert_eq!(
            last.borrow()
                .op
                .as_ref()
                .and_then(|op| op.field("ripple"))
                .and_then(Json::as_bool),
            Some(false)
        );
        apply_op("item.delete");
        assert!(project.borrow().markers.is_empty());
    }

    #[test]
    fn custom_speed_applies_with_mixed_fps_and_magnetic_mode() {
        use splice_core::{commands, edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        for ripple in [false, true] {
            let (mut project, id, slot) = component_fixture();
            *project.clips.timing_mut(slot).unwrap() = ClipTiming {
                start: 0,
                length: 60,
                source: 60,
                speed: 1.0,
            };
            let next = ClipId::from_serial(2);
            project
                .clips
                .insert(splice_model::Clip {
                    id: next,
                    timing: ClipTiming {
                        start: 60,
                        length: 60,
                        source: 0,
                        speed: 1.0,
                    },
                    data: ClipData::default(),
                })
                .unwrap();
            let next = project.clips.find(next).unwrap();
            let project = RefCell::new(project);
            let intent = Cell::new(None);
            let mut draw = |ui: &mut Ui| {
                if let Some(op) = paint(
                    ui,
                    Some(&project.borrow()),
                    Selection::Clip(id),
                    Scope::Selection,
                    120,
                    (268.0, false),
                    (&[], ripple, None, None, None),
                )
                .op
                {
                    intent.set(Some(op));
                }
            };
            let mut ui = inspector_ui();
            let frame = FrameIn::new(268.0, 500.0);
            for _ in 0..2 {
                ui.frame(&frame, &mut draw).unwrap();
            }
            const ENTER: u32 = 0x11_0005;
            for (text, expected) in [("32", None), ("0.24", None), ("1.25", Some(1.25))] {
                let field = testing::find(&ui, Role::TextField, "Custom speed").unwrap();
                for kind in [pxui::Event::POINTER_PRESSED, pxui::Event::POINTER_RELEASED] {
                    let event = pxui::Event::pointer(
                        kind,
                        0,
                        f64::from(field.x + field.w * 0.5),
                        f64::from(field.y + field.h * 0.5),
                    );
                    ui.frame(&frame.events(&[event]), &mut draw).unwrap();
                }
                assert!(ui.text_input_focused());
                let event = pxui::Event {
                    kind: pxui::Event::TEXT_INPUT,
                    text: pxui::Str {
                        ptr: text.as_ptr(),
                        len: text.len(),
                    },
                    ..pxui::Event::default()
                };
                ui.frame(&frame.events(&[event]), &mut draw).unwrap();
                assert!(intent.take().is_none());
                let events = [
                    pxui::Event {
                        kind: pxui::Event::KEY_PRESSED,
                        code: ENTER,
                        ..pxui::Event::default()
                    },
                    pxui::Event {
                        kind: pxui::Event::KEY_RELEASED,
                        code: ENTER,
                        ..pxui::Event::default()
                    },
                ];
                ui.frame(&frame.events(&events), &mut draw).unwrap();
                let op = intent.take();
                assert_eq!(
                    op.as_ref()
                        .and_then(|op| op.field("speed"))
                        .and_then(Json::as_f64),
                    expected
                );
                if let Some(op) = op {
                    assert_eq!(op.field("ripple").and_then(Json::as_bool), Some(ripple));
                    commands::decode(&op).unwrap();
                    apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
                }
            }
            {
                let project = project.borrow();
                let current = project.clips.timing(slot).unwrap();
                assert_eq!(
                    (current.speed, current.source, current.length),
                    (1.25, 60, 48)
                );
                assert_eq!(
                    timing::consumed(&project, slot, f64::from(current.length)).unwrap(),
                    120.0
                );
                assert_eq!(
                    project.clips.timing(next).unwrap().start,
                    if ripple { 48 } else { 60 }
                );
            }

            ui.frame(&frame, &mut draw).unwrap();
            let slider = testing::find(&ui, Role::Slider, "Custom").unwrap();
            for (kind, fraction) in [
                (pxui::Event::POINTER_PRESSED, 0.9),
                (pxui::Event::POINTER_MOVED, 0.5),
                (pxui::Event::POINTER_RELEASED, 0.5),
            ] {
                let event = pxui::Event::pointer(
                    kind,
                    0,
                    f64::from(slider.x + slider.w * fraction),
                    f64::from(slider.y + slider.h * 0.5),
                );
                ui.frame(&frame.events(&[event]), &mut draw).unwrap();
                if kind != pxui::Event::POINTER_RELEASED {
                    assert!(
                        intent.take().is_none(),
                        "drag preview must leave Core unchanged"
                    );
                }
            }
            let op = intent.take().unwrap();
            let speed = op.field("speed").and_then(Json::as_f64).unwrap();
            assert!((0.25..=16.0).contains(&speed));
            assert!((speed * 20.0 - (speed * 20.0).round()).abs() < 1e-9);
            assert_eq!(op.field("ripple").and_then(Json::as_bool), Some(ripple));
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
            assert_eq!(project.borrow().clips.timing(slot).unwrap().speed, speed);
        }
    }

    #[test]
    fn selected_title_and_overlay_controls_apply_store_commands() {
        use splice_core::{edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let mut project = Project::default();
        project.media.push(splice_model::Media {
            id: "med_1".parse().unwrap(),
            kind: MediaKind::Video,
            path: "video.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: splice_model::Probe {
                fps: 60.0,
                frames: 120,
                width: 1920,
                height: 1080,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        let setup = splice_json::parse(
            r#"[
            {"cmd":"clip.insert","media":"med_1"},
            {"cmd":"title.add","at":6,"length":18,"text":"Opening"},
            {"cmd":"overlay.add","kind":"media","media":"med_1","at":8,"length":30}
        ]"#,
        )
        .unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
        let [title, overlay] = [false, true].map(|media| {
            let slot = project
                .items
                .order()
                .iter()
                .copied()
                .find(|&slot| {
                    matches!(project.items.data(slot).unwrap().kind, ItemKind::Media(_)) == media
                })
                .unwrap();
            project.items.id(slot).unwrap()
        });
        let project = RefCell::new(project);
        let selection = Cell::new(title);
        let last = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            last.set(
                paint(
                    ui,
                    Some(&project.borrow()),
                    Selection::Item(selection.get()),
                    Scope::Selection,
                    60,
                    (268.0, false),
                    (&[], false, None, None, None),
                )
                .op,
            );
        };
        let mut ui = inspector_ui();
        let frame = FrameIn::new(268.0, 500.0);
        let click = |ui: &mut Ui, role, label, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, label).unwrap();
            for kind in [pxui::Event::POINTER_PRESSED, pxui::Event::POINTER_RELEASED] {
                let event = pxui::Event::pointer(
                    kind,
                    0,
                    f64::from(
                        rect.x
                            + if role == Role::Toggle {
                                rect.w - 14.0
                            } else {
                                rect.w / 2.0
                            },
                    ),
                    f64::from(rect.y + rect.h / 2.0),
                );
                ui.frame(&frame.events(&[event]), draw).unwrap();
            }
        };
        let apply_op = |command| {
            let op = last.take().unwrap_or_else(|| panic!("Expected {command}"));
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        };
        for (selected, field, value, command) in [
            (title, "Title text", "Discarded", None),
            (title, "Title text", "New opening", Some("title.set")),
            (title, "Start", "14", Some("item.move")),
            (title, "Length", "20", Some("item.trim")),
            (overlay, "Source", "60", Some("item.slip")),
        ] {
            selection.set(selected);
            for _ in 0..2 {
                ui.frame(&frame, &mut draw).unwrap();
            }
            click(&mut ui, Role::TextField, field, &mut draw);
            assert!(ui.text_input_focused());
            let event = pxui::Event {
                kind: pxui::Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: value.as_ptr(),
                    len: value.len(),
                },
                ..pxui::Event::default()
            };
            ui.frame(&frame.events(&[event]), &mut draw).unwrap();
            assert!(last.take().is_none());
            testing::key(
                &mut ui,
                if command.is_some() {
                    pxui::KeyCode::Enter
                } else {
                    pxui::KeyCode::Escape
                },
                &mut draw,
            );
            assert!(!ui.text_input_focused());
            if let Some(command) = command {
                apply_op(command);
            } else {
                assert!(last.take().is_none());
            }
        }
        for (selected, control, command) in [
            (title, "Pinned to timeline", "item.anchor"),
            (overlay, "Behind presenter", "overlay.set"),
            (overlay, "Audio muted", "overlay.set"),
        ] {
            selection.set(selected);
            for _ in 0..2 {
                ui.frame(&frame, &mut draw).unwrap();
            }
            click(&mut ui, Role::Toggle, control, &mut draw);
            apply_op(command);
        }

        let project = project.borrow();
        let title_id = title;
        let slot = project.items.find(title).unwrap();
        assert!(
            matches!(&project.items.data(slot).unwrap().kind, ItemKind::Title { text } if text == "New opening")
        );
        let title = project.items.timing(slot).unwrap();
        assert_eq!((title.anchor, title.length), (Anchor::Pinned(14), 20));
        let slot = project.items.find(overlay).unwrap();
        let item = project.items.timing(slot).unwrap();
        assert_eq!(timing::anchor_start(&project, item.anchor).unwrap(), 8);
        assert_eq!(item.length, 30);
        assert!(
            matches!(project.items.data(slot).unwrap().kind, ItemKind::Media(media)
            if media.source == 60 && media.behind_presenter && !media.muted)
        );
        drop(project);
        mem_sys::set_peer(pxui::library_counter);
        for item in [overlay, title_id] {
            selection.set(item);
            for _ in 0..8 {
                ui.frame(&frame, &mut draw).unwrap();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        ui.frame(&frame, &mut draw).unwrap();
                    }
                });
        }
    }

    #[test]
    fn motion_and_overlay_fields_commit_native_item_edits() {
        use pxui::{Event, KeyCode};
        use splice_core::{edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let (mut project, _, _) = component_fixture();
        let setup = splice_json::parse(r#"[
            {"cmd":"overlay.add","kind":"media","media":"med_1","at":24,"length":30,"placement":"card","source":12,"behindPresenter":true},
            {"cmd":"lane.create","kind":"overlay","name":"Cutaways"},
            {"cmd":"lane.create","kind":"overlay","name":"Occupied"},
            {"cmd":"zoom.add","at":24,"length":30,"amount":1.7,"focus":{"x":0.2,"y":0.45},"mode":"content","motionBlur":0.3}
        ]"#).unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
        let [overlay, zoom] = [false, true].map(|zoom| {
            let slot = project
                .items
                .order()
                .iter()
                .copied()
                .find(|&slot| {
                    matches!(project.items.data(slot).unwrap().kind, ItemKind::Zoom(_)) == zoom
                })
                .unwrap();
            project.items.id(slot).unwrap()
        });
        let lane = |name| {
            project
                .lanes
                .iter()
                .find(|lane| lane.name == name)
                .unwrap()
                .id
        };
        let (free, occupied) = (lane("Cutaways"), lane("Occupied"));
        let setup = splice_json::parse(&format!(r#"{{"cmd":"overlay.add","kind":"media","media":"med_1","at":28,"length":10,"lane":"{occupied}"}}"#)).unwrap();
        apply(&mut project, &[setup], Source::Human, false).unwrap();

        let project = RefCell::new(project);
        let selection = Cell::new(overlay);
        let last = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            if let Some(op) = paint(
                ui,
                Some(&project.borrow()),
                Selection::Item(selection.get()),
                Scope::Selection,
                180,
                (268.0, false),
                (&[], false, None, None, None),
            )
            .op
            {
                assert!(last.replace(Some(op)).is_none());
            }
        };
        let mut ui = inspector_ui();
        let frame = FrameIn::new(268.0, 540.0);
        let click = |ui: &mut Ui, role, name, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {name}"));
            let [x, y] = [rect.x + rect.w / 2.0, rect.y + rect.h / 2.0].map(f64::from);
            for event in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                ui.frame(&frame.events(&[Event::pointer(event, 0, x, y)]), draw)
                    .unwrap();
            }
        };
        let key = |ui: &mut Ui, key, draw: &mut dyn FnMut(&mut Ui)| {
            testing::key(ui, key, draw);
            ui.frame(&frame, draw).unwrap();
        };
        let apply_op = |command| {
            let op = last.take().unwrap_or_else(|| panic!("expected {command}"));
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(command));
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        };
        mem_sys::set_peer(pxui::library_counter);
        let check_warm = |ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..8 {
                ui.frame(&frame, draw).unwrap();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        ui.frame(&frame, draw).unwrap();
                    }
                });
            assert!(last.take().is_none());
        };

        check_warm(&mut ui, &mut draw);
        let placement = testing::find(&ui, Role::ComboBox, "Placement").unwrap();
        assert_eq!((placement.x, placement.w, placement.h), (98.0, 158.0, 22.0));
        click(&mut ui, Role::ComboBox, "Placement", &mut draw);
        assert!(last.take().is_none());
        assert!(testing::find(&ui, Role::Button, "corner-br").is_some());
        key(&mut ui, KeyCode::Escape, &mut draw);
        assert!(testing::find(&ui, Role::Button, "corner-br").is_none());
        click(&mut ui, Role::ComboBox, "Placement", &mut draw);
        click(&mut ui, Role::Button, "corner-br", &mut draw);
        apply_op("overlay.set");

        click(&mut ui, Role::ComboBox, "Lane", &mut draw);
        check_warm(&mut ui, &mut draw);
        let disabled = ui
            .a11y_nodes()
            .iter()
            .find(|node| ui.a11y_name_of(node) == "Occupied")
            .unwrap();
        assert_eq!(disabled.states & A11yStates::ENABLED.bits(), 0);
        click(&mut ui, Role::Button, "Occupied", &mut draw);
        assert!(last.take().is_none());
        click(&mut ui, Role::Button, "Cutaways", &mut draw);
        apply_op("item.lane");

        selection.set(zoom);
        check_warm(&mut ui, &mut draw);
        for (value, finish) in [
            ("6", KeyCode::Enter),
            ("4", KeyCode::Escape),
            ("2.375", KeyCode::Enter),
        ] {
            click(&mut ui, Role::TextField, "Amount", &mut draw);
            assert!(ui.text_input_focused());
            let text = Event {
                kind: Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: value.as_ptr(),
                    len: value.len(),
                },
                ..Event::default()
            };
            ui.frame(&frame.events(&[text]), &mut draw).unwrap();
            assert!(last.take().is_none());
            key(&mut ui, finish, &mut draw);
            if value == "2.375" {
                apply_op("zoom.set");
            } else {
                assert!(last.take().is_none());
                if ui.text_input_focused() {
                    key(&mut ui, KeyCode::Escape, &mut draw);
                }
            }
        }

        click(&mut ui, Role::ComboBox, "Spring", &mut draw);
        key(&mut ui, KeyCode::Tab, &mut draw);
        key(&mut ui, KeyCode::Enter, &mut draw);
        apply_op("zoom.set");
        check_warm(&mut ui, &mut draw);
        let project = project.borrow();
        let item = |id| project.items.data(project.items.find(id).unwrap()).unwrap();
        assert_eq!(item(overlay).lane, free);
        assert!(matches!(item(overlay).kind, ItemKind::Media(media)
            if media.placement == Placement::BottomRight && media.source == 12 && media.behind_presenter));
        assert!(matches!(item(zoom).kind, ItemKind::Zoom(zoom)
            if zoom.amount == 2.375 && zoom.spring == Spring::Smooth && zoom.focus == Some([0.2, 0.45])
                && zoom.mode == splice_model::CameraMode::Content && zoom.motion_blur == 0.3));
    }

    #[test]
    fn overlay_and_title_appearance_controls_apply_core_edits() {
        use pxui::{Event, KeyCode};
        use splice_core::{commands, edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let (mut project, original_spine, _) = component_fixture();
        let setup = splice_json::parse(
            r#"[
            {"cmd":"overlay.add","kind":"media","media":"med_1","at":24,"length":45,"source":16},
            {"cmd":"overlay.add","kind":"media","media":"med_1","length":30,
             "anchor":{"pinned":true,"frame":28}},
            {"cmd":"title.add","at":24,"length":45,"text":"Callout"}
        ]"#,
        )
        .unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
        let item_id = |matches: &dyn Fn(Slot) -> bool| {
            let slot = project
                .items
                .order()
                .iter()
                .copied()
                .find(|&slot| matches(slot))
                .unwrap();
            project.items.id(slot).unwrap()
        };
        let media = |slot| matches!(project.items.data(slot).unwrap().kind, ItemKind::Media(_));
        let pinned = |slot| {
            matches!(
                project.items.timing(slot).unwrap().anchor,
                Anchor::Pinned(_)
            )
        };
        let overlay = item_id(&|slot| media(slot) && !pinned(slot));
        let pinned_overlay = item_id(&|slot| media(slot) && pinned(slot));
        let title = item_id(&|slot| !media(slot));
        assert_ne!(
            project
                .items
                .data(project.items.find(overlay).unwrap())
                .unwrap()
                .lane,
            project
                .items
                .data(project.items.find(pinned_overlay).unwrap())
                .unwrap()
                .lane
        );
        let settings = splice_json::parse(&format!(
            r#"[
            {{"cmd":"item.set","items":["{overlay}"],"fadeOut":31}},
            {{"cmd":"clip.crop","items":["{overlay}"],"rect":{{"x":0.25,"y":0.25,"w":0.5,"h":0.5}}}}
        ]"#
        ))
        .unwrap();
        apply(
            &mut project,
            settings.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();

        let project = RefCell::new(project);
        let selected = Cell::new(pinned_overlay);
        let inline = Cell::new(false);
        let action = Cell::new(None);
        let crop = Cell::new(None);
        let parent = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let intents = paint(
                ui,
                Some(&project.borrow()),
                Selection::Item(selected.get()),
                Scope::Selection,
                180,
                (268.0, inline.get()),
                (&[], false, None, None, None),
            );
            if let Some(op) = intents.op {
                assert!(action.replace(Some(op)).is_none());
            }
            crop.set(crop.get().or(intents.crop));
            parent.set(parent.get().or(intents.select_parent));
        };
        let mut ui = inspector_ui();
        let input = FrameIn::new(268.0, 900.0);
        let ready = |ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..2 {
                ui.frame(&input, draw).unwrap();
            }
        };
        let click = |ui: &mut Ui, role, label, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, label).unwrap_or_else(|| panic!("missing {label}"));
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                let event = Event::pointer(
                    kind,
                    0,
                    f64::from(rect.x + rect.w / 2.0),
                    f64::from(rect.y + rect.h / 2.0),
                );
                ui.frame(&input.events(&[event]), draw).unwrap();
            }
        };
        let enter = |ui: &mut Ui, field, value: &str, key, draw: &mut dyn FnMut(&mut Ui)| {
            click(ui, Role::TextField, field, draw);
            assert!(ui.text_input_focused());
            let event = Event {
                kind: Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: value.as_ptr(),
                    len: value.len(),
                },
                ..Event::default()
            };
            ui.frame(&input.events(&[event]), draw).unwrap();
            assert!(action.take().is_none());
            testing::key(ui, key, draw);
            ready(ui, draw);
        };
        let apply_op = |cmd| {
            let op = action.take().unwrap_or_else(|| panic!("expected {cmd}"));
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(cmd));
            commands::decode(&op).unwrap();
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap()
        };
        let disabled = |ui: &Ui, label| {
            let node = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == label)
                .unwrap();
            assert_eq!(node.states & A11yStates::ENABLED.bits(), 0);
        };
        mem_sys::set_peer(pxui::library_counter);
        for (id, document) in [
            (pinned_overlay, false),
            (overlay, false),
            (overlay, true),
            (title, false),
        ] {
            selected.set(id);
            inline.set(document);
            for _ in 0..8 {
                ui.frame(&input, &mut draw).unwrap();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..8 {
                        ui.frame(&input, &mut draw).unwrap();
                    }
                });
            assert!(action.take().is_none() && crop.get().is_none() && parent.get().is_none());
        }

        selected.set(pinned_overlay);
        ready(&mut ui, &mut draw);
        disabled(&ui, "Replace Spine Clip");
        click(&mut ui, Role::Button, "Replace Spine Clip", &mut draw);
        assert!(action.take().is_none());
        assert!(testing::find(&ui, Role::Button, "Select anchor clip").is_none());

        selected.set(overlay);
        ready(&mut ui, &mut draw);
        assert!(testing::find(&ui, Role::Label, "960×540 at 480, 270").is_some());
        click(&mut ui, Role::Button, "Select anchor clip", &mut draw);
        assert_eq!(parent.take(), Some(original_spine));
        click(&mut ui, Role::Button, "Crop overlay", &mut draw);
        assert_eq!(crop.take(), Some(TimelineId::Item(overlay)));
        assert!(action.take().is_none());
        click(&mut ui, Role::Button, "Fit", &mut draw);
        apply_op("clip.crop");
        assert!(testing::find(&ui, Role::Label, "960×540 at 480, 270").is_some());
        click(&mut ui, Role::Button, "Reset crop", &mut draw);
        apply_op("clip.crop");
        ready(&mut ui, &mut draw);
        disabled(&ui, "Reset crop");
        assert!(testing::find(&ui, Role::Label, "Full frame · 1920×1080 px").is_some());
        click(&mut ui, Role::Button, "Reset crop", &mut draw);
        assert!(action.take().is_none());

        enter(&mut ui, "Fade in frames", "15", KeyCode::Enter, &mut draw);
        assert!(
            action.take().is_none(),
            "an overlapping fade does not dispatch"
        );
        if ui.text_input_focused() {
            testing::key(&mut ui, KeyCode::Escape, &mut draw);
            ready(&mut ui, &mut draw);
        }
        let fade = testing::find(&ui, Role::Slider, "Fade in").unwrap();
        let readout = testing::find(&ui, Role::TextField, "Fade in frames").unwrap();
        assert_eq!((readout.w, readout.h), (52.0, 22.0));
        for (kind, fraction) in [
            (Event::POINTER_PRESSED, 0.2),
            (Event::POINTER_MOVED, 0.99),
            (Event::POINTER_RELEASED, 0.99),
        ] {
            let event = Event::pointer(
                kind,
                0,
                f64::from(fade.x + fade.w * fraction),
                f64::from(fade.y + fade.h / 2.0),
            );
            ui.frame(&input.events(&[event]), &mut draw).unwrap();
            if kind != Event::POINTER_RELEASED {
                assert!(action.take().is_none());
            }
        }
        apply_op("item.set");
        click(&mut ui, Role::Button, "Orange", &mut draw);
        apply_op("item.set");
        {
            let project = project.borrow();
            let item = project
                .items
                .data(project.items.find(overlay).unwrap())
                .unwrap();
            assert_eq!(
                (item.appearance.fade_in, item.appearance.fade_out),
                (14, 31)
            );
            assert_eq!(item.appearance.color.as_deref(), Some("orange"));
            assert!(matches!(item.kind, ItemKind::Media(media)
                if media.framing.fit == Fit::Fit && media.framing.crop.is_none() && media.source == 16));
        }
        ready(&mut ui, &mut draw);
        click(&mut ui, Role::Button, "No color", &mut draw);
        apply_op("item.set");

        selected.set(title);
        ready(&mut ui, &mut draw);
        enter(&mut ui, "Fade out frames", "7", KeyCode::Enter, &mut draw);
        apply_op("item.set");
        enter(&mut ui, "Fade out frames", "10", KeyCode::Escape, &mut draw);
        assert!(action.take().is_none());
        click(&mut ui, Role::Button, "Blue", &mut draw);
        apply_op("item.set");
        {
            let project = project.borrow();
            let appearance = &project
                .items
                .data(project.items.find(title).unwrap())
                .unwrap()
                .appearance;
            assert_eq!(
                (appearance.fade_out, appearance.color.as_deref()),
                (7, Some("blue"))
            );
            let appearance = &project
                .items
                .data(project.items.find(overlay).unwrap())
                .unwrap()
                .appearance;
            assert_eq!(appearance.color, None);
        }

        selected.set(overlay);
        ready(&mut ui, &mut draw);
        click(&mut ui, Role::Button, "Fill", &mut draw);
        apply_op("clip.crop");
        click(&mut ui, Role::Button, "Replace Spine Clip", &mut draw);
        let report = apply_op("overlay.replaceSpine");
        let commands::Target::Timeline(TimelineId::Clip(replacement)) =
            report.changes[0].created[0]
        else {
            panic!("expected replacement clip");
        };
        let project = project.borrow();
        assert!(
            project.items.find(overlay).is_none() && project.clips.find(original_spine).is_none()
        );
        assert!(project.items.find(pinned_overlay).is_some());
        let slot = project.clips.find(replacement).unwrap();
        let clip = project.clips.timing(slot).unwrap();
        assert_eq!(
            (clip.start, clip.source, clip.length, clip.speed),
            (12, 16, 45, 1.0)
        );
        assert_eq!(project.clips.data(slot).unwrap().framing.fit, Fit::Fill);
    }

    #[test]
    fn local_effect_controls_apply_exact_ids_and_keep_params() {
        use splice_core::{edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let mut project = Project::default();
        project.media.push(splice_model::Media {
            id: "med_1".parse().unwrap(),
            kind: MediaKind::Video,
            path: "video.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: splice_model::Probe {
                fps: 30.0,
                frames: 120,
                width: 1920,
                height: 1080,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        let setup = splice_json::parse(
            r#"[
            {"cmd":"clip.insert","media":"med_1"},
            {"cmd":"overlay.add","kind":"media","media":"med_1","at":8,"length":30},
            {"cmd":"effect.add","target":"project","type":"window","enabled":false}
        ]"#,
        )
        .unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();
        let clip = project.clips.id(project.clips.order()[0]).unwrap();
        let item = project.items.id(project.items.order()[0]).unwrap();
        let lane = project.items.data(project.items.order()[0]).unwrap().lane;
        let spine = project.spine.id;
        let project = RefCell::new(project);

        let selection = Cell::new(Selection::None);
        let scope = Cell::new(Scope::Selection);
        let last = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            last.set(
                paint(
                    ui,
                    Some(&project.borrow()),
                    selection.get(),
                    scope.get(),
                    120,
                    (268.0, false),
                    (&[], true, None, None, None),
                )
                .op,
            );
        };
        let mut ui = inspector_ui();
        let frame = FrameIn::new(268.0, 1000.0);
        let title = splice_core::commands::effect_schemas()
            .field("window")
            .unwrap()
            .field("title")
            .and_then(Json::as_str)
            .unwrap();
        let click = |ui: &mut Ui, role, label, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, label).unwrap();
            let x = rect.x
                + if role == Role::Toggle {
                    rect.w - 14.0
                } else {
                    rect.w / 2.0
                };
            for kind in [pxui::Event::POINTER_PRESSED, pxui::Event::POINTER_RELEASED] {
                let event =
                    pxui::Event::pointer(kind, 0, f64::from(x), f64::from(rect.y + rect.h / 2.0));
                ui.frame(&frame.events(&[event]), draw).unwrap();
            }
        };
        mem_sys::set_peer(pxui::library_counter);
        for (selected, owner, inspector_scope) in [
            (Selection::Clip(clip), clip.to_string(), Scope::Selection),
            (Selection::Item(item), item.to_string(), Scope::Selection),
            (Selection::Lane(lane), lane.to_string(), Scope::Selection),
            (Selection::Lane(spine), spine.to_string(), Scope::Selection),
            (Selection::None, "project".into(), Scope::Canvas),
        ] {
            let add = splice_json::parse(&format!(
                r#"{{"cmd":"effect.add","target":"{owner}","type":"window","params":{{"padding":0.12}}}}"#
            )).unwrap();
            apply(&mut project.borrow_mut(), &[add], Source::Human, false).unwrap();
            selection.set(selected);
            scope.set(inspector_scope);
            for _ in 0..8 {
                ui.frame(&frame, &mut draw).unwrap();
            }
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || ui.frame(&frame, &mut draw).map(|_| ()).unwrap());
            let effect = |project: &Project| {
                let effects = match selected {
                    Selection::Clip(id) => {
                        &project
                            .clips
                            .data(project.clips.find(id).unwrap())
                            .unwrap()
                            .appearance
                            .effects
                    }
                    Selection::Item(id) => {
                        &project
                            .items
                            .data(project.items.find(id).unwrap())
                            .unwrap()
                            .appearance
                            .effects
                    }
                    Selection::Lane(id) if id == spine => &project.spine.effects,
                    Selection::Lane(id) => {
                        &project
                            .lanes
                            .iter()
                            .find(|lane| lane.id == id)
                            .unwrap()
                            .effects
                    }
                    _ => &project.effects,
                };
                effects
                    .iter()
                    .find(|effect| effect.kind == "window")
                    .cloned()
            };
            let before = effect(&project.borrow()).unwrap();
            click(&mut ui, Role::Toggle, title, &mut draw);
            let op = last.take().expect("toggle emits effect.set");
            assert_eq!(
                op.field("effect").and_then(Json::as_str),
                Some(before.id.to_string().as_str())
            );
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
            let after = effect(&project.borrow()).unwrap();
            let on = match selected {
                Selection::Item(_) => false,
                Selection::Lane(id) => id == spine,
                _ => true,
            };
            assert_eq!(after.enabled, Some(on));
            assert_eq!(after.params, before.params);
            if inspector_scope == Scope::Selection {
                assert_eq!(project.borrow().effects[0].enabled, Some(false));
            }
            ui.frame(&frame, &mut draw).unwrap();
            click(&mut ui, Role::Button, "Window options", &mut draw);
            click(
                &mut ui,
                Role::Button,
                if on && inspector_scope == Scope::Selection {
                    "Reset to inherited"
                } else {
                    "Remove effect"
                },
                &mut draw,
            );
            let op = last.take().expect("remove emits effect.remove");
            assert_eq!(
                op.field("effects").and_then(Json::as_array).unwrap()[0].as_str(),
                Some(before.id.to_string().as_str())
            );
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
            assert!(effect(&project.borrow()).is_none());
            if inspector_scope == Scope::Selection {
                assert_eq!(project.borrow().effects[0].enabled, Some(false));
            }
        }
    }

    #[test]
    fn effective_effect_controls_round_trip_values_and_scope() {
        use pxui::{Event, KeyCode};
        use splice_core::{commands, edit::apply, package::journal::Source};
        use std::cell::{Cell, RefCell};

        let (mut project, clip, _) = component_fixture();
        let setup = splice_json::parse(r##"[
            {"cmd":"effect.add","target":"project","type":"window","params":{"padding":0.11,"background":"color","color":"#112233"}},
            {"cmd":"effect.add","target":"lan_spine","type":"window","params":{"shadow":0.4}},
            {"cmd":"effect.add","target":"lan_spine","type":"cursor","params":{"clickRings":false}}
        ]"##).unwrap();
        apply(
            &mut project,
            setup.as_array().unwrap(),
            Source::Human,
            false,
        )
        .unwrap();

        let project = RefCell::new(project);
        let last = Cell::new(None);
        let selected_lane = Cell::new(None);
        let mut draw = |ui: &mut Ui| {
            let intents = paint(
                ui,
                Some(&project.borrow()),
                Selection::Clip(clip),
                Scope::Selection,
                180,
                (268.0, false),
                (&[], false, None, None, None),
            );
            if let Some(op) = intents.op {
                assert!(last.replace(Some(op)).is_none());
            }
            if let Some(lane) = intents.select_lane {
                selected_lane.set(Some(lane));
            }
        };
        let mut ui = inspector_ui();
        let frame = FrameIn::new(268.0, 2300.0);
        let ready = |ui: &mut Ui, draw: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..2 {
                ui.frame(&frame, draw).unwrap();
            }
        };
        let click = |ui: &mut Ui, role, name, draw: &mut dyn FnMut(&mut Ui)| {
            let rect = testing::find(ui, role, name).unwrap_or_else(|| panic!("missing {name}"));
            let x = rect.x
                + if role == Role::Toggle {
                    rect.w - 13.0
                } else {
                    rect.w / 2.0
                };
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                let event = Event::pointer(kind, 0, f64::from(x), f64::from(rect.y + rect.h / 2.0));
                ui.frame(&frame.events(&[event]), draw).unwrap();
            }
        };
        let enter = |ui: &mut Ui, label, text: &str, key, draw: &mut dyn FnMut(&mut Ui)| {
            click(ui, Role::TextField, label, draw);
            assert!(ui.text_input_focused());
            let event = Event {
                kind: Event::TEXT_INPUT,
                text: pxui::Str {
                    ptr: text.as_ptr(),
                    len: text.len(),
                },
                ..Event::default()
            };
            ui.frame(&frame.events(&[event]), draw).unwrap();
            assert!(last.take().is_none());
            testing::key(ui, key, draw);
            ready(ui, draw);
        };
        let seen = |ui: &Ui, name, value| {
            let field = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::TextField) && ui.a11y_name_of(node) == name)
                .unwrap();
            assert_eq!(ui.a11y_value_of(field), Some(value));
        };
        let effect = |kind| {
            let project = project.borrow();
            project
                .clips
                .data(project.clips.find(clip).unwrap())
                .unwrap()
                .appearance
                .effects
                .iter()
                .find(|effect| effect.kind == kind)
                .cloned()
        };
        let dispatch = |cmd, id: Option<splice_model::EffectId>| {
            let op = last.take().unwrap_or_else(|| panic!("expected {cmd}"));
            assert_eq!(op.field("cmd").and_then(Json::as_str), Some(cmd));
            if let Some(id) = id {
                assert_eq!(
                    op.field("effect").and_then(Json::as_str),
                    Some(id.to_string().as_str())
                );
            } else {
                assert_eq!(
                    op.field("target").and_then(Json::as_str),
                    Some(clip.to_string().as_str())
                );
            }
            commands::decode(&op).unwrap();
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        };

        ready(&mut ui, &mut draw);
        seen(&ui, "Padding", "0.11");
        seen(&ui, "Shadow", "0.4");
        seen(&ui, "Corners", "0.03");
        assert!(testing::find(&ui, Role::Label, "Project").is_some());
        assert!(testing::find(&ui, Role::Label, "Default").is_some());
        assert!(testing::find(&ui, Role::Label, "Caption Style").is_none());
        click(&mut ui, Role::Button, "Window options", &mut draw);
        click(&mut ui, Role::Button, "Edit on Video lane", &mut draw);
        assert_eq!(selected_lane.take(), Some(LaneId::SPINE));
        assert!(last.take().is_none());

        let slider = testing::find(&ui, Role::Slider, "Shadow").unwrap();
        for (event, fraction) in [
            (Event::POINTER_PRESSED, 0.4),
            (Event::POINTER_MOVED, 0.75),
            (Event::POINTER_RELEASED, 0.75),
        ] {
            let event = Event::pointer(
                event,
                0,
                (slider.x + slider.w * fraction).into(),
                (slider.y + slider.h / 2.0).into(),
            );
            ui.frame(&frame.events(&[event]), &mut draw).unwrap();
            if event.kind != Event::POINTER_RELEASED {
                assert!(last.take().is_none());
            }
        }
        dispatch("effect.set", None);
        let id = effect("window").unwrap().id;
        ready(&mut ui, &mut draw);
        seen(&ui, "Shadow", "0.75");
        click(&mut ui, Role::Button, "Reset Shadow", &mut draw);
        dispatch("effect.set", Some(id));
        assert!(effect("window").is_none());
        ready(&mut ui, &mut draw);
        seen(&ui, "Shadow", "0.4");

        enter(&mut ui, "Padding", "0.18", KeyCode::Enter, &mut draw);
        dispatch("effect.set", None);
        let id = effect("window").unwrap().id;
        ready(&mut ui, &mut draw);
        seen(&ui, "Padding", "0.18");
        enter(&mut ui, "Color", "#bad", KeyCode::Enter, &mut draw);
        assert!(last.take().is_none());
        if ui.text_input_focused() {
            testing::key(&mut ui, KeyCode::Escape, &mut draw);
            ready(&mut ui, &mut draw);
        }
        enter(&mut ui, "Color", "#33aa55", KeyCode::Enter, &mut draw);
        dispatch("effect.set", Some(id));
        ready(&mut ui, &mut draw);
        seen(&ui, "Color", "#33aa55");
        click(&mut ui, Role::ComboBox, "Background", &mut draw);
        click(&mut ui, Role::Button, "gradient", &mut draw);
        dispatch("effect.set", Some(id));
        ready(&mut ui, &mut draw);

        click(&mut ui, Role::Button, "Reset Padding", &mut draw);
        dispatch("effect.set", Some(id));
        ready(&mut ui, &mut draw);
        seen(&ui, "Padding", "0.11");
        let window = effect("window").unwrap();
        assert_eq!(window.id, id);
        assert!(window.params.iter().any(|p| p.name == "background"
            && matches!(&p.value, EffectValue::Text(text) if text == "gradient")));

        click(&mut ui, Role::Toggle, "Click rings", &mut draw);
        dispatch("effect.set", None);
        assert!(
            effect("cursor")
                .unwrap()
                .params
                .iter()
                .any(|p| p.name == "clickRings" && p.value == EffectValue::Bool(true))
        );
        ready(&mut ui, &mut draw);
        click(&mut ui, Role::Button, "Window options", &mut draw);
        click(&mut ui, Role::Button, "Use for whole lane", &mut draw);
        dispatch("effect.promote", Some(id));
        assert!(effect("window").is_none());
        assert!(
            project
                .borrow()
                .spine
                .effects
                .iter()
                .find(|e| e.kind == "window")
                .unwrap()
                .params
                .iter()
                .any(|p| p.name == "color"
                    && matches!(&p.value, EffectValue::Text(s) if s == "#33aa55"))
        );

        ready(&mut ui, &mut draw);
        click(&mut ui, Role::Button, "+ Effect", &mut draw);
        assert!(testing::find(&ui, Role::Button, "Caption Style").is_none());
        assert!(testing::find(&ui, Role::Button, "Transition").is_none());
        let title = commands::effect_schemas()
            .field("camera")
            .unwrap()
            .field("title")
            .unwrap()
            .as_str()
            .unwrap();
        mem_sys::set_peer(pxui::library_counter);
        for _ in 0..8 {
            ui.frame(&frame, &mut draw).unwrap();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut || ui.frame(&frame, &mut draw).map(|_| ()).unwrap());
        click(&mut ui, Role::Button, title, &mut draw);
        dispatch("effect.add", None);
        assert!(effect("camera").is_some());

        // A stack can share a type without sharing identity or the edited parameter.
        for amount in ["0.2", "0.4"] {
            let op = splice_json::parse(&format!(
                r#"{{"cmd":"effect.add","target":"{clip}","type":"mask","params":{{"amount":{amount}}}}}"#,
            ))
            .unwrap();
            apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        }
        ready(&mut ui, &mut draw);
        let masks = || {
            let project = project.borrow();
            project
                .clips
                .data(project.clips.find(clip).unwrap())
                .unwrap()
                .appearance
                .effects
                .iter()
                .filter(|effect| effect.kind == "mask")
                .map(|effect| effect.id)
                .collect::<Vec<_>>()
        };
        let stack = masks();
        assert_eq!(stack.len(), 2);
        let second = ui
            .a11y_nodes()
            .iter()
            .filter(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == "Mask options")
            .nth(1)
            .unwrap()
            .rect;
        for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
            let event = Event::pointer(
                kind,
                0,
                (second.x + second.w / 2.0).into(),
                (second.y + second.h / 2.0).into(),
            );
            ui.frame(&frame.events(&[event]), &mut draw).unwrap();
        }
        click(&mut ui, Role::Button, "Remove effect", &mut draw);
        let op = last.take().expect("remove exactly one mask");
        assert_eq!(
            op.field("effects").and_then(Json::as_array).unwrap()[0].as_str(),
            Some(stack[1].to_string().as_str()),
        );
        apply(&mut project.borrow_mut(), &[op], Source::Human, false).unwrap();
        assert_eq!(masks(), [stack[0]]);
    }

    #[test]
    fn warm_inspector_frames_do_not_allocate() {
        mem_sys::set_peer(pxui::library_counter);
        let mut project = Project::default();
        let clip = "clp_take".parse().unwrap();
        let media = MediaId::from_serial(1);
        project.media.push(splice_model::Media {
            id: media,
            kind: MediaKind::Video,
            path: "assets/Screen take 01.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: splice_model::Probe {
                fps: 30.0,
                frames: 420,
                width: 1920,
                height: 1080,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        project
            .clips
            .insert(splice_model::Clip {
                id: clip,
                timing: splice_model::ClipTiming {
                    start: 120,
                    length: 420,
                    source: 0,
                    speed: 1.0,
                },
                data: splice_model::ClipData {
                    media: Some(media),
                    ..splice_model::ClipData::default()
                },
            })
            .unwrap();
        let marker = MarkerId::from_serial(1);
        project.markers.push(Marker {
            id: marker,
            at: 18,
            name: "Edit beat".into(),
            color: None,
        });
        let lane = LaneId::from_serial(2);
        project.lanes.push(Lane {
            id: lane,
            name: "Cards".into(),
            kind: LaneKind::Overlay,
            effects: Vec::new(),
        });
        let mut ui = inspector_ui();
        let input = FrameIn::new(268.0, 241.0);
        let mut tick = || {
            for (selection, scope) in [
                (Selection::None, Scope::Selection),
                (Selection::None, Scope::Canvas),
                (Selection::Marker(marker), Scope::Selection),
                (Selection::Lane(lane), Scope::Selection),
                (Selection::Lane(project.spine.id), Scope::Selection),
                (Selection::Clip(clip), Scope::Selection),
            ] {
                ui.frame(&input, &mut |ui| {
                    paint(
                        ui,
                        Some(&project),
                        selection,
                        scope,
                        540,
                        (268.0, false),
                        (&[], true, None, None, None),
                    );
                })
                .unwrap();
            }
        };
        for _ in 0..8 {
            tick();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut tick);

        assert!(testing::find(&ui, Role::Heading, "Screen take 01").is_some());
        assert!(testing::find(&ui, Role::Label, "Spine clip · clp_take").is_some());
        assert!(testing::find(&ui, Role::Label, "120 f · 00:04:00").is_some());
        let source = testing::find(&ui, Role::Label, "from 0 f · 420 f long").unwrap();
        assert!(source.y + source.h <= 241.0);
        for (label, value) in [
            ("Clip name", "Screen take 01"),
            ("In", "120"),
            ("Out", "540"),
        ] {
            let field = ui
                .a11y_nodes()
                .iter()
                .find(|node| node.is_role(Role::TextField) && ui.a11y_name_of(node) == label)
                .unwrap();
            assert_eq!(ui.a11y_value_of(field), Some(value));
            assert_eq!(
                (field.rect.x, field.rect.w, field.rect.h),
                (98.0, 158.0, 22.0)
            );
        }
    }
}
