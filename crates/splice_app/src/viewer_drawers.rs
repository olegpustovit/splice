use pxui::{
    Button, Fill, Label, Node, Overlay, Paint, Point, Rect, Stroke, Ui,
    geom::Rgba,
    ui::{Look, Size, State, States},
};
use splice_core::{commands::Target, edit::ApplyReport};
use splice_model::{ClipId, Media, MediaId, MediaKind, Project, TimelineId};
use std::path::{Path, PathBuf};

#[path = "viewer_drawers/effects.rs"]
mod effects;

pub(super) use effects::{Effect, EffectTargets, drop_at as effect_drop};

const CONTROL: Rgba = Rgba::rgb(20, 20, 21);
const LINE: Rgba = Rgba::rgb(37, 37, 39);
const BORDER: Rgba = Rgba::rgb(56, 56, 59);
const INK: Rgba = Rgba::rgb(244, 243, 238);
const MUTED: Rgba = Rgba::rgb(177, 177, 171);
const MATTE: Rgba = Rgba::rgb(6, 6, 7);
const VIDEO: Rgba = Rgba::rgb(236, 230, 206);
const DRAWER: Look = Look::NONE.fill(CONTROL).border(BORDER, 1.0);
const BUTTON: States = States::of(Look::NONE.text(MUTED).border(LINE, 1.0))
    .hover(
        Look::NONE
            .text(INK)
            .fill(Rgba::rgb(34, 34, 36))
            .border(BORDER, 1.0),
    )
    .pressed(
        Look::NONE
            .text(INK)
            .fill(Rgba::rgb(42, 42, 45))
            .border(BORDER, 1.0),
    );
const MEDIA_BUTTON: States = States::of(Look::NONE.text(INK).fill(CONTROL).border(LINE, 1.0))
    .hover(
        Look::NONE
            .text(INK)
            .fill(Rgba::rgb(34, 34, 36))
            .border(BORDER, 1.0),
    )
    .disabled(Look::NONE.text(QUIET).fill(CONTROL).border(LINE, 1.0));
const QUIET: Rgba = Rgba::rgb(130, 130, 125);
const MEDIA_NAME: Label = Label::MONO
    .size(12.0)
    .line_height(16.8)
    .w(Size::Grow)
    .wrap();
const MEDIA_DETAIL: Label = Label::MONO
    .size(10.0)
    .line_height(14.0)
    .w(Size::Grow)
    .wrap();
const SCREEN_SAMPLE: &str = "med_screen";
const WEBCAM_SAMPLE: &str = "med_webcam";
const CAPTION: Label = Label::MONO.size(10.0).w(Size::Grow).ellipsis();
const BODY: Label = Label::MONO.size(12.0).w(Size::Grow).ellipsis();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Drawer {
    Media,
    Effects,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    Import,
    Append(MediaId),
    Overlay(MediaId),
    Effect(Effect),
}

pub(super) struct Intents {
    pub open: bool,
    pub action: Option<Action>,
}

pub(super) fn paint(
    ui: &mut Ui,
    drawer: Drawer,
    (project, mut sources): (Option<&Project>, Option<(&Path, &mut MediaSources)>),
    [width, height]: [f32; 2],
    targets: EffectTargets,
) -> Intents {
    let effects = drawer == Drawer::Effects;
    let (name, key, desktop_x, desktop_width) = match drawer {
        Drawer::Media => ("Media", "viewer-media", 8.0, 300.0),
        Drawer::Effects => ("Effects", "viewer-effects", 83.0, 380.0),
    };
    let compact = width <= 480.0;
    let x = if compact { 12.0 } else { desktop_x };
    let y = 63.5;
    let maximum = (width - x - 12.0).max(180.0);
    let drawer_width = if compact {
        maximum
    } else {
        maximum.min(desktop_width)
    };
    let available_height = (height - y - 8.0).max(1.0);
    let max_height = if effects {
        (height * 0.7).min(available_height).max(1.0)
    } else {
        available_height
    };
    let phase = effects.then(|| {
        ui.node(
            "effect-drag-phase",
            Node::DEFAULT.at(0.0, 0.0).w_px(0.0).h_px(0.0),
        )
    });
    let hidden = effects && effects::carrying(ui);
    if let Some(phase) = phase
        && !hidden
        && ui.state(&phase, 0) != 0
    {
        ui.set_state(&phase, 0);
        return Intents {
            open: false,
            action: None,
        };
    }
    let clear = Rgba::rgba(0, 0, 0, 0);
    let options = Overlay::DEFAULT
        .at(Point::new(x, y))
        .w_px(drawer_width)
        .max_h_px(max_height)
        .padding(1.0)
        .look(if hidden { &Look::NONE } else { &DRAWER })
        .stroke(if hidden { clear } else { BORDER })
        .name(name);
    let options = if effects {
        options.h_px(max_height)
    } else {
        options
    };
    let mut action = None;
    let popup = ui.popup(key, &options, |ui| {
        if hidden {
            ui.node("effect-capture", Node::DEFAULT.h_px(1.0));
            return;
        }
        ui.column(Node::DEFAULT.w(Size::Grow), |ui| {
            ui.row(
                Node::DEFAULT
                    .w(Size::Grow)
                    .h_px(if effects { 34.4 } else { 36.0 })
                    .center()
                    .pad_x(8.0)
                    .gap(8.0),
                |ui| {
                    let heading = super::EDITOR_HEADING.size(16.0).w(Size::Grow);
                    let label = ui.label(name, heading);
                    if hidden {
                        ui.style(
                            &label,
                            &States::of(Look::NONE.text(clear)),
                            State::default(),
                        );
                    }
                    if effects {
                        let hint =
                            ui.label("drag onto a clip, lane or the viewer", CAPTION.w(Size::Fit));
                        let ink = if hidden {
                            clear
                        } else {
                            Rgba::rgb(130, 130, 125)
                        };
                        ui.style(&hint, &States::of(Look::NONE.text(ink)), State::default());
                    } else if tool(
                        ui,
                        "import-media",
                        "+ Import video",
                        (Size::Px(119.0), 24.0),
                        12.0,
                        project.is_some(),
                    ) {
                        action = Some(Action::Import);
                    }
                },
            );
            rule(ui, "drawer-rule");
            match drawer {
                Drawer::Media => media(ui, project, drawer_width, &mut sources, &mut action),
                Drawer::Effects => {
                    action = effects::paint(ui, project, &targets).map(Action::Effect)
                }
            }
        });
    });
    // Retain the popup's capture while its visible contents are gone.
    if let Some(phase) = phase
        && !hidden
        && popup.open
        && effects::carrying(ui)
    {
        if ui.transfer_carry(&popup.handle) {
            ui.set_state(&phase, 1);
        } else {
            ui.cancel_carry();
        }
    }
    Intents {
        open: popup.open && action.is_none(),
        action,
    }
}

#[derive(Default)]
pub(super) struct MediaSources {
    source: PathBuf,
    component: PathBuf,
}

#[derive(Clone, Copy, Default)]
struct MediaAvailability {
    append: bool,
    overlay: bool,
}

impl MediaSources {
    pub(super) fn allows(&mut self, project: &Project, package: &Path, action: &Action) -> bool {
        let id = match *action {
            Action::Append(id) | Action::Overlay(id) => id,
            _ => return true,
        };
        let Some(file) = project.media.iter().find(|file| file.id == id) else {
            return false;
        };
        let available = self.check(project, package, file, true);
        match action {
            Action::Append(_) => available.append,
            Action::Overlay(_) => available.overlay,
            _ => unreachable!("media action"),
        }
    }

    fn check(
        &mut self,
        project: &Project,
        package: &Path,
        file: &Media,
        read: bool,
    ) -> MediaAvailability {
        if file.kind == MediaKind::Compound {
            let exists = project.compounds.iter().any(|entry| entry.media == file.id);
            return MediaAvailability {
                append: exists,
                overlay: exists,
            };
        }
        self.source.clear();
        self.source.push(package);
        self.source.push(&file.path);
        let Ok(metadata) = self.source.metadata() else {
            return MediaAvailability::default();
        };
        let primary = media_file(&self.source, &metadata, read);
        if let Some(take) = file.take.as_ref().filter(|_| file.kind == MediaKind::Take) {
            // AV reads a take's components inside its directory or beside its primary movie.
            if !metadata.is_dir() {
                self.source.pop();
            }
            let mut present = |file: &str| {
                self.component.clear();
                self.component.push(&self.source);
                self.component.push(file);
                self.component
                    .metadata()
                    .is_ok_and(|info| media_file(&self.component, &info, read))
            };
            let screen = take.screen.as_deref().map(&mut present);
            let camera = take.camera.as_deref().map(&mut present);
            let mic = take.mic.as_deref().map(&mut present);
            let system = take.system.as_deref().map(&mut present);
            let tracks = [screen, camera, mic, system];
            let online = tracks.into_iter().flatten().all(|present| present);
            return MediaAvailability {
                append: online && (primary || tracks.into_iter().any(|part| part.is_some())),
                overlay: online && screen.unwrap_or(primary),
            };
        }
        MediaAvailability {
            append: primary && append_kind(file.kind),
            overlay: primary,
        }
    }
}

fn media_file(path: &Path, info: &std::fs::Metadata, read: bool) -> bool {
    info.is_file() && info.len() > 0 && (!read || std::fs::File::open(path).is_ok())
}

pub(super) fn appended_clip(
    project: &Project,
    report: &ApplyReport,
) -> Option<(ClipId, i32, bool)> {
    let Target::Timeline(TimelineId::Clip(id)) = *report.changes.first()?.created.first()? else {
        return None;
    };
    let slot = project.clips.find(id)?;
    let start = project.clips.timing(slot)?.start;
    let take = project.clips.data(slot)?.media.is_some_and(|media| {
        project
            .media
            .iter()
            .any(|file| file.id == media && file.kind == MediaKind::Take)
    });
    Some((id, start, take))
}

fn media(
    ui: &mut Ui,
    project: Option<&Project>,
    width: f32,
    sources: &mut Option<(&Path, &mut MediaSources)>,
    action: &mut Option<Action>,
) {
    let mut samples = [None; 2];
    ui.column(Node::column().w(Size::Grow).pad(4.0), |ui| {
        let Some(project) = project else { return };
        for file in &project.media {
            let available = sources
                .as_mut()
                .map_or(MediaAvailability::default(), |(package, paths)| {
                    paths.check(project, package, file, false)
                });
            if file.kind == MediaKind::Take
                && available.append
                && let Some(take) = &file.take
            {
                if take.screen.is_some() && SCREEN_SAMPLE.parse().ok() == Some(file.id) {
                    samples[0] = Some(file.id);
                }
                if take.camera.is_some() && WEBCAM_SAMPLE.parse().ok() == Some(file.id) {
                    samples[1] = Some(file.id);
                }
            }
            ui.id(file.id.serial(), |ui| {
                media_row(ui, file, width, available, action)
            });
        }
    });
    rule(ui, "media-capture-rule");
    ui.row(
        Node::row()
            .w(Size::Grow)
            .h_px(40.0)
            .center()
            .pad_x(8.0)
            .gap(4.0),
        |ui| {
            for (key, title, sample) in [
                ("screen-sample", "+ Screen sample", samples[0]),
                ("webcam-sample", "+ Webcam sample", samples[1]),
            ] {
                if tool(ui, key, title, (Size::Grow, 24.0), 11.0, sample.is_some())
                    && let Some(id) = sample
                {
                    *action = Some(Action::Append(id));
                }
            }
        },
    );
}

fn media_row(
    ui: &mut Ui,
    file: &Media,
    width: f32,
    available: MediaAvailability,
    action: &mut Option<Action>,
) {
    let (name, kind, glyph) = media_chrome(file);
    let append = append_kind(file.kind);
    let button_width = if append { 108.0 } else { 56.0 };
    // Border, list and row padding, swatch and the two column gaps.
    let name_width = (width - 2.0 - 8.0 - 10.0 - 32.0 - 16.0 - button_width).max(1.0);
    ui.row(Node::row().w(Size::Grow).center().pad(5.0).gap(8.0), |ui| {
        let swatch = ui.node("media-swatch", Node::DEFAULT.w_px(32.0).h_px(22.0));
        ui.canvas(&swatch, move |paint, rect| {
            paint.fill(rect, Fill::new(VIDEO));
            media_icon(paint, rect, glyph);
        });
        ui.column(Node::column().w_px(name_width), |ui| {
            ui.label(name, MEDIA_NAME);
            if let Some(seconds) = source_seconds(file) {
                ui.label_fmt(
                    format_args!("{} · {seconds:.2} s · {kind}", file.id),
                    MEDIA_DETAIL,
                );
            } else if file.kind == MediaKind::Image {
                ui.label_fmt(format_args!("{} · still · {kind}", file.id), MEDIA_DETAIL);
            } else {
                ui.label_fmt(format_args!("{} · {kind}", file.id), MEDIA_DETAIL);
            }
        });
        ui.row(Node::row().center().gap(2.0), |ui| {
            if append
                && tool(
                    ui,
                    "append",
                    "Append",
                    (Size::Px(50.0), 20.0),
                    10.0,
                    available.append,
                )
            {
                *action = Some(Action::Append(file.id));
            }
            if tool(
                ui,
                "overlay",
                "Overlay",
                (Size::Px(56.0), 20.0),
                10.0,
                available.overlay,
            ) {
                *action = Some(Action::Overlay(file.id));
            }
        });
    });
}

fn append_kind(kind: MediaKind) -> bool {
    matches!(
        kind,
        MediaKind::Video | MediaKind::Take | MediaKind::Compound
    )
}

#[derive(Clone, Copy)]
enum MediaGlyph {
    Film,
    Audio,
    Image,
    Screen,
    Webcam,
    Shapes,
}

fn media_chrome(file: &Media) -> (&str, &'static str, MediaGlyph) {
    use MediaGlyph::*;
    for (id, kind, name, label, glyph) in [
        (
            "med_opener",
            MediaKind::Video,
            "Opening motion",
            "graphic",
            Shapes,
        ),
        (
            SCREEN_SAMPLE,
            MediaKind::Take,
            "Screen take 01",
            "sync take",
            Screen,
        ),
        (
            WEBCAM_SAMPLE,
            MediaKind::Take,
            "Webcam take 01",
            "sync take",
            Webcam,
        ),
        (
            "med_endcard",
            MediaKind::Video,
            "End card",
            "graphic",
            Shapes,
        ),
        (
            "med_broll",
            MediaKind::Video,
            "Desk b-roll",
            "graphic",
            Film,
        ),
        ("med_logo", MediaKind::Image, "Logo bug", "image", Image),
    ] {
        if file.kind == kind && id.parse().ok() == Some(file.id) {
            return (name, label, glyph);
        }
    }
    let (label, glyph) = match file.kind {
        MediaKind::Video => ("local video", Film),
        MediaKind::Audio => ("audio", Audio),
        MediaKind::Image => ("image", Image),
        MediaKind::Take => (
            "sync take",
            if file
                .take
                .as_ref()
                .is_some_and(|take| take.screen.is_none() && take.camera.is_some())
            {
                Webcam
            } else {
                Screen
            },
        ),
        MediaKind::Compound => ("compound", Film),
    };
    (file_name(file), label, glyph)
}

fn media_icon(paint: &mut Paint<'_>, rect: Rect, glyph: MediaGlyph) {
    let unit = 14.0 / 24.0;
    let left = rect.x + (rect.w - 14.0) * 0.5;
    let top = rect.y + (rect.h - 14.0) * 0.5;
    let point = |(x, y): (f32, f32)| (left + x * unit, top + y * unit);
    let stroke = Stroke::new(MATTE).width(1.6 * unit);
    let path = |paint: &mut Paint<'_>, points: &[(f32, f32)]| {
        for pair in points.windows(2) {
            paint.line(point(pair[0]), point(pair[1]), stroke);
        }
    };
    let frame = |paint: &mut Paint<'_>, x, y, w, h| {
        let (x, y) = point((x, y));
        paint.frame(Rect::new(x, y, w * unit, h * unit), stroke);
    };
    match glyph {
        MediaGlyph::Film => {
            frame(paint, 3.0, 5.0, 18.0, 14.0);
            for (left, right, x) in [(3.0, 7.5, 7.5), (16.5, 21.0, 16.5)] {
                path(paint, &[(x, 5.0), (x, 19.0)]);
                for y in [9.5, 14.5] {
                    path(paint, &[(left, y), (right, y)]);
                }
            }
        }
        MediaGlyph::Audio => asset_icon(paint, rect, MATTE, 1),
        MediaGlyph::Image => {
            frame(paint, 3.0, 4.0, 18.0, 16.0);
            paint.circle(point((9.0, 9.5)), 1.8 * unit, None, Some(stroke));
            path(
                paint,
                &[
                    (3.0, 17.0),
                    (8.5, 12.0),
                    (12.5, 15.5),
                    (15.5, 13.0),
                    (21.0, 17.5),
                ],
            );
        }
        MediaGlyph::Screen => {
            frame(paint, 3.0, 4.0, 18.0, 12.0);
            path(paint, &[(8.0, 20.0), (16.0, 20.0)]);
            path(paint, &[(12.0, 16.0), (12.0, 20.0)]);
        }
        MediaGlyph::Webcam => {
            frame(paint, 3.0, 7.0, 13.0, 10.0);
            path(
                paint,
                &[(16.0, 11.0), (21.0, 8.0), (21.0, 16.0), (16.0, 13.0)],
            );
        }
        MediaGlyph::Shapes => {
            paint.circle(point((9.0, 9.0)), 5.0 * unit, None, Some(stroke));
            frame(paint, 11.5, 11.5, 8.5, 8.5);
        }
    }
}

fn rule(ui: &mut Ui, key: &'static str) {
    ui.node(
        key,
        Node::DEFAULT
            .w(Size::Grow)
            .h_px(1.0)
            .look(Look::NONE.fill(LINE)),
    );
}

fn tool(
    ui: &mut Ui,
    key: &'static str,
    title: &str,
    (width, height): (Size, f32),
    font: f32,
    enabled: bool,
) -> bool {
    let button = Button::DEFAULT
        .size(width, Size::Px(height))
        .centered()
        .disabled(!enabled)
        .look(&MEDIA_BUTTON)
        .label(Label::MONO.size(font));
    ui.button(key, title, &button).clicked
}

fn file_name(file: &Media) -> &str {
    Path::new(&file.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&file.path)
}

fn source_seconds(file: &Media) -> Option<f64> {
    let fps = file.probe.fps;
    (file.kind != MediaKind::Image && fps.is_finite() && fps > 0.0)
        .then(|| f64::from(file.probe.frames.max(0)) / fps)
}

fn asset_icon(paint: &mut Paint<'_>, rect: Rect, ink: Rgba, word: u32) {
    let side = rect.w.min(rect.h).min(14.0);
    let unit = side / 24.0;
    let left = rect.x + (rect.w - side) * 0.5;
    let top = rect.y + (rect.h - side) * 0.5;
    let point = |(x, y): (f32, f32)| (left + x * unit, top + y * unit);
    let stroke = Stroke::new(ink);
    let path = |paint: &mut Paint<'_>, points: &[(f32, f32)]| {
        for pair in points.windows(2) {
            paint.line(point(pair[0]), point(pair[1]), stroke);
        }
    };
    match word {
        1 => {
            for (x, a, b) in [
                (4.0, 10.0, 14.0),
                (8.0, 7.0, 17.0),
                (12.0, 4.0, 20.0),
                (16.0, 8.0, 16.0),
                (20.0, 10.0, 14.0),
            ] {
                path(paint, &[(x, a), (x, b)]);
            }
        }
        4 => {
            path(paint, &[(4.0, 20.0), (14.5, 9.5)]);
            path(paint, &[(12.5, 7.5), (16.5, 11.5)]);
            path(paint, &[(18.0, 3.0), (18.0, 7.0)]);
            path(paint, &[(16.0, 5.0), (20.0, 5.0)]);
        }
        _ => {
            paint.frame(
                Rect::new(
                    left + 2.0 * unit,
                    top + 4.0 * unit,
                    20.0 * unit,
                    16.0 * unit,
                ),
                stroke,
            );
            match word {
                2 => path(
                    paint,
                    &[
                        (4.0, 17.0),
                        (9.0, 10.0),
                        (13.0, 15.0),
                        (16.0, 12.0),
                        (20.0, 17.0),
                    ],
                ),
                3 => {
                    path(paint, &[(7.0, 22.0), (17.0, 22.0)]);
                    path(paint, &[(12.0, 20.0), (12.0, 22.0)]);
                }
                _ => path(
                    paint,
                    &[(10.0, 8.0), (16.0, 12.0), (10.0, 16.0), (10.0, 8.0)],
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pxui::{Role, testing};
    use splice_json::Value as Json;
    use std::cell::Cell;

    #[test]
    fn media_drawer_import_is_accessible_and_returns_an_intent() {
        let action = Cell::new(None);
        let mut frame = |ui: &mut Ui| {
            let project = Project::default();
            let intents = paint(
                ui,
                Drawer::Media,
                (Some(&project), None),
                [320.0, 240.0],
                EffectTargets::default(),
            );
            action.set(intents.action);
        };
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        testing::frame(&mut ui, &[], &mut frame);
        let button = testing::find(&ui, Role::Button, "+ Import video").unwrap();
        testing::click(
            &mut ui,
            f64::from(button.x + button.w * 0.5),
            f64::from(button.y + button.h * 0.5),
            &mut frame,
        );
        assert_eq!(action.take(), Some(Action::Import));
    }

    fn draw_media(
        ui: &mut Ui,
        (project, package): (&Project, &Path),
        width: f32,
        events: &[pxui::Event],
        sources: &mut MediaSources,
    ) -> Option<Action> {
        let mut result = None;
        let input = pxui::FrameIn::new(width, 640.0).events(events);
        ui.frame(&input, &mut |ui| {
            let intents = paint(
                ui,
                Drawer::Media,
                (Some(project), Some((package, sources))),
                [width, 640.0],
                EffectTargets::default(),
            );
            result = result.take().or(intents.action);
        })
        .unwrap();
        result
    }

    fn media_button(ui: &Ui, name: &str) -> (Rect, bool) {
        let button = ui
            .a11y_nodes()
            .iter()
            .find(|node| node.is_role(Role::Button) && ui.a11y_name_of(node) == name)
            .expect("named media button");
        (
            button.rect,
            button.states & pxui::A11yStates::ENABLED.bits() != 0,
        )
    }

    #[test]
    fn media_drawer_uses_the_reference_labels_columns_and_responsive_footer() {
        let bundle =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
        let (_, project) = splice_core::package::Package::open(&bundle).unwrap();
        mem_sys::set_peer(pxui::library_counter);
        for (width, left, extent) in [(960.0, 8.0, 300.0), (480.0, 12.0, 456.0)] {
            let mut sources = MediaSources::default();
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            for _ in 0..3 {
                assert!(
                    draw_media(&mut ui, (&project, &bundle), width, &[], &mut sources).is_none()
                );
            }
            let heading = testing::find(&ui, Role::Label, "Opening motion").unwrap();
            let screen_name = testing::find(&ui, Role::Label, "Screen take 01").unwrap();
            let webcam_name = testing::find(&ui, Role::Label, "Webcam take 01").unwrap();
            let detail =
                testing::find(&ui, Role::Label, "med_screen · 18.00 s · sync take").unwrap();
            let image = testing::find(&ui, Role::Label, "med_logo · still · image").unwrap();
            assert_eq!(heading.x, left + 50.0);
            assert!((heading.y - 110.5).abs() < 1.0, "{heading:?}");
            assert!(
                heading.y < screen_name.y && screen_name.y < webcam_name.y && detail.y < image.y
            );

            let (append, append_available) = media_button(&ui, "Append");
            let (overlay, overlay_available) = media_button(&ui, "Overlay");
            assert_eq!(append.w, 50.0);
            assert_eq!(append.h, overlay.h);
            assert_eq!(overlay.x - append.x - append.w, 2.0);
            assert_eq!(overlay.x + overlay.w, left + extent - 10.0);
            assert!(!append_available && !overlay_available);

            let (import, import_available) = media_button(&ui, "+ Import video");
            assert!((import.y - 70.5).abs() < 1.0, "{import:?}");
            assert!(import_available);
            let (screen, screen_available) = media_button(&ui, "+ Screen sample");
            let (webcam, webcam_available) = media_button(&ui, "+ Webcam sample");
            assert!(!screen_available && !webcam_available);
            assert_eq!(screen.x, left + 9.0);
            assert_eq!(screen.y, webcam.y);
            assert_eq!(screen.h, 24.0);
            assert_eq!(webcam.w, screen.w);
            assert_eq!(webcam.x - screen.x - screen.w, 4.0);
            assert_eq!(webcam.x + webcam.w, left + extent - 9.0);
            assert!(screen.y > image.y + image.h && screen.y + screen.h < 640.0);
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                .warm(&mut || {
                    for _ in 0..12 {
                        assert!(
                            draw_media(&mut ui, (&project, &bundle), width, &[], &mut sources)
                                .is_none()
                        );
                    }
                });
        }
    }

    #[test]
    fn media_drawer_samples_check_live_sources_and_append_through_the_store() {
        use splice_core::history::{Store, WriteOptions};
        use splice_model::{Probe, TakeComponents};
        use std::fs;

        let folder =
            std::env::temp_dir().join(format!("splice-media-drawer-{}", std::process::id()));
        let bundle = folder.join("Media.splice");
        fs::create_dir_all(&folder).unwrap();
        let screen_id: MediaId = SCREEN_SAMPLE.parse().unwrap();
        let webcam_id: MediaId = WEBCAM_SAMPLE.parse().unwrap();
        let mut project = Project::default();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/media/probe.mov");
        for (id, path, screen, camera) in [
            (
                screen_id,
                "assets/screen.mov",
                Some("screen.mov"),
                "camera.mov",
            ),
            (webcam_id, "takes/webcam", None, "camera.mov"),
        ] {
            project.media.push(Media {
                id,
                kind: MediaKind::Take,
                path: path.into(),
                bookmark: None,
                size: fs::metadata(&fixture).unwrap().len(),
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
                take: Some(TakeComponents {
                    screen: screen.map(str::to_owned),
                    camera: Some(camera.into()),
                    ..TakeComponents::default()
                }),
            });
        }
        let mut store = Store::create(&bundle, &project).unwrap();
        let mut sources = MediaSources::default();
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let draw =
            |ui: &mut Ui, store: &Store, sources: &mut MediaSources, events: &[pxui::Event]| {
                draw_media(ui, (store.project(), store.path()), 480.0, events, sources)
            };
        for _ in 0..3 {
            assert!(draw(&mut ui, &store, &mut sources, &[]).is_none());
        }
        assert!(!media_button(&ui, "+ Screen sample").1);
        assert!(!media_button(&ui, "+ Webcam sample").1);
        let click = |ui: &mut Ui, store: &Store, sources: &mut MediaSources, name| {
            let (rect, _) = media_button(ui, name);
            let x = f64::from(rect.x + rect.w * 0.5);
            let y = f64::from(rect.y + rect.h * 0.5);
            assert!(
                draw(
                    ui,
                    store,
                    sources,
                    &[pxui::Event::pointer(pxui::Event::POINTER_PRESSED, 0, x, y)]
                )
                .is_none()
            );
            draw(
                ui,
                store,
                sources,
                &[pxui::Event::pointer(pxui::Event::POINTER_RELEASED, 0, x, y)],
            )
        };
        assert!(click(&mut ui, &store, &mut sources, "+ Screen sample").is_none());
        assert_eq!(store.project().rev, 0);

        fs::create_dir_all(bundle.join("assets")).unwrap();
        fs::create_dir_all(bundle.join("takes/webcam")).unwrap();
        fs::copy(&fixture, bundle.join("assets/screen.mov")).unwrap();
        fs::copy(&fixture, bundle.join("takes/webcam/camera.mov")).unwrap();
        assert!(draw(&mut ui, &store, &mut sources, &[]).is_none());
        assert!(!media_button(&ui, "+ Screen sample").1);
        assert!(media_button(&ui, "+ Webcam sample").1);
        // A declared but absent camera prevents inserting an incomplete take.
        fs::copy(&fixture, bundle.join("assets/camera.mov")).unwrap();
        assert!(draw(&mut ui, &store, &mut sources, &[]).is_none());
        assert!(media_button(&ui, "+ Screen sample").1);

        for (index, label, media) in [
            (0, "+ Screen sample", screen_id),
            (1, "+ Webcam sample", webcam_id),
        ] {
            let action = click(&mut ui, &store, &mut sources, label).unwrap();
            assert_eq!(action, Action::Append(media));
            assert!(sources.allows(store.project(), store.path(), &action));
            let op = crate::drawer_operation(action, store.project(), 5).unwrap();
            let receipt = store.apply(&[op], &WriteOptions::default()).unwrap();
            let (id, start, take) = appended_clip(store.project(), &receipt.report).unwrap();
            let slot = store.project().clips.find(id).unwrap();
            let data = store.project().clips.data(slot).unwrap();
            assert!(take && data.enabled.camera == Some(true));
            assert_eq!(data.enabled.screen, (media == screen_id).then_some(true));
            assert_eq!(data.media, Some(media));
            assert_eq!(start, index * 30);
            assert_eq!(store.project().clips.timing(slot).unwrap().length, 30);
            assert!(draw(&mut ui, &store, &mut sources, &[]).is_none());
        }
        assert_eq!(store.project().rev, 2);
        fs::remove_file(bundle.join("takes/webcam/camera.mov")).unwrap();
        assert!(draw(&mut ui, &store, &mut sources, &[]).is_none());
        assert!(!media_button(&ui, "+ Webcam sample").1);
        assert!(!sources.allows(store.project(), store.path(), &Action::Append(webcam_id)));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn effects_drawer_measures_the_categories_and_applies_the_project_button() {
        use pxui::{Event, FrameIn};

        let project = Project::default();
        for width in [960.0, 800.0, 480.0] {
            let mut ui = crate::viewer_ui().unwrap();
            ui.set_a11y(true);
            let mut applied = None;
            let mut build = |ui: &mut Ui| {
                let intents = paint(
                    ui,
                    Drawer::Effects,
                    (Some(&project), None),
                    [width, 640.0],
                    EffectTargets::default(),
                );
                if let Some(action) = intents.action {
                    applied = Some(action);
                }
            };
            let frame = FrameIn::new(width, 640.0);
            for _ in 0..3 {
                ui.frame(&frame, &mut build).unwrap();
            }
            let named = |role, name| {
                testing::find(&ui, role, name).unwrap_or_else(|| panic!("missing {name}"))
            };
            let frame_group = named(Role::Label, "FRAME");
            let cursor_group = named(Role::Label, "CURSOR");
            let first = named(Role::Button, "Window");
            let device = named(Role::Button, "Device Frame");
            let blur = named(Role::Button, "Background Blur");
            assert!(
                frame_group.y < first.y
                    && first.y < device.y
                    && device.y < blur.y
                    && blur.y < cursor_group.y
            );
            let project_button = named(Role::Button, "Project");
            let selected = named(Role::Button, "Selection");
            assert_eq!(project_button.y, selected.y);
            assert!(first.w > 300.0 && first.x >= 12.0 && first.x + first.w < width);
            assert!((first.y - 133.9).abs() < 1.0, "{first:?}");
            println!(
                "drawer {width} frame {frame_group:?} window {first:?} cursor {cursor_group:?} project {project_button:?}"
            );
            let x = f64::from(project_button.x + project_button.w * 0.5);
            let y = f64::from(project_button.y + project_button.h * 0.5);
            for kind in [Event::POINTER_PRESSED, Event::POINTER_RELEASED] {
                ui.frame(&frame.events(&[Event::pointer(kind, 0, x, y)]), &mut build)
                    .unwrap();
            }
            let Some(Action::Effect(effect)) = applied else {
                panic!("Project click lost")
            };
            assert_eq!(effect.target, splice_core::commands::Target::Project);
            let operation = effect.operation(&project).unwrap();
            assert_eq!(
                operation.field("type").and_then(Json::as_str),
                Some("window")
            );
            splice_core::commands::decode(&operation).unwrap();
        }
    }

    #[test]
    fn effects_drawer_pointer_carries_apply_to_real_timeline_and_program() {
        for width in [960.0, 800.0, 480.0] {
            carry_through_editor(width);
        }
    }

    fn carry_through_editor(width: f32) {
        use crate::{program, timeline};
        use pxui::{Event, FrameIn, Handle};
        use splice_core::{
            commands::Target,
            history::{Store, WriteOptions},
        };
        use splice_model::{LaneId, TimelineId};
        use std::{cell::RefCell, fs};

        let folder =
            std::env::temp_dir().join(format!("splice-effect-drag-{}", std::process::id()));
        let bundle = folder.join("Effects.splice");
        fs::create_dir_all(bundle.join("transcripts")).unwrap();
        let sample =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
        for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
            fs::copy(sample.join(file), bundle.join(file)).unwrap();
        }
        let mut store = Store::open(&bundle).unwrap();
        let clip = "clp_take".parse().unwrap();
        let item = "ovl_logo".parse().unwrap();
        let lane = "lan_ov1".parse::<LaneId>().unwrap();
        let mut grouped = timeline::State::default();
        grouped.set_selected(vec![TimelineId::Clip(clip), TimelineId::Item(item)]);
        let targets = crate::drawer_targets(store.project(), &grouped, 192);
        assert!(targets.selection.is_none() && targets.lane.is_none());

        let canvas = Cell::new(true);
        let stage = Cell::new(None::<Handle>);
        let cache = RefCell::new(timeline::Cache::default());
        let frame = FrameIn::new(width, 640.0);
        let mut ui = crate::viewer_ui().unwrap();
        ui.set_a11y(true);
        let draw = |ui: &mut Ui, project: &Project, events: &[Event], drawer: bool| {
            let mut emitted = None;
            let mut open = drawer;
            ui.frame(&frame.events(events), &mut |ui| {
                if drawer {
                    let intents = paint(
                        ui,
                        Drawer::Effects,
                        (Some(project), None),
                        [width, 640.0],
                        EffectTargets {
                            selection: Some(Target::Timeline(TimelineId::Clip(clip))),
                            lane: Some(LaneId::SPINE),
                        },
                    );
                    open &= intents.open;
                    if let Some(Action::Effect(effect)) = intents.action {
                        emitted = Some(effect);
                    }
                }
                ui.column(Node::column().w(Size::Grow).h(Size::Grow), |ui| {
                    ui.node("toolbar", Node::DEFAULT.h_px(64.0));
                    ui.column(Node::column().w(Size::Grow).h_px(181.0), |ui| {
                        let (_, handle, _) = program::paint(
                            ui,
                            Some(project.size),
                            None,
                            None,
                            program::View::default(),
                            &[],
                            None,
                        );
                        stage.set(Some(handle));
                        let target = if canvas.get() {
                            Target::Project
                        } else {
                            Target::Timeline(TimelineId::Clip(clip))
                        };
                        if let Some(effect) = effect_drop(ui, &handle, project, target) {
                            emitted = Some(effect);
                        }
                    });
                    let mut state = timeline::State::default();
                    state.fit();
                    let intents = timeline::paint(
                        ui,
                        Some(project),
                        timeline::View {
                            width,
                            height: 395.,
                            at: 192,
                            end: 660,
                            dragging: false,
                            history: (false, false),
                            state,
                        },
                        &mut cache.borrow_mut(),
                        (&[], None),
                    );
                    if let Some(effect) = intents.effect {
                        emitted = Some(effect);
                    }
                });
            })
            .unwrap();
            (emitted, open)
        };
        let find = |ui: &Ui, name| {
            testing::find(ui, Role::Button, name).unwrap_or_else(|| panic!("missing {name}"))
        };
        let point = |rect: Rect| {
            (
                f64::from(rect.x + rect.w * 0.5),
                f64::from(rect.y + rect.h * 0.5),
            )
        };
        let gesture = |ui: &mut Ui, project: &Project, from: Rect, to: Rect| {
            let (x, y) = point(from);
            let (xx, yy) = point(to);
            for event in [
                Event::pointer(Event::POINTER_PRESSED, 0, x, y),
                Event::pointer(Event::POINTER_MOVED, 0, xx, yy),
            ] {
                let (edit, open) = draw(ui, project, &[event], true);
                assert!(
                    edit.is_none() && open,
                    "event {}, from {from:?}, to {to:?}, edit {edit:?}, open {open}, carry {}",
                    event.kind,
                    effects::carrying(ui)
                );
            }
            assert!(effects::carrying(ui), "pointer carry rejected {from:?}");
            assert!(draw(ui, project, &[], true).0.is_none());
            let result = draw(
                ui,
                project,
                &[Event::pointer(Event::POINTER_RELEASED, 0, xx, yy)],
                true,
            );
            assert!(!effects::carrying(ui));
            assert!(!result.1, "drawer closes after a pointer drag");
            result.0
        };
        let ready = |ui: &mut Ui, project: &Project| {
            for _ in 0..3 {
                assert!(draw(ui, project, &[], true).0.is_none());
            }
        };
        ready(&mut ui, store.project());
        let from = find(&ui, "Window");
        let screen = find(&ui, "screen");
        let watermark = find(&ui, "Watermark");
        let overlay_lane = find(&ui, "Overlay 1");
        let captions = find(&ui, "Captions");
        let program = ui.rect(&stage.get().unwrap());
        // This actual stage region lies beneath the drawer's visible bounds before pickup.
        let (left, extent) = if width <= 480.0 {
            (12.0, width - 24.0)
        } else {
            (83.0, 380.0)
        };
        let program = program.intersect(Rect::new(left, 64.0, extent, 448.0));
        for (to, target) in [
            (screen, Target::Timeline(TimelineId::Clip(clip))),
            (watermark, Target::Timeline(TimelineId::Item(item))),
            (overlay_lane, Target::Lane(lane)),
            (program, Target::Project),
        ] {
            assert!(to.y >= 64.0 && to.y + to.h <= 640.0, "{target:?} {to:?}");
            ready(&mut ui, store.project());
            let effect = gesture(&mut ui, store.project(), from, to)
                .unwrap_or_else(|| panic!("no drop for {target:?}: {to:?}"));
            assert_eq!(effect.target, target);
            let operation = effect.operation(store.project()).unwrap();
            splice_core::commands::decode(&operation).unwrap();
            store.apply(&[operation], &WriteOptions::default()).unwrap();
        }
        let project = store.project();
        assert_eq!(project.rev, 4);
        assert!(project.effects.iter().any(|effect| effect.kind == "window"));
        let clip_data = project
            .clips
            .data(project.clips.find(clip).unwrap())
            .unwrap();
        assert!(
            clip_data
                .appearance
                .effects
                .iter()
                .any(|effect| effect.kind == "window")
        );
        let item_data = project
            .items
            .data(project.items.find(item).unwrap())
            .unwrap();
        assert!(
            item_data
                .appearance
                .effects
                .iter()
                .any(|effect| effect.kind == "window")
        );
        assert!(
            project
                .lanes
                .iter()
                .find(|row| row.id == lane)
                .unwrap()
                .effects
                .iter()
                .any(|effect| effect.kind == "window")
        );
        ready(&mut ui, project);
        assert!(gesture(&mut ui, project, from, captions).is_none());
        assert_eq!(project.rev, 4);

        if width == 960.0 {
            ready(&mut ui, project);
            let (x, y) = point(from);
            let (xx, yy) = point(program);
            for event in [
                Event::pointer(Event::POINTER_PRESSED, 0, x, y),
                Event::pointer(Event::POINTER_MOVED, 0, xx, yy),
            ] {
                assert!(draw(&mut ui, project, &[event], true).0.is_none());
            }
            assert!(effects::carrying(&ui));
            let escape = Event::key(Event::KEY_PRESSED, 0x11_0006);
            assert!(draw(&mut ui, project, &[escape], true).0.is_none());
            assert!(!effects::carrying(&ui));
            let release = Event::pointer(Event::POINTER_RELEASED, 0, xx, yy);
            assert!(draw(&mut ui, project, &[release], false).0.is_none());
        }

        // The same Program stage can attach to the clip at the playhead.
        canvas.set(false);
        let mut warm = || {
            ready(&mut ui, project);
            let edit = gesture(&mut ui, project, from, program).unwrap();
            assert_eq!(edit.target, Target::Timeline(TimelineId::Clip(clip)));
            // The main loop commits only after paint, so the measurement covers the whole gesture.
        };
        mem_sys::set_peer(pxui::library_counter);
        for _ in 0..8 {
            warm();
        }
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut warm);
        println!("warm effect carry at {width}: zero allocations");
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }
}
