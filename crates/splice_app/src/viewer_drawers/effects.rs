use super::{BODY, BUTTON, CAPTION, INK, LINE, MUTED, asset_icon};
use pxui::{
    Button, DropRef, Fill, Handle, Label, Node, Paint, Pressable, Rect, Role, Signal, Stroke, Ui,
    geom::{Edges, Rgba},
    ui::{Look, Size, State, States},
};
use splice_core::{
    commands::{Target, effect_schemas},
    edit::{effect_target_allowed, transition_capacity},
};
use splice_json::Value as Json;
use splice_model::{LaneId, Project, TimelineId};

const ACCENT: Rgba = Rgba::rgb(255, 138, 92);
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const CARRY: &str = "splice-effect";
const DETAIL: Label = Label::MONO
    .size(10.0)
    .line_height(14.0)
    .w(Size::Grow)
    .wrap();
const NAME: Label = BODY.line_height(16.8).weight(500);
const GROUP: Label = DETAIL.caps(true).tracking(0.06);
const ROW: States = States::of(Look::NONE)
    .hover(Look::NONE.fill(Rgba::rgb(34, 34, 36)))
    .focus(Look::NONE.border(ACCENT, 1.0));

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Effect {
    pub target: Target,
    choice: Choice,
}

impl Effect {
    pub(crate) fn operation(&self, project: &Project) -> Option<Json> {
        let Choice { kind, mode } = self.choice;
        if !allowed(project, &self.target, self.choice) {
            return None;
        }
        let target = match self.target {
            Target::Timeline(id) => id.to_string(),
            Target::Lane(id) => id.to_string(),
            Target::Project => "project".into(),
            _ => return None,
        };
        let mut fields = vec![
            ("cmd".into(), Json::String("effect.add".into())),
            ("target".into(), Json::String(target)),
            ("type".into(), Json::String(kind.into())),
        ];
        if let Some(mode) = mode {
            fields.push((
                "params".into(),
                Json::Object(vec![("mode".into(), Json::String(mode.into()))]),
            ));
        }
        if kind == "transition"
            && let Target::Timeline(TimelineId::Clip(clip)) = self.target
        {
            let length = effect_schemas()
                .field(kind)?
                .field("params")?
                .field("length")?
                .field("def")?
                .as_i64()?;
            fields.push((
                "params".into(),
                Json::Object(vec![(
                    "length".into(),
                    Json::Signed(length.min(i64::from(transition_capacity(project, clip)))),
                )]),
            ));
        }
        Some(Json::Object(fields))
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct EffectTargets {
    pub selection: Option<Target>,
    pub lane: Option<LaneId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Choice {
    kind: &'static str,
    mode: Option<&'static str>,
}

impl Choice {
    const fn kind(kind: &'static str) -> Self {
        Self { kind, mode: None }
    }
}

fn allowed(project: &Project, target: &Target, choice: Choice) -> bool {
    effect_target_allowed(project, target, choice.kind)
        && (choice.kind != "transition"
            || matches!(target, Target::Timeline(TimelineId::Clip(id))
                if transition_capacity(project, *id) >= 2))
}

pub(super) fn carrying(ui: &Ui) -> bool {
    ui.carrying().is_some_and(|payload| payload.kind == CARRY)
}

// Targets use their own retained and clipped PXUI nodes. The drop can still be
// read after the pointer carry retires on the release frame.
pub(crate) fn drop_at(
    ui: &mut Ui,
    node: &Handle,
    project: &Project,
    target: Target,
) -> Option<Effect> {
    let active = match ui.carrying() {
        Some(payload) if payload.kind == CARRY => choice(payload.value),
        Some(_) => return None,
        None => None,
    };
    if active.is_some_and(|kind| !allowed(project, &target, kind)) {
        return None;
    }
    let mut hovered = false;
    let mut dropped = None;
    match ui.drop_target(node) {
        Some(DropRef::Hover(payload)) if payload.kind == CARRY => hovered = true,
        Some(DropRef::Dropped(payload)) if payload.kind == CARRY => {
            dropped = choice(payload.value).filter(|&kind| allowed(project, &target, kind));
        }
        _ => {}
    }
    if active.is_some() {
        ui.canvas_over(node, move |paint, rect| {
            if hovered {
                paint.fill(rect, Fill::new(ACCENT.alpha(34)));
            }
            let color = if hovered { ACCENT } else { ACCENT.alpha(135) };
            paint.frame(
                rect,
                Stroke::new(color).width(if hovered { 2.0 } else { 1.0 }),
            );
        });
    }
    dropped.map(|choice| Effect { target, choice })
}

#[derive(Clone, Copy)]
struct Row {
    key: &'static str,
    title: &'static str,
    detail: &'static str,
    glyph: Glyph,
    choice: Option<Choice>,
}

impl Row {
    const fn live(
        key: &'static str,
        title: &'static str,
        detail: &'static str,
        glyph: Glyph,
    ) -> Self {
        Self {
            key,
            title,
            detail,
            glyph,
            choice: Some(Choice::kind(key)),
        }
    }

    const fn preview(
        key: &'static str,
        title: &'static str,
        detail: &'static str,
        glyph: Glyph,
    ) -> Self {
        Self {
            key,
            title,
            detail,
            glyph,
            choice: None,
        }
    }

    const fn mask(mut self, mode: &'static str) -> Self {
        self.choice = Some(Choice {
            kind: "mask",
            mode: Some(mode),
        });
        self
    }
}

const GROUPS: &[(&str, &[Row])] = &[
    (
        "Frame",
        &[
            Row::live(
                "window",
                "Window",
                "Padding, background, rounded corners, shadow and inset around the picture (Recorder's Background + Frame). Applied after crop. The background stays put while zooms move the window.",
                Glyph::Window,
            ),
            Row::preview(
                "deviceFrame",
                "Device Frame",
                "MacBook or browser chrome around the window.",
                Glyph::Device,
            ),
            Row::preview(
                "backgroundBlur",
                "Background Blur",
                "Blurred copy of the content as background.",
                Glyph::Blur,
            ),
        ],
    ),
    (
        "Cursor",
        &[
            Row::live(
                "cursor",
                "Cursor",
                "Scales the recorded cursor, adds click rings or hides it. Without it the cursor renders as recorded.",
                Glyph::Pointer,
            ),
            Row::preview(
                "clickZoom",
                "Click Zoom",
                "Legacy effect. Use Auto Zoom for recorded clicks.",
                Glyph::Zoom,
            ),
            Row::preview(
                "motionBlur",
                "Motion Blur",
                "Blur on zoom, pan and cursor moves (Recorder Animation).",
                Glyph::Motion,
            ),
            Row::preview(
                "clickSound",
                "Click Sound",
                "Audio effect on recorded clicks.",
                Glyph::Sound,
            ),
        ],
    ),
    (
        "Camera",
        &[Row::live(
            "camera",
            "Camera Layout",
            "Places the camera bubble above the window, outside the padding. Without it the bubble uses the default corner.",
            Glyph::Camera,
        )],
    ),
    (
        "Captions",
        &[Row::live(
            "captions",
            "Caption Style",
            "How captions look and animate: preset, size, position, colors, stroke, safe zone. Set it on the Captions lane for the whole track. A cue overrides single values.",
            Glyph::Captions,
        )],
    ),
    (
        "Keystrokes",
        &[Row::live(
            "keystrokes",
            "Keystroke Chips",
            "Styles the chips drawn from recorded keystrokes. Hiding single keystrokes stays a clip edit (keys.hide).",
            Glyph::Keys,
        )],
    ),
    (
        "Privacy",
        &[
            Row::preview(
                "blurRegion",
                "Blur Region",
                "Blur a rectangular region of the picture.",
                Glyph::Blur,
            )
            .mask("blur"),
            Row::preview(
                "spotlight",
                "Spotlight",
                "Dim everything except a region.",
                Glyph::Spotlight,
            )
            .mask("spotlight"),
        ],
    ),
    (
        "Look",
        &[Row::live(
            "color",
            "Color",
            "Adjust exposure, contrast, saturation and color.",
            Glyph::Grade,
        )],
    ),
];

fn choice(key: &str) -> Option<Choice> {
    if let Some(row) = GROUPS
        .iter()
        .flat_map(|(_, rows)| *rows)
        .find(|row| row.key == key)
    {
        return row.choice;
    }
    effect_schemas()
        .as_object()?
        .iter()
        .find(|(kind, _)| kind == key)
        .map(|(kind, _)| Choice::kind(kind))
}

fn featured(kind: &str) -> bool {
    // Keep the general Mask in the native catalog in addition to its two presets.
    GROUPS
        .iter()
        .flat_map(|(_, rows)| *rows)
        .any(|row| row.key == kind)
}

pub(super) fn paint(
    ui: &mut Ui,
    project: Option<&Project>,
    targets: &EffectTargets,
) -> Option<Effect> {
    let mut effect = None;
    for &(group, rows) in GROUPS {
        heading(ui, group);
        for &row in rows {
            effect = paint_row(ui, row, project, targets).or(effect);
        }
    }
    heading(ui, "More effects");
    for (key, schema) in effect_schemas().as_object().unwrap_or_default() {
        if featured(key) {
            continue;
        }
        let row = Row::live(
            key,
            schema.field("title").and_then(Json::as_str).unwrap_or(key),
            schema
                .field("appliesTo")
                .and_then(Json::as_str)
                .unwrap_or(""),
            Glyph::Effects,
        );
        effect = paint_row(ui, row, project, targets).or(effect);
    }
    effect
}

fn heading(ui: &mut Ui, text: &str) {
    ui.column(
        Node::column()
            .w(Size::Grow)
            .h_px(28.0)
            .padding(Edges::new(8.0, 10.0, 8.0, 4.0)),
        |ui| text_color(ui, text, GROUP, QUIET),
    );
}

fn paint_row(
    ui: &mut Ui,
    row: Row,
    project: Option<&Project>,
    targets: &EffectTargets,
) -> Option<Effect> {
    let mut effect = None;
    let live = row
        .choice
        .filter(|value| effect_schemas().field(value.kind).is_some());
    let selection = targets.selection.as_ref();
    let lane = targets.lane.map(Target::Lane);
    let project_target = Target::Project;
    let valid = |target: Option<&Target>| {
        target.is_some_and(|target| {
            project
                .zip(live)
                .is_some_and(|(project, value)| allowed(project, target, value))
        })
    };
    let title_ink = if live.is_some() { INK } else { MUTED };
    ui.id(row.key, |ui| {
        ui.column(Node::column().w(Size::Grow), |ui| {
            ui.column(
                Node::column()
                    .w(Size::Grow)
                    .padding(Edges::new(8.0, 6.0, 8.0, 6.0))
                    .gap(6.0),
                |ui| {
                    let response = ui.pressable(
                        "effect-source",
                        Node::row()
                            .w(Size::Grow)
                            .min_h(26.0)
                            .gap(6.0)
                            .role(Role::Button),
                        &Pressable::DEFAULT
                            .look(&ROW)
                            .disabled(live.is_none() || project.is_none()),
                        |ui, _| {
                            ui.stack(Node::DEFAULT.w_px(18.0).h_px(26.0), |ui| {
                                let glyph = ui.node(
                                    "glyph",
                                    Node::DEFAULT.at(0.0, 0.0).w_px(26.0).h_px(26.0),
                                );
                                ui.canvas(&glyph, move |paint, rect| {
                                    paint_glyph(paint, rect, row.glyph, live.is_some())
                                });
                            });
                            ui.column(Node::column().w(Size::Grow), |ui| {
                                ui.row(
                                    Node::row()
                                        .w(Size::Grow)
                                        .center()
                                        .gap(4.0)
                                        .min_h(if live.is_some() { 16.8 } else { 18.0 }),
                                    |ui| {
                                        text_color(ui, row.title, NAME.w(Size::Fit), title_ink);
                                        if live.is_none() {
                                            let status = if row.key == "clickZoom" {
                                                "legacy"
                                            } else {
                                                "planned"
                                            };
                                            text_color(ui, status, CAPTION, QUIET);
                                        }
                                    },
                                );
                                text_color(ui, row.detail, DETAIL, QUIET);
                            });
                        },
                    );
                    ui.a11y_name(&response.handle, row.title);
                    if let Some(choice) = live {
                        let handle = response.handle;
                        let signal = ui.signal(&handle);
                        let primary = signal.button == Some(Signal::BUTTON_PRIMARY);
                        // PXUI reports a double press before release. Require a completed
                        // first click, so starting a second drag can't apply to Selection.
                        let double = if signal.pressed && primary {
                            let next = signal.double_clicked && ui.state(&handle, 0) == 1;
                            ui.set_state(&handle, if next { 2 } else { 0 });
                            next
                        } else {
                            response.clicked && ui.state(&handle, 0) == 2
                        };
                        if signal.held && primary {
                            let was_carrying = carrying(ui);
                            ui.carry(Some(&handle), CARRY, row.key);
                            if !was_carrying && carrying(ui) {
                                ui.set_state(&handle, 0);
                                ui.request_frame();
                            }
                        }
                        if response.clicked {
                            ui.set_state(&handle, i64::from(!response.activated));
                            if (double || response.activated) && valid(selection) {
                                effect = selection.cloned().map(|target| Effect { target, choice });
                            }
                        }
                        ui.row(
                            Node::row()
                                .w(Size::Grow)
                                .gap(2.0)
                                .padding(Edges::new(24.0, 0.0, 0.0, 0.0)),
                            |ui| {
                                for (title, width, target) in [
                                    ("Selection", 68.0, selection),
                                    ("Lane", 38.0, lane.as_ref()),
                                    ("Project", 56.0, Some(&project_target)),
                                ] {
                                    if ui
                                        .button(
                                            title,
                                            title,
                                            &Button::sized(width, 20.0)
                                                .label(CAPTION.w(Size::Fit))
                                                .look(&BUTTON)
                                                .disabled(!valid(target)),
                                        )
                                        .clicked
                                    {
                                        effect =
                                            target.cloned().map(|target| Effect { target, choice });
                                    }
                                }
                            },
                        );
                    }
                },
            );
            ui.node(
                "effect-rule",
                Node::DEFAULT
                    .w(Size::Grow)
                    .h_px(1.0)
                    .look(Look::NONE.fill(LINE)),
            );
        });
    });
    effect
}

fn text_color(ui: &mut Ui, text: &str, label: Label, ink: Rgba) {
    let handle = ui.label(text, label);
    ui.style(&handle, &States::of(Look::NONE.text(ink)), State::default());
}

#[derive(Clone, Copy)]
enum Glyph {
    Window,
    Device,
    Blur,
    Pointer,
    Zoom,
    Motion,
    Sound,
    Camera,
    Captions,
    Keys,
    Spotlight,
    Grade,
    Effects,
}

fn paint_glyph(paint: &mut Paint<'_>, rect: Rect, glyph: Glyph, live: bool) {
    let ink = if live { ACCENT } else { ACCENT.alpha(130) };
    paint.frame(rect, Stroke::new(LINE));
    if matches!(glyph, Glyph::Effects | Glyph::Sound) {
        asset_icon(
            paint,
            rect,
            ink,
            if matches!(glyph, Glyph::Sound) { 1 } else { 4 },
        );
        return;
    }
    let units = 16.0 / 24.0;
    let p = |(x, y): (f32, f32)| (rect.x + 5.0 + x * units, rect.y + 5.0 + y * units);
    let line = Stroke::new(ink);
    let path = |paint: &mut Paint<'_>, points: &[(f32, f32)]| {
        for pair in points.windows(2) {
            paint.line(p(pair[0]), p(pair[1]), line);
        }
    };
    let box_at = |paint: &mut Paint<'_>, x, y, w, h| {
        let (x, y) = p((x, y));
        paint.frame(Rect::new(x, y, w * units, h * units), line);
    };
    let circle = |paint: &mut Paint<'_>, x, y, r| {
        paint.circle(p((x, y)), r * units, None, Some(line));
    };
    match glyph {
        Glyph::Window => {
            box_at(paint, 3.0, 4.0, 18.0, 16.0);
            box_at(paint, 7.0, 8.0, 10.0, 8.0);
        }
        Glyph::Device => {
            box_at(paint, 5.0, 5.0, 14.0, 10.0);
            path(paint, &[(3.0, 19.0), (21.0, 19.0)]);
        }
        Glyph::Blur => {
            box_at(paint, 3.0, 5.0, 18.0, 14.0);
            circle(paint, 12.0, 12.0, 2.5);
        }
        Glyph::Pointer | Glyph::Zoom => {
            path(
                paint,
                &[
                    (4.0, 4.0),
                    (4.0, 18.0),
                    (8.0, 14.0),
                    (11.0, 20.0),
                    (14.0, 18.0),
                    (11.0, 12.0),
                    (16.0, 12.0),
                    (4.0, 4.0),
                ],
            );
            if matches!(glyph, Glyph::Zoom) {
                circle(paint, 17.0, 16.0, 4.5);
            }
        }
        Glyph::Motion => {
            for y in [8.0, 12.0, 16.0] {
                path(paint, &[(3.0, y), (8.0, y)]);
            }
            circle(paint, 15.5, 12.0, 5.0);
        }
        Glyph::Camera => {
            box_at(paint, 3.0, 4.0, 18.0, 16.0);
            circle(paint, 15.5, 14.5, 3.0);
        }
        Glyph::Captions => {
            box_at(paint, 3.0, 5.0, 18.0, 14.0);
            for (x, w) in [(6.0, 4.0), (13.0, 5.0)] {
                path(paint, &[(x, 14.0), (x + w, 14.0)]);
            }
        }
        Glyph::Keys => {
            for x in [3.0, 13.0] {
                box_at(paint, x, 7.0, 8.0, 10.0);
                path(paint, &[(x + 3.0, 12.0), (x + 5.0, 12.0)]);
            }
        }
        Glyph::Spotlight => {
            circle(paint, 12.0, 12.0, 4.0);
            for (a, b) in [
                ((12.0, 3.0), (12.0, 6.0)),
                ((12.0, 18.0), (12.0, 21.0)),
                ((3.0, 12.0), (6.0, 12.0)),
                ((18.0, 12.0), (21.0, 12.0)),
            ] {
                path(paint, &[a, b]);
            }
        }
        Glyph::Grade => {
            circle(paint, 12.0, 12.0, 8.5);
            path(paint, &[(12.0, 3.5), (12.0, 20.5)]);
            for y in [7.5, 12.0, 16.5] {
                path(paint, &[(12.0, y), (17.0, y)]);
            }
        }
        Glyph::Effects | Glyph::Sound => {}
    }
}
