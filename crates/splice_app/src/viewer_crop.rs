use pxui::{
    A11yStates, Button, Fill, Image, ImageStyle, KeyCode, Label, Mods, Node, Rect, Role, Signal,
    Stroke, ThemeHandle, Ui, ValueCallbacks, ValueField,
    geom::Rgba,
    theme::ColorToken,
    ui::{Align, Flags, Look, Size, States},
};
use splice_core::history::StoreError;
use splice_json::Value as Json;
use splice_model::{
    Fit, Framing, ItemKind, MediaId, MediaKind, Project, Rect as Crop, TimelineId, timing,
};
use std::path::Path;

const FULL: Crop = Crop {
    x: 0.0,
    y: 0.0,
    w: 1.0,
    h: 1.0,
};
const MIN: f64 = 0.05;
const BG: Rgba = Rgba::rgb(9, 9, 10);
const MATTE: Rgba = Rgba::rgb(6, 6, 7);
const STAGE: Rgba = Rgba::rgb(22, 22, 24);
const LINE: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const BODY: Label = Label::MONO.size(11.0);
const CAPTION: Label = Label::CAPTION.font(1).weight(300).size(10.0);
const HANDLE: f32 = 10.0;
const READOUT_HEIGHT: f32 = 16.0;
const READOUT_PAD: f32 = 5.0;
const READOUT_INK: ColorToken = ColorToken(0);
const READOUT: Label = Label::MONO
    .size(10.0)
    .line_height(READOUT_HEIGHT)
    .color(READOUT_INK);

thread_local! {
    static READOUT_THEME: ThemeHandle = {
        let mut theme = ThemeHandle::default();
        theme.set_color(READOUT_INK, BG);
        theme
    };
}
const CONTROL: Look = Look::NONE.text(MUTED).border(LINE, 1.0);
const BUTTON: States = States::of(CONTROL)
    .hover(CONTROL.fill(Rgba::rgb(34, 34, 36)).text(INK))
    .checked(CONTROL.fill(Rgba::rgb(42, 42, 45)).text(INK));

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Draft {
    id: TimelineId,
    size: [u32; 2],
    output: f64,
    rect: Crop,
    fit: Fit,
    lock: usize,
    drag: Option<Drag>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    edge: [i32; 2],
    start: Crop,
}

pub(super) enum Action {
    None,
    Update(Draft),
    Apply(Json),
    Cancel,
}

const LOCKS: [(&str, f64); 7] = [
    ("Free", 0.0),
    ("Output", 0.0),
    ("16:9", 16.0 / 9.0),
    ("4:3", 4.0 / 3.0),
    ("1:1", 1.0),
    ("4:5", 4.0 / 5.0),
    ("9:16", 9.0 / 16.0),
];

impl Draft {
    pub(super) fn new(project: &Project, id: TimelineId) -> Option<Self> {
        let (media_id, framing, _) = target_data(project, id)?;
        let media = project.media.iter().find(|media| media.id == media_id)?;
        let size = [media.probe.width, media.probe.height];
        if matches!(media.kind, MediaKind::Audio | MediaKind::Compound) || size.contains(&0) {
            return None;
        }
        Some(Self {
            id,
            size,
            output: project.aspect.ratio(),
            rect: framing.crop.unwrap_or(FULL),
            fit: framing.fit,
            lock: 0,
            drag: None,
        })
    }

    pub(super) fn target(&self) -> TimelineId {
        self.id
    }

    pub(super) fn name(self, project: &Project) -> Option<&str> {
        let (id, _, name) = target_data(project, self.id)?;
        name.filter(|name| !name.is_empty()).or_else(|| {
            let media = project.media.iter().find(|media| media.id == id)?;
            super::viewer_drawers::media_title(media)
        })
    }

    pub(super) fn preview(
        &self,
        project: &Project,
        package: &Path,
        at: i32,
    ) -> Result<(Vec<u8>, [u32; 2]), StoreError> {
        let unavailable = || preview_error("This item has no standalone visual source.");
        let (id, _, _) = target_data(project, self.id).ok_or_else(unavailable)?;
        let media = project
            .media
            .iter()
            .find(|media| media.id == id)
            .ok_or_else(unavailable)?;
        if matches!(media.kind, MediaKind::Audio | MediaKind::Compound) {
            return Err(unavailable());
        }
        let source = package.join(&media.path);
        let path = match media.take.as_ref().and_then(|take| take.screen.as_deref()) {
            Some(file) if source.is_dir() => source.join(file),
            Some(file) => source.with_file_name(file),
            None => source,
        };
        let decoded = if media.kind == MediaKind::Image {
            apple_sys::image::decode_rgba(&path)
        } else {
            let source = source_frame(project, self.id, at, media.probe.fps).ok_or_else(|| {
                preview_error("The source frame could not be mapped from the playhead.")
            })?;
            if media.probe.frames <= 0 {
                return Err(preview_error("The media has no source frames."));
            }
            let source = source.clamp(0.0, f64::from(media.probe.frames - 1));
            apple_sys::media::frame_at(&path, source / media.probe.fps)
        };
        decoded.map_err(|error| preview_error(&error.to_string()))
    }
    fn ratio(self) -> Option<f64> {
        let ratio = if self.lock == 1 {
            self.output
        } else {
            LOCKS[self.lock].1
        };
        let normalized = ratio * f64::from(self.size[1]) / f64::from(self.size[0]);
        (MIN..=1.0 / MIN)
            .contains(&normalized)
            .then_some(normalized)
    }

    fn command(self) -> Json {
        let rect = self.rect;
        let full = rect.x <= 0.001 && rect.y <= 0.001 && rect.w >= 0.999 && rect.h >= 0.999;
        // Round the endpoints first so serialization preserves the bounds and minimum size.
        let edge = |value: f64| (value * 10_000.0).round() as i32;
        let left = edge(rect.x).clamp(0, 9500);
        let top = edge(rect.y).clamp(0, 9500);
        let width = edge(rect.x + rect.w).clamp(left + 500, 10_000) - left;
        let height = edge(rect.y + rect.h).clamp(top + 500, 10_000) - top;
        let units = |value| Json::Float(f64::from(value) / 10_000.0);
        Json::Object(vec![
            ("cmd".into(), Json::String("clip.crop".into())),
            (
                "items".into(),
                Json::Array(vec![Json::String(self.id.to_string())]),
            ),
            (
                "fit".into(),
                Json::String(if self.fit == Fit::Fit { "fit" } else { "fill" }.into()),
            ),
            (
                "rect".into(),
                if full {
                    Json::Null
                } else {
                    Json::Object(vec![
                        ("x".into(), units(left)),
                        ("y".into(), units(top)),
                        ("w".into(), units(width)),
                        ("h".into(), units(height)),
                    ])
                },
            ),
        ])
    }
}

fn target_data(project: &Project, id: TimelineId) -> Option<(MediaId, Framing, Option<&str>)> {
    match id {
        TimelineId::Clip(id) => {
            let clip = project.clips.data(project.clips.find(id)?)?;
            Some((clip.media?, clip.framing, clip.appearance.name.as_deref()))
        }
        TimelineId::Item(id) => {
            let item = project.items.data(project.items.find(id)?)?;
            let ItemKind::Media(overlay) = &item.kind else {
                return None;
            };
            Some((
                overlay.media,
                overlay.framing,
                item.appearance.name.as_deref(),
            ))
        }
        TimelineId::Marker(_) => None,
    }
}

fn source_frame(project: &Project, id: TimelineId, at: i32, source_fps: f64) -> Option<f64> {
    if !source_fps.is_finite() || source_fps <= 0.0 || project.fps == 0 {
        return None;
    }
    match id {
        TimelineId::Clip(id) => {
            let slot = project.clips.find(id)?;
            let clip = project.clips.timing(slot)?;
            let local = at
                .saturating_sub(clip.start)
                .clamp(0, clip.length.saturating_sub(1).max(0));
            Some(f64::from(clip.source) + timing::consumed(project, slot, f64::from(local)).ok()?)
        }
        TimelineId::Item(id) => {
            let slot = project.items.find(id)?;
            let item = project.items.data(slot)?;
            let ItemKind::Media(overlay) = &item.kind else {
                return None;
            };
            let timing = project.items.timing(slot)?;
            let start = timing::anchor_start(project, timing.anchor).ok()?;
            let local = at
                .saturating_sub(start)
                .clamp(0, timing.length.saturating_sub(1).max(0));
            Some(
                f64::from(overlay.source)
                    + f64::from(local) * (source_fps / f64::from(project.fps)),
            )
        }
        TimelineId::Marker(_) => None,
    }
}
fn preview_error(message: &str) -> StoreError {
    StoreError::new(
        "CROP_PREVIEW",
        message,
        "Relink the media or choose another clip to preview its source.",
    )
}

pub(super) fn paint(
    ui: &mut Ui,
    draft: Draft,
    name: &str,
    preview: Option<Image>,
    [width, height]: [f32; 2],
) -> Action {
    let mut next = draft;
    let mut apply = false;
    let mut cancel = false;
    let editing = ui.text_input_focused();
    let combined = width >= 680.0;
    let title_inline = width >= 390.0;
    let numeric_fields = height >= 260.0;
    let header = if combined {
        34.0
    } else if title_inline {
        58.0
    } else {
        80.0
    };
    let guide_height = (height - header - if numeric_fields { 46.0 } else { 0.0 }).max(0.0);
    let toolbar = Node::DEFAULT
        .w(Size::Grow)
        .center()
        .pad_x(8.0)
        .gap(4.0)
        .look(Look::NONE.fill(Rgba::rgb(20, 20, 21)));
    ui.column(
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(height.max(0.0))
            .flags(Flags::CLIP)
            .look(Look::NONE.fill(MATTE)),
        |ui| {
            ui.row(toolbar.h_px(34.0), |ui| {
                if title_inline {
                    crop_title(ui, draft, name);
                } else {
                    ui.node("crop-spacer", Node::DEFAULT.w(Size::Grow));
                }
                if combined {
                    locks(ui, &mut next);
                }
                (apply, cancel) = controls(ui, &mut next);
            });
            if !title_inline {
                ui.row(toolbar.h_px(22.0), |ui| crop_title(ui, draft, name));
            }
            if !combined {
                ui.row(toolbar.h_px(24.0), |ui| locks(ui, &mut next));
            }
            if guide_height > 0.0 {
                guide(ui, &mut next, preview, [width, guide_height]);
            }
            if numeric_fields {
                fields(ui, &mut next);
            }
        },
    );
    if !editing && !ui.text_input_focused() {
        cancel |= ui.key_pressed(KeyCode::Escape, Mods::NONE);
        apply |= ui.key_pressed(KeyCode::Enter, Mods::NONE);
        let dx = nudge(ui, KeyCode::ArrowRight) - nudge(ui, KeyCode::ArrowLeft);
        let dy = nudge(ui, KeyCode::ArrowDown) - nudge(ui, KeyCode::ArrowUp);
        next.rect.x =
            (next.rect.x + dx / f64::from(draft.size[0])).clamp(0.0, (1.0 - next.rect.w).max(0.0));
        next.rect.y =
            (next.rect.y + dy / f64::from(draft.size[1])).clamp(0.0, (1.0 - next.rect.h).max(0.0));
    }
    if cancel {
        Action::Cancel
    } else if apply {
        Action::Apply(next.command())
    } else if next != draft {
        Action::Update(next)
    } else {
        Action::None
    }
}

fn nudge(ui: &Ui, key: KeyCode) -> f64 {
    if ui.key_pressed(key, Mods::NONE) {
        1.0
    } else if ui.key_pressed(key, Mods::SHIFT)
        // PXUI matches a nonempty mask as a subset of the modifiers at the press.
        && ![Mods::CTRL, Mods::ALT, Mods::META]
            .into_iter()
            .any(|other| ui.key_pressed(key, Mods::SHIFT.with(other)))
    {
        10.0
    } else {
        0.0
    }
}

fn crop_title(ui: &mut Ui, draft: Draft, name: &str) {
    let [width, height] = draft.size;
    // Leave space for the dimensions in PXUI's 128-byte formatted label.
    let name = &name[..name.floor_char_boundary(80)];
    ui.label_fmt(
        format_args!("Crop · {name} · source {width}×{height}"),
        CAPTION.grow().ellipsis_middle(),
    );
}

fn locks(ui: &mut Ui, draft: &mut Draft) {
    ui.row(Node::DEFAULT.w(Size::Fit).gap(2.0), |ui| {
        for (index, (label, _)) in LOCKS.into_iter().enumerate() {
            let mut lock = *draft;
            lock.lock = index;
            let button = Button::sized(if index == 1 { 48.0 } else { 35.0 }, 22.0)
                .look(&BUTTON)
                .label(BODY)
                .selected(index == draft.lock)
                .disabled(index != 0 && lock.ratio().is_none());
            if ui.button(label, label, &button).clicked {
                draft.lock = index;
                if let Some(ratio) = draft.ratio() {
                    draft.rect = conform(draft.rect, ratio);
                }
            }
        }
    });
}

fn controls(ui: &mut Ui, draft: &mut Draft) -> (bool, bool) {
    let mut apply = false;
    let mut cancel = false;
    ui.row(Node::DEFAULT.w(Size::Fit).gap(4.0).center(), |ui| {
        for (label, fit) in [("Fit", Fit::Fit), ("Fill", Fit::Fill)] {
            let button = Button::sized(42.0, 22.0)
                .look(&BUTTON)
                .label(BODY)
                .selected(draft.fit == fit);
            if ui.button(label, label, &button).clicked {
                draft.fit = fit;
            }
        }
        if ui
            .button(
                "reset",
                "Reset",
                &Button::sized(46.0, 22.0).look(&BUTTON).label(BODY),
            )
            .clicked
        {
            draft.rect = FULL;
            draft.lock = 0;
        }
        cancel = ui
            .button(
                "cancel",
                "Cancel",
                &Button::sized(50.0, 22.0).look(&BUTTON).label(BODY),
            )
            .clicked;
        apply = ui
            .button(
                "apply",
                "Apply ↵",
                &Button::sized(58.0, 22.0).look(&BUTTON).label(BODY),
            )
            .clicked;
    });
    (apply, cancel)
}

fn fields(ui: &mut Ui, draft: &mut Draft) {
    ui.row(
        Node::DEFAULT.w(Size::Grow).h_px(46.0).pad_x(8.0).gap(6.0),
        |ui| {
            for (index, label) in ["X", "Y", "Width", "Height"].into_iter().enumerate() {
                let size = draft.size[index % 2];
                let scale = f64::from(size);
                let rect = draft.rect;
                let old = [rect.x, rect.y, rect.w, rect.h][index];
                let Some(value) =
                    pixel_field(ui, label, name(label), (old * scale).round() as u32, size)
                else {
                    continue;
                };
                let value = f64::from(value) / scale;
                draft.rect = match index {
                    0 => Crop {
                        x: value.clamp(0.0, 1.0 - rect.w),
                        ..rect
                    },
                    1 => Crop {
                        y: value.clamp(0.0, 1.0 - rect.h),
                        ..rect
                    },
                    2 => Crop {
                        w: value.clamp(MIN, (1.0 - rect.x).max(MIN)),
                        ..rect
                    },
                    _ => Crop {
                        h: value.clamp(MIN, (1.0 - rect.y).max(MIN)),
                        ..rect
                    },
                };
                if index >= 2
                    && let Some(ratio) = draft.ratio()
                {
                    if index == 2 {
                        draft.rect.h = draft.rect.w / ratio;
                    } else {
                        draft.rect.w = draft.rect.h * ratio;
                    }
                    draft.rect = conform(draft.rect, ratio);
                }
            }
        },
    );
}
fn name(label: &str) -> &'static str {
    match label {
        "X" => "Crop left in pixels",
        "Y" => "Crop top in pixels",
        "Width" => "Crop width in pixels",
        _ => "Crop height in pixels",
    }
}

fn pixel_field(
    ui: &mut Ui,
    label: &'static str,
    accessible: &'static str,
    value: u32,
    max: u32,
) -> Option<u32> {
    use std::io::Write;
    let mut bytes = [0; 10];
    let mut buffer = bytes.as_mut_slice();
    write!(buffer, "{value}").expect("a u32 fits 10 bytes");
    let len = 10 - buffer.len();
    let text = std::str::from_utf8(&bytes[..len]).expect("decimal pixels");
    let mut change = None;
    ui.column(Node::DEFAULT.w_px(65.0).h_px(40.0), |ui| {
        ui.label(label, BODY);
        let valid = |text: &str| text.parse::<u32>().is_ok_and(|value| value <= max);
        let style = ValueField::DEFAULT.size(11).name(accessible);
        let input = pxui::ext::value_field(
            ui,
            label,
            text,
            Rect::new(0.0, 17.0, 65.0, 22.0),
            &style,
            &ValueCallbacks::DEFAULT.valid(&valid),
        );
        change = input
            .changed
            .and_then(|text| text.parse().ok())
            .filter(|&next| next != value);
        let handle = input.response.handle;
        ui.set_role(&handle, Role::TextField, A11yStates::EDITABLE);
    });
    change
}

fn guide(ui: &mut Ui, draft: &mut Draft, preview: Option<Image>, [width, height]: [f32; 2]) {
    let [w, h] = draft.size.map(|size| size as f32);
    let scale = ((width - 32.0).max(1.0) / w).min((height - 24.0).max(1.0) / h);
    let [w, h] = [w * scale, h * scale];
    ui.row(
        Node {
            w: Size::Grow,
            h: Size::Px(height),
            flags: Flags::CLIP,
            align: Align::Center,
            justify: pxui::ui::Justify::Center,
            look: Look::NONE.fill(MATTE),
            ..Node::DEFAULT
        },
        |ui| {
            let guide = ui.node("crop-guide", Node::DEFAULT.w_px(w).h_px(h));
            let shown = *draft;
            let rect = shown.rect;
            ui.canvas(&guide, move |paint, r| {
                paint.fill(r, Fill::new(STAGE));
                if let Some(image) = preview {
                    paint
                        .image(r, &image, &ImageStyle::DEFAULT.fit(pxui::draw::Fit::Fill))
                        .expect("live source preview texture");
                }
                let dim = Fill::new(Rgba::rgb(0, 0, 0).with_alpha(158));
                let b = Rect::new(
                    r.x + r.w * rect.x as f32,
                    r.y + r.h * rect.y as f32,
                    r.w * rect.w as f32,
                    r.h * rect.h as f32,
                );
                paint.fill(Rect::new(r.x, r.y, r.w, b.y - r.y), dim);
                paint.fill(
                    Rect::new(r.x, b.y + b.h, r.w, (r.y + r.h - b.y - b.h).max(0.0)),
                    dim,
                );
                paint.fill(Rect::new(r.x, b.y, b.x - r.x, b.h), dim);
                paint.fill(
                    Rect::new(b.x + b.w, b.y, (r.x + r.w - b.x - b.w).max(0.0), b.h),
                    dim,
                );
                paint.stroke(b, Stroke::new(INK));
                for f in [1.0 / 3.0, 2.0 / 3.0] {
                    paint.fill(Rect::new(b.x + b.w * f, b.y, 1.0, b.h), Fill::new(LINE));
                    paint.fill(Rect::new(b.x, b.y + b.h * f, b.w, 1.0), Fill::new(LINE));
                }
            });
            let preserve = ui.key_down(KeyCode::Shift);
            ui.within(&guide, &mut |ui| {
                let x = rect.x as f32 * w;
                let y = rect.y as f32 * h;
                let width = rect.w as f32 * w;
                let height = rect.h as f32 * h;
                let flags = Flags::CLICKABLE | Flags::DRAG_FREE;
                let move_box = ui.node(
                    "crop-move",
                    Node::DEFAULT.at(x, y).w_px(width).h_px(height).flags(flags),
                );
                motion(draft, ui.signal(&move_box), [0, 0], [w, h], preserve);
                for (index, edge) in [
                    [-1, -1],
                    [0, -1],
                    [1, -1],
                    [-1, 0],
                    [1, 0],
                    [-1, 1],
                    [0, 1],
                    [1, 1],
                ]
                .into_iter()
                .enumerate()
                {
                    let [a, b] = edge;
                    let handle = ui.node(
                        index as u64,
                        Node::DEFAULT
                            .at(
                                x + (a + 1) as f32 * width * 0.5 - HANDLE * 0.5,
                                y + (b + 1) as f32 * height * 0.5 - HANDLE * 0.5,
                            )
                            .w_px(HANDLE)
                            .h_px(HANDLE)
                            .flags(flags)
                            .look(Look::NONE.fill(INK).border(BG, 1.0)),
                    );
                    motion(draft, ui.signal(&handle), edge, [w, h], preserve);
                }
            });
            readout(
                ui,
                shown,
                [
                    (width - w) * 0.5 + rect.x as f32 * w,
                    (height - h) * 0.5 + rect.y as f32 * h,
                ],
                [width, height],
            );
        },
    );
}

fn readout(ui: &mut Ui, draft: Draft, [left, top]: [f32; 2], [width, height]: [f32; 2]) {
    let rect = draft.rect;
    let [source_w, source_h] = draft.size.map(f64::from);
    let [w, h, x, y] = [
        rect.w * source_w,
        rect.h * source_h,
        rect.x * source_w,
        rect.y * source_h,
    ]
    .map(|pixels| pixels.round() as u32);
    let digits: u32 = [w, h, x, y]
        .into_iter()
        .map(|value| value.checked_ilog10().unwrap_or(0) + 1)
        .sum();
    READOUT_THEME.with(|theme| {
        ui.themed(theme, &mut |ui| {
            let mut text = ui.text_service();
            let mut advance = |ch| text.width(ch, READOUT, None).expect("crop caption font");
            // label_digits draws individual glyphs, with every digit on the advance of zero.
            let label_width = digits as f32 * advance("0")
                + ["×", " ", "p", "x", " ", "a", "t", " ", ",", " "]
                    .into_iter()
                    .map(advance)
                    .sum::<f32>();
            let badge_width = (label_width.ceil() + 2.0 * READOUT_PAD).min(width);
            let above = top - READOUT_HEIGHT - 4.0;
            // Below the top handles when the toolbar leaves too little room above the crop.
            let badge_top = if above < 0.0 {
                top + HANDLE * 0.5 + 4.0
            } else {
                above
            };
            let badge = Node::DEFAULT
                .at(
                    left.clamp(0.0, (width - badge_width).max(0.0)),
                    badge_top.clamp(0.0, (height - READOUT_HEIGHT).max(0.0)),
                )
                .w_px(badge_width)
                .h_px(READOUT_HEIGHT)
                .pad_x(READOUT_PAD)
                .flags(Flags::CLIP)
                .look(Look::NONE.fill(INK));
            ui.row(badge, |ui| {
                ui.label_digits(format_args!("{w}×{h} px at {x}, {y}"), READOUT);
            });
        });
    });
}

fn motion(draft: &mut Draft, signal: Signal, edge: [i32; 2], [w, h]: [f32; 2], preserve: bool) {
    if signal.pressed && signal.button == Some(Signal::BUTTON_PRIMARY) {
        draft.drag = Some(Drag {
            edge,
            start: draft.rect,
        });
    }
    if let Some(drag) = draft.drag.filter(|drag| drag.edge == edge) {
        if signal.held || signal.released {
            let point = if signal.released {
                signal.release_point
            } else {
                signal.pointer
            };
            let dx = f64::from((point.0 - signal.press_point.0) / w);
            let dy = f64::from((point.1 - signal.press_point.1) / h);
            let start = drag.start;
            draft.rect = resize(
                start,
                edge,
                dx,
                dy,
                draft
                    .ratio()
                    .or_else(|| preserve.then_some((start.w / start.h).clamp(MIN, 1.0 / MIN))),
            );
        }
        if !signal.held {
            draft.drag = None;
        }
    }
}

fn resize(
    start: Crop,
    [horizontal, vertical]: [i32; 2],
    dx: f64,
    dy: f64,
    ratio: Option<f64>,
) -> Crop {
    if horizontal == 0 && vertical == 0 {
        return Crop {
            x: (start.x + dx).clamp(0.0, (1.0 - start.w).max(0.0)),
            y: (start.y + dy).clamp(0.0, (1.0 - start.h).max(0.0)),
            ..start
        };
    }
    let (mut x, mut y, mut r, mut b) = (start.x, start.y, start.x + start.w, start.y + start.h);
    if horizontal < 0 {
        x = (x + dx).clamp(0.0, (r - MIN).max(0.0));
    }
    if horizontal > 0 {
        r = (r + dx).clamp((x + MIN).min(1.0), 1.0);
    }
    if vertical < 0 {
        y = (y + dy).clamp(0.0, (b - MIN).max(0.0));
    }
    if vertical > 0 {
        b = (b + dy).clamp((y + MIN).min(1.0), 1.0);
    }
    let mut rect = Crop {
        x,
        y,
        w: r - x,
        h: b - y,
    };
    if let Some(ratio) = ratio {
        if horizontal == 0 {
            rect.w = rect.h * ratio;
            rect.x = start.x + (start.w - rect.w) * 0.5;
        } else {
            rect.h = rect.w / ratio;
            if vertical < 0 {
                rect.y = b - rect.h;
            }
            if vertical == 0 {
                rect.y = start.y + (start.h - rect.h) * 0.5;
            }
        }
        rect = conform(rect, ratio);
    }
    rect
}

fn conform(rect: Crop, ratio: f64) -> Crop {
    let w = rect
        .w
        .min(rect.h * ratio)
        .clamp(MIN.max(MIN * ratio), 1.0_f64.min(ratio));
    let h = w / ratio;
    Crop {
        x: (rect.x + (rect.w - w) * 0.5).clamp(0.0, (1.0 - w).max(0.0)),
        y: (rect.y + (rect.h - h) * 0.5).clamp(0.0, (1.0 - h).max(0.0)),
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_for_crop_ui() -> Draft {
        Draft {
            id: TimelineId::Clip(splice_model::ClipId::from_serial(7)),
            size: [1920, 1080],
            output: 9.0 / 16.0,
            rect: Crop {
                x: 0.25,
                y: 0.25,
                w: 0.5,
                h: 0.5,
            },
            fit: Fit::Fit,
            lock: 0,
            drag: None,
        }
    }

    #[test]
    fn crop_ui_readout_and_handles_fit_desktop_and_stacked_stages() {
        use pxui::{FrameIn, testing};
        use splice_testkit::alloc::Meter;

        mem_sys::set_peer(pxui::library_counter);
        for [width, height] in [[691.0, 177.0], [480.0, 300.0]] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let frame = FrameIn::new(width, height);
            let draft = draft_for_crop_ui();
            {
                let mut idle = || {
                    ui.frame(&frame, &mut |ui| {
                        assert!(matches!(
                            paint(ui, draft, "Screen take", None, [width, height]),
                            Action::None
                        ));
                    })
                    .unwrap();
                };
                for _ in 0..8 {
                    idle();
                }
                Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(&mut idle);
            }

            let title = "Crop · Screen take · source 1920×1080";
            let title_box = testing::find(&ui, Role::Label, title).unwrap();
            let title_width = ui.text_service().width(title, CAPTION, None).unwrap();
            eprintln!("stage {width}×{height}: title box {title_box:?}, text width {title_width}");
            assert!(title_box.w >= title_width && title_box.x + title_box.w <= width);
            assert!(
                testing::find(&ui, Role::TextField, "Crop width in pixels")
                    .is_none_or(|_| height >= 260.0)
            );
            let apply = testing::find(&ui, Role::Button, "Apply ↵").unwrap();
            assert!(apply.x + apply.w <= width, "{apply:?}");
            assert!(
                ui.a11y_nodes()
                    .iter()
                    .any(|node| { ui.a11y_value_of(node) == Some("960×540 px at 480, 270") })
            );
            let ink = [INK.r, INK.g, INK.b, INK.a];
            let mut handles = Vec::new();
            let mut badge = None;
            for item in ui.layers().flat_map(|layer| layer.instances) {
                if item.color != ink {
                    continue;
                }
                let [x, y, right, bottom] = item.rect;
                if right - x == 10.0 && bottom - y == 10.0 {
                    handles.push(item.rect);
                } else if right - x > 40.0 && bottom - y == 16.0 {
                    badge = Some(item.rect);
                }
            }
            assert_eq!(handles.len(), 8, "{handles:?}");
            let [x, y, right, bottom] =
                badge.expect("pixel readout is painted with an ink background");
            let [handle_x, handle_y, ..] = handles[0];
            assert!((x - handle_x - HANDLE * 0.5).abs() <= 1.0);
            assert!((bottom - handle_y - 1.0).abs() <= 1.0);
            assert!(x >= 0.0 && y >= 0.0 && right <= width && bottom <= height);
            eprintln!(
                "stage {width}×{height}: badge {:?}, NW handle {:?}",
                badge.unwrap(),
                handles[0]
            );

            let edge = Draft {
                size: [7680, 1080],
                rect: Crop {
                    x: 0.95,
                    y: 0.0,
                    w: MIN,
                    h: 0.5,
                },
                ..draft
            };
            ui.frame(&frame, &mut |ui| {
                paint(ui, edge, "Screen take", None, [width, height]);
            })
            .unwrap();
            let [x, y, right, bottom] = ui
                .layers()
                .flat_map(|layer| layer.instances)
                .find(|item| {
                    item.color == ink
                        && item.rect[2] - item.rect[0] > 40.0
                        && item.rect[3] - item.rect[1] == 16.0
                })
                .expect("edge readout is painted")
                .rect;
            assert!(x >= 0.0 && y >= 34.0 && right == width && bottom <= height);
            eprintln!(
                "stage {width}×{height}: edge badge {:?}",
                [x, y, right, bottom]
            );

            if height < 260.0 {
                let second = Draft {
                    rect: Crop {
                        x: 0.27,
                        ..draft.rect
                    },
                    ..draft
                };
                let mut changing = || {
                    for crop in [draft, second] {
                        ui.frame(&frame, &mut |ui| {
                            assert!(matches!(
                                paint(ui, crop, "Screen take", None, [width, height]),
                                Action::None
                            ));
                        })
                        .unwrap();
                    }
                };
                for _ in 0..8 {
                    changing();
                }
                Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(&mut changing);
            }
        }
    }

    #[test]
    fn crop_ui_modifiers_and_mouse_only_change_the_draft_until_apply() {
        use pxui::{Event, FrameIn};

        let mut ui = crate::viewer_ui().unwrap();
        let draft = draft_for_crop_ui();
        let stage = [691.0, 177.0];
        // PXUI's public Event ABI encodes a named key as 0x11_0000 plus its index.
        let press = |key: u32| Event::key(Event::KEY_PRESSED, 0x11_0000 | key);
        let release = |key: u32| Event::key(Event::KEY_RELEASED, 0x11_0000 | key);
        let right = 3;
        let down = 1;
        let shift = 9;
        let run = |ui: &mut Ui, draft, events: &[Event]| {
            let mut result = Action::None;
            ui.frame(
                &FrameIn::new(stage[0], stage[1]).events(events),
                &mut |ui| {
                    result = paint(ui, draft, "Screen take", None, stage);
                },
            )
            .unwrap();
            result
        };
        for _ in 0..3 {
            assert!(matches!(run(&mut ui, draft, &[]), Action::None));
        }
        let Action::Update(plain) = run(&mut ui, draft, &[press(right), release(right)]) else {
            panic!("unmodified right arrow should nudge");
        };
        assert!((plain.rect.x - draft.rect.x - 1.0 / 1920.0).abs() < 1e-12);
        let Action::Update(shifted) = run(
            &mut ui,
            plain,
            &[press(shift), press(down), release(down), release(shift)],
        ) else {
            panic!("Shift+Down should nudge even when Shift is released in the same frame");
        };
        assert!((shifted.rect.y - plain.rect.y - 10.0 / 1080.0).abs() < 1e-12);
        for other in [10, 11, 12] {
            assert!(matches!(
                run(
                    &mut ui,
                    shifted,
                    &[press(other), press(right), release(right), release(other)]
                ),
                Action::None
            ));
            assert!(matches!(
                run(
                    &mut ui,
                    shifted,
                    &[
                        press(shift),
                        press(other),
                        press(right),
                        release(right),
                        release(other),
                        release(shift)
                    ]
                ),
                Action::None
            ));
        }
        assert!(matches!(
            run(&mut ui, shifted, &[press(6), release(6)]),
            Action::Cancel
        ));
        let Action::Apply(op) = run(&mut ui, shifted, &[press(5), release(5)]) else {
            panic!("Enter should commit one crop op");
        };
        assert_eq!(op, shifted.command());

        for _ in 0..2 {
            assert!(matches!(run(&mut ui, draft, &[]), Action::None));
        }
        let [left, top, ..] = ui
            .layers()
            .flat_map(|layer| layer.instances)
            .find(|item| {
                item.color == [INK.r, INK.g, INK.b, INK.a]
                    && item.rect[2] - item.rect[0] == 10.0
                    && item.rect[3] - item.rect[1] == 10.0
            })
            .expect("northwest handle")
            .rect;
        let (x, y) = (f64::from(left) + 0.5, f64::from(top) + 4.5);
        let Action::Update(grabbed) = run(
            &mut ui,
            draft,
            &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
        ) else {
            panic!("the outer half of the 10pt handle should start the drag");
        };
        assert_eq!(grabbed.rect, draft.rect);
        let Action::Update(dragged) = run(
            &mut ui,
            grabbed,
            &[Event::pointer(Event::POINTER_MOVED, 0, x + 20.0, y)],
        ) else {
            panic!("dragging should only update the draft");
        };
        assert!(dragged.rect.x > draft.rect.x);
        assert_eq!(dragged.rect.x + dragged.rect.w, draft.rect.x + draft.rect.w);
        let Action::Update(released) = run(
            &mut ui,
            dragged,
            &[Event::pointer(Event::POINTER_RELEASED, 0, x + 20.0, y)],
        ) else {
            panic!("release should finish the drag without applying");
        };
        assert_eq!(released.rect, dragged.rect);
        assert!(released.drag.is_none());
    }

    #[test]
    fn crop_preview_decodes_uncropped_source_at_retimed_playhead() {
        use splice_model::{
            Anchor, Clip, ClipData, ClipId, ClipTiming, Framing, Item, ItemData, ItemId,
            ItemTiming, Lane, LaneId, LaneKind, Media, MediaId, Overlay, Placement, Probe,
        };

        let package = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media");
        let clip = ClipId::from_serial(2);
        let media = MediaId::from_serial(1);
        let mut project = Project {
            fps: 60,
            ..Project::default()
        };
        project.media.push(Media {
            id: media,
            kind: MediaKind::Video,
            path: "probe.mov".into(),
            bookmark: None,
            size: 0,
            mtime: 0,
            probe: Probe {
                fps: 30.0,
                frames: 30,
                width: 96,
                height: 64,
                audio: Vec::new(),
                codec: None,
                loudness: None,
            },
            take: None,
        });
        project
            .clips
            .insert(Clip {
                id: clip,
                timing: ClipTiming {
                    start: 10,
                    length: 10,
                    source: 4,
                    speed: 2.0,
                },
                data: ClipData {
                    media: Some(media),
                    framing: Framing {
                        crop: Some(Crop {
                            x: 0.25,
                            y: 0.25,
                            w: 0.5,
                            h: 0.5,
                        }),
                        fit: Fit::Fill,
                    },
                    ..ClipData::default()
                },
            })
            .unwrap();
        let draft = Draft::new(&project, TimelineId::Clip(clip)).unwrap();
        let decoded = draft.preview(&project, &package, 15).unwrap();
        let source = apple_sys::media::frame_at(&package.join("probe.mov"), 9.0 / 30.0).unwrap();
        assert_eq!(decoded.1, [96, 64]);
        assert_eq!(decoded, source);
        let lane = LaneId::from_serial(3);
        project.lanes.push(Lane {
            id: lane,
            name: "Overlay".into(),
            kind: LaneKind::Overlay,
            effects: Vec::new(),
        });
        let item = ItemId::from_serial(4);
        project
            .items
            .insert(Item {
                id: item,
                timing: ItemTiming {
                    anchor: Anchor::ClipSource { clip, source: 6 },
                    length: 10,
                },
                data: ItemData {
                    lane,
                    kind: ItemKind::Media(Overlay {
                        media,
                        source: 3,
                        placement: Placement::Card,
                        behind_presenter: false,
                        muted: true,
                        framing: Framing {
                            crop: Some(FULL),
                            fit: Fit::Fill,
                        },
                    }),
                    appearance: splice_model::Appearance::default(),
                },
            })
            .unwrap();
        let draft = Draft::new(&project, TimelineId::Item(item)).unwrap();
        let decoded = draft.preview(&project, &package, 20).unwrap();
        let source = apple_sys::media::frame_at(&package.join("probe.mov"), 7.0 / 30.0).unwrap();
        assert_eq!(decoded, source);
        assert_eq!(draft.target(), TimelineId::Item(item));
        let command = draft.command();
        assert_eq!(
            command.field("items").and_then(Json::as_array).unwrap()[0].as_str(),
            Some(item.to_string().as_str()),
        );
    }
    #[test]
    fn crop_fit_and_bounds_emit_one_valid_wire_command() {
        let mut draft = Draft {
            id: TimelineId::Clip(splice_model::ClipId::from_serial(7)),
            size: [1920, 1080],
            output: 9.0 / 16.0,
            rect: FULL,
            fit: Fit::Fit,
            lock: 0,
            drag: None,
        };
        assert_eq!(draft.command().field("rect"), Some(&Json::Null));
        draft.fit = Fit::Fill;
        draft.lock = 1;
        let ratio = draft.ratio().unwrap();
        draft.rect = resize(conform(FULL, ratio), [-1, -1], 0.9, 0.9, Some(ratio));
        assert!((draft.rect.w / draft.rect.h - ratio).abs() < 1e-10);
        let command = draft.command();
        assert_eq!(
            command.field("cmd").and_then(Json::as_str),
            Some("clip.crop")
        );
        assert_eq!(command.field("fit").and_then(Json::as_str), Some("fill"));
        let crop = command.field("rect").unwrap();
        let point = |key| crop.field(key).and_then(Json::as_f64).unwrap();
        assert!(point("w") >= MIN && point("h") >= MIN);
        assert!(point("x") + point("w") <= 1.0 && point("y") + point("h") <= 1.0);
    }
}
