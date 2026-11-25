//! Native Splice viewer, hosted by PXUI.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

use commands::clip_at;
use input::Event;
use ogpu_core::backend::Device;
use pxui::{
    Flags, FontData, FrameIn, Image, KeyCode, Label, Mods, Node, Overlay, Point, Pressable, Rect,
    Rgba, Role, Signal, ThemeHandle, Ui, UiDesc,
    theme::{Look, States, Tokens},
    ui::{Bar, Cursor, Size},
};
use pxui_host::{Accessibility, Events, Platform, Window};
use pxui_sdl::{DialogFilter, DialogKind};
use splice_core::history::{Scope as UndoScope, Store, StoreError, WriteOptions};
use splice_json::Value as Json;
use splice_kit::Invoked;
use splice_mcp::live::{Server, ViewCommand, ViewSelection, ViewTab, Viewer, ViewerZoom};
use splice_model::{Project, SessionId};
use std::{
    borrow::Cow,
    ffi::OsString,
    fmt::Display,
    fs::{self, OpenOptions},
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod commands;
mod editor_lab;
mod editor_palette;
mod editor_session;
mod editor_shortcuts;
mod editor_titlebar;
mod editor_views;
mod inspector;
mod playbar;
mod program;
mod snapshot;
mod surface;
mod timeline;
mod timeline_media;
mod viewer_captions;
mod viewer_chrome;
mod viewer_crop;
mod viewer_drawers;
mod viewer_zoom;

pub use snapshot::render as snapshot;

const VIEWER_SIZE: [u32; 2] = [960, 640];
const DEFAULT_TIMELINE_HEIGHT: f32 = 360.0;

/// Opens a viewer on the application thread. With no project, presents the native open panel.
pub fn open(path: Option<&Path>, state: Option<&Path>) -> Result<(), StoreError> {
    run(path, state, false)
        .map(|_| ())
        .inspect_err(|error| report(error, path, state))
}

/// A person launching from Finder never sees stderr: show the error and log it (T10.4).
fn report(error: &StoreError, project: Option<&Path>, state: Option<&Path>) {
    if let Some(state) = state {
        splice_mcp::friction::viewer_error(Some(state.to_path_buf()), project, error);
    }
    if apple_sys::app::has_launchd_parent() {
        let title = project.and_then(Path::file_name).map_or_else(
            || "Splice could not open the project".to_owned(),
            |name| format!("Splice could not open {}", name.to_string_lossy()),
        );
        apple_sys::app::alert(
            &title,
            &format!("{}\n\n{}\n\n({})", error.message, error.fix, error.code),
        );
    }
}

/// Presents the first available program frame to a hidden window.
pub fn smoke_test(path: Option<&Path>, state: Option<&Path>) -> Result<Option<i32>, StoreError> {
    run(path, state, true)
}

fn run(path: Option<&Path>, state: Option<&Path>, smoke: bool) -> Result<Option<i32>, StoreError> {
    let mut project = path.map(|path| open_store(path, state)).transpose()?;
    let [width, height] = VIEWER_SIZE.map(i64::from);
    let mut window = Window::open_with("Splice", width, height, !smoke, smoke, !smoke)
        .map_err(|error| viewer_error(&error))?;
    window
        .set_resizable(true)
        .map_err(|error| viewer_error(&error))?;
    let mut events = Vec::with_capacity(input::EVENT_RESERVE);
    let last = state.map(|dir| dir.join("last-project"));
    if project.is_none() {
        let open = window.pump(&mut events);
        if let Some(error) = window.take_failure() {
            return Err(viewer_error(&error));
        }
        if !open {
            window.destroy().map_err(|error| viewer_error(&error))?;
            return Ok(None);
        }
        project = project_drop(&events)
            .map(|path| open_store(path, state))
            .transpose()?;
    }
    if project.is_none() && !smoke {
        let recent = last.as_deref().and_then(recall);
        let folder = recent
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::to_str);
        let Some(selected) = choose_project(&mut window, &mut events, folder)? else {
            window.destroy().map_err(|error| viewer_error(&error))?;
            return Ok(None);
        };
        project = Some(open_store(&selected, state)?);
    }
    let result = loop {
        if let (Some(file), Some(store)) = (&last, &project)
            && let Err(error) = remember(file, store.path())
        {
            eprintln!("splice: could not remember the last project: {error}");
        }
        let mut viewer = project.map(|store| Viewer::new(store, state.map(Path::to_path_buf)));
        let server = if !smoke && let (Some(viewer), Some(state)) = (&viewer, state) {
            Some(
                Server::start(viewer.store(), state, pxui_sdl::wake)
                    .map_err(|error| viewer_error(&error))?,
            )
        } else {
            None
        };
        let result = run_surface(
            &mut window,
            &mut events,
            viewer.as_mut(),
            server.as_ref(),
            state,
            smoke,
        );
        drop(server);
        let closed = viewer.map_or(Ok(()), Viewer::close);
        let (shown, next) = result?;
        closed.map_err(|error| viewer_error(&error))?;
        let Some(next) = next else { break shown };
        project = Some(next);
    };
    window.destroy().map_err(|error| viewer_error(&error))?;
    Ok(result)
}

/// Opens the writer, taking the package over from an agent's headless `splice mcp` (T10.1).
fn open_store(path: &Path, state: Option<&Path>) -> Result<Store, StoreError> {
    let state = state
        .map(Path::to_path_buf)
        .or_else(|| splice_mcp::paths::state_dir().ok());
    match (Store::open(path), state) {
        (Err(error), Some(state))
            if error.code == "LOCKED" && splice_mcp::live::handoff(&state, path)? =>
        {
            Store::open(path)
        }
        (result, _) => result,
    }
}

fn project_drop(events: &[Event]) -> Option<&Path> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::FilesDropped {
                paths: Ok(paths), ..
            } => Some(paths.as_slice()),
            _ => None,
        })
        .flatten()
        .map(Path::new)
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("splice"))
        })
}

fn choose_project(
    window: &mut Window,
    events: &mut Vec<Event>,
    folder: Option<&str>,
) -> Result<Option<PathBuf>, StoreError> {
    let filters = [DialogFilter {
        name: "Splice Projects".into(),
        pattern: "splice".into(),
    }];
    let dialog = window
        .show_dialog(&filters, folder, DialogKind::OpenFile)
        .map_err(|error| viewer_error(&error))?;
    loop {
        events.clear();
        let open = window.pump(events);
        if let Some(error) = window.take_failure() {
            return Err(viewer_error(&error));
        }
        if !open {
            return Ok(None);
        }
        if let Some(path) = project_drop(events) {
            apple_sys::app::cancel_open_panel();
            return Ok(Some(path.to_path_buf()));
        }
        for event in events.iter() {
            if let Event::FileDialog { id, result } = event
                && *id == dialog
            {
                let paths = result.as_ref().map_err(|error| viewer_error(error))?;
                if paths.is_empty() {
                    return Ok(None);
                }
                let selection = window
                    .take_dialog_selection(dialog)
                    .map_err(|error| viewer_error(&error))?;
                return Ok(selection.first().map(|file| file.path().to_path_buf()));
            }
        }
        window.wait(None);
    }
}

fn run_surface(
    window: &mut Window,
    events: &mut Vec<Event>,
    mut viewer: Option<&mut Viewer>,
    server: Option<&Server>,
    state: Option<&Path>,
    smoke: bool,
) -> Result<(Option<i32>, Option<Store>), StoreError> {
    let store = viewer.as_deref().map(Viewer::store);
    let name = store
        .as_deref()
        .map(|store| project_name(store).into_owned());
    let mut program = store
        .as_deref()
        .map(program::Program::prepare)
        .transpose()?
        .flatten();
    let mut history = store.as_deref().map(commands::History::new);
    let mut cache = ViewerCache::for_app();
    let mut previews = Vec::new();
    let mut media_key = store.map(|store| timeline_media_key(store.project()));
    let mut media_decoder =
        store.map(|store| timeline_media::Decoder::start(store.project(), store.path()));
    let mut lab = editor_lab::Lab::new();
    let mut palette = PaletteState::default();
    let mut edit_arrows = Vec::with_capacity(input::EVENT_RESERVE);
    let mut session = editor_session::State::default();
    let mut media_dialog = None;
    let mut media_seek = None;
    let mut layout = EditorLayout::new(VIEWER_SIZE.map(|side| side as f32));
    let timeline_preference = state.map(|dir| dir.join("timeline-height"));
    let mut saved_timeline_height = timeline_preference
        .as_deref()
        .and_then(recall_timeline_height)
        .unwrap_or(DEFAULT_TIMELINE_HEIGHT);
    layout.timeline_height = saved_timeline_height;
    let mut revision = store.as_deref().map(|store| store.project().rev);
    let keymap = state.map(|dir| dir.join("keymap"));
    let mut keys = commands::Keys::new(keymap.as_deref());
    let mut ui = viewer_ui()?;
    let mut surface =
        surface::Surface::open(window, &ui, program.as_ref().map(program::Program::size))?;
    let mut accessibility = Accessibility::open(window).map_err(|error| viewer_error(&error))?;
    let mut platform = Platform::default();
    let mut encoded = Events::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut presented = None;
    let mut last_follow_rate = 0;
    let next_project = loop {
        events.clear();
        let open = platform
            .pump(window, events)
            .map_err(|error| viewer_error(&error))?;
        if let Some(error) = window.take_failure() {
            return Err(viewer_error(&error));
        }
        if !open {
            break None;
        }
        let (width, height) = window.size();
        if width > 0 && height > 0 {
            layout.size = [width as f32, height as f32];
        }
        if let Some(path) = project_drop(events) {
            match path.canonicalize() {
                Ok(path)
                    if viewer
                        .as_deref()
                        .is_some_and(|viewer| viewer.store().path() == path) => {}
                Ok(path) => match open_store(&path, None) {
                    Ok(project) => break Some(project),
                    Err(error) => {
                        eprintln!("splice: {error}");
                        report(&error, Some(&path), None);
                    }
                },
                Err(error) => eprintln!("splice: could not open {}: {error}", path.display()),
            }
        }
        if let Some(request) = server.and_then(Server::next_request)
            && let Some(viewer) = viewer.as_deref_mut()
        {
            let at = program.as_ref().map_or(0, program::Program::at);
            let response = viewer.handle(&request.body, at, &mut |store, command| {
                apply_view_command(
                    &mut layout,
                    program.as_mut(),
                    store,
                    command,
                    Some((&lab, keys.bindings())),
                    Some((surface.device(), &previews, media_key)),
                )
            });
            lab.observe_viewer_jobs(&request.body, &response);
            request.reply(response);
        }
        let mut store = viewer.as_deref_mut().map(Viewer::store_mut);
        if let Some(dialog) = media_dialog
            && events
                .iter()
                .any(|event| matches!(event, Event::FileDialog { id, .. } if *id == dialog))
        {
            media_dialog = None;
            match window.take_dialog_selection(dialog) {
                Ok(files) => {
                    let ops: Vec<Json> = files
                        .iter()
                        .filter_map(|file| file.path().to_str())
                        .map(|path| {
                            Json::Object(vec![
                                ("cmd".into(), Json::String("clip.insert".into())),
                                ("path".into(), Json::String(path.into())),
                                ("insert".into(), Json::Bool(false)),
                            ])
                        })
                        .collect();
                    if !ops.is_empty()
                        && let Some(store) = store.as_deref_mut()
                    {
                        apply_ui_ops(store, &ops);
                    }
                }
                Err(error) => eprintln!("splice: {error}"),
            }
        }
        let text_editing = ui.text_input_focused();
        edit_arrows.clear();
        let mut escape_timeline = false;
        let mut pane_action = None;
        let mut repaint = false;
        cache.timeline.wheel.begin(events);
        events.retain(|event| {
            let overlay_open =
                layout.drawer.is_some() || layout.sheet.is_some() || layout.timeline.menu_open();
            let editor = layout.view == editor_titlebar::View::Editor;
            if cache.timeline.wheel.route(
                &ui,
                &mut layout.timeline,
                event,
                editor && !text_editing && layout.drawer.is_none() && layout.sheet.is_none(),
            ) {
                repaint = true;
                return false;
            }
            if editor
                && !overlay_open
                && let Event::PointerMoved((x, y)) | Event::PointerPressed(_, (x, y)) = *event
            {
                let point = (x as f32, y as f32);
                if layout.program_bounds.contains(point) {
                    layout.program_active = true;
                } else if layout.timeline_bounds.contains(point) {
                    layout.program_active = false;
                }
            }
            let mut consumed = false;
            let mut defer = false;
            let routed = keys.route(event, &mut |action| {
                let global = matches!(action.id, commands::PALETTE | commands::SHORTCUTS);
                if overlay_open || (!global && (text_editing || !editor)) {
                    return;
                }
                if matches!(action.id, commands::PREVIOUS_EDIT | commands::NEXT_EDIT)
                    && let Event::KeyPressed(input::Key::ArrowUp | input::Key::ArrowDown) = *event
                {
                    let down = matches!(*event, Event::KeyPressed(input::Key::ArrowDown));
                    edit_arrows.push((action, down));
                    defer = true;
                    return;
                }
                if action.id == commands::ESCAPE {
                    escape_timeline = true;
                    defer = true;
                    return;
                }
                consumed = true;
                if global {
                    layout.sheet = Some(if action.id == commands::PALETTE {
                        palette.open();
                        EditorSheet::Palette
                    } else {
                        EditorSheet::Shortcuts
                    });
                    return;
                }
                if action.id == commands::CROP {
                    pane_action = Some(viewer_chrome::PaneAction::Crop);
                    return;
                }
                if let Err(error) = dispatch_editor(
                    action,
                    &mut layout,
                    store.as_deref_mut(),
                    history.as_mut(),
                    program.as_mut(),
                    &mut session,
                    &cache.timeline,
                ) {
                    eprintln!("splice: {error}");
                }
            });
            defer || !(consumed || (routed && editor && !overlay_open && !text_editing))
        });
        if let Some(store) = store.as_deref()
            && revision != Some(store.project().rev)
        {
            revision = Some(store.project().rev);
            layout.timeline.retain_selection(store.project());
            if matches!(layout.sheet, Some(EditorSheet::Crop(_))) {
                layout.sheet = None;
            }
            if let Some(history) = &mut history {
                history.sync(store);
            }
            let at = media_seek
                .take()
                .unwrap_or_else(|| program.as_ref().map_or(0, program::Program::at));
            let rate = program.as_ref().map_or(0, program::Program::rate);
            let mut next = match program::Program::prepare(store) {
                Ok(program) => program,
                Err(error) => {
                    eprintln!("splice: {error}");
                    None
                }
            };
            if let Some(next) = &mut next {
                next.seek(at)?;
                if (rate > 0 && next.at() < next.end() - 1) || (rate < 0 && next.at() > 0) {
                    next.set_rate(rate);
                }
            }
            let size = next.as_ref().map(program::Program::size);
            let same_surface = size == program.as_ref().map(program::Program::size);
            let new_media_key = timeline_media_key(store.project());
            if same_surface {
                surface.clear_program()?;
                if media_key != Some(new_media_key) {
                    surface.clear_thumbnails()?;
                }
            } else {
                surface.close()?;
                surface = surface::Surface::open(window, &ui, size)?;
            }
            if !same_surface || media_key != Some(new_media_key) {
                previews.clear();
                media_key = Some(new_media_key);
                media_decoder = Some(timeline_media::Decoder::start(
                    store.project(),
                    store.path(),
                ));
            }
            program = next;
            presented = None;
        }
        if !matches!(layout.sheet, Some(EditorSheet::Crop(_))) && surface.crop_preview().is_some() {
            surface.set_crop_preview(None)?;
        }
        if let Some(decoder) = &media_decoder {
            for _ in 0..2 {
                let Some(decoded) = decoder.take() else { break };
                let mut upload = |source: Option<(Vec<u8>, [u32; 2])>| {
                    source.and_then(
                        |(pixels, size)| match surface.add_thumbnail(&pixels, size) {
                            Ok(image) => Some(image),
                            Err(error) => {
                                eprintln!("splice: {error}");
                                None
                            }
                        },
                    )
                };
                let thumbnail = upload(decoded.thumbnail);
                let camera = upload(decoded.camera);
                let index = previews.partition_point(|preview: &timeline::MediaPreview| {
                    preview.id.serial() < decoded.id.serial()
                });
                previews.insert(
                    index,
                    timeline::MediaPreview {
                        id: decoded.id,
                        thumbnail,
                        waveform: decoded.waveform,
                        camera,
                        camera_only: decoded.camera_only,
                        mic: decoded.mic,
                        system: decoded.system,
                        events: decoded.events,
                    },
                );
                repaint = true;
            }
        }
        let mut call = None;
        session.refresh(store.as_deref(), layout.activity_session(store.as_deref()));
        if layout.view == editor_titlebar::View::Agent {
            surface.refresh_lab_frames(
                lab.frame_reply()
                    .zip(store.as_deref())
                    .map(|((id, files), store)| (id, files, store.path())),
            )?;
        }
        if surface.begin(window)? {
            let fps = store.as_deref().map_or(30, |store| store.project().fps);
            if let Some(program) = &mut program {
                let rate = program.rate();
                program.sample(&mut surface)?;
                if rate != 0 {
                    if rate != last_follow_rate {
                        layout.timeline.resume_follow();
                    }
                    if layout.view == editor_titlebar::View::Editor {
                        layout
                            .timeline
                            .follow(layout.size[0], program.at(), program.end(), fps);
                    }
                }
                last_follow_rate = program.rate();
            } else {
                last_follow_rate = 0;
            }
            let input = FrameIn::new(layout.size[0], layout.size[1])
                .scale(window.scale() as f32)
                .time(window.time())
                .environment(platform.environment())
                .keyboard(platform.keyboard())
                .events(encoded.encode(events));
            let mut intents = ViewerIntents::default();
            intents.timeline.state = layout.timeline.clone();
            let mut host_events = events.as_slice();
            let mut inspector_focus = layout.inspector_focus.take().filter(|_| {
                !events
                    .iter()
                    .any(|event| matches!(event, Event::PointerPressed(..) | Event::FocusLost))
            });
            let (at, end, rate, dragging) = program.as_ref().map_or((0, 0, 0, false), |program| {
                (
                    program.at(),
                    program.end(),
                    program.rate(),
                    program.dragging(),
                )
            });
            if layout.view == editor_titlebar::View::Agent {
                lab.refresh(
                    store.as_deref(),
                    at,
                    end,
                    layout.timeline.selection(),
                    layout.timeline.lane(),
                );
            }
            let playing = rate != 0;
            let play = commands::states(end > 0 && !dragging, playing, (false, false))
                [commands::PLAY.0 as usize];
            if matches!(layout.sheet, Some(EditorSheet::Palette)) {
                let history = store
                    .as_deref()
                    .map_or((false, false), Store::human_history_available);
                palette.states = commands::states(end > 0 && !dragging, playing, history);
                let project = store.as_deref().map(Store::project);
                let selected = layout.timeline.selection();
                for (id, enabled) in [
                    (commands::PALETTE, false),
                    (
                        commands::CROP,
                        project.is_some_and(|p| crop_target(p, selected, at).is_some()),
                    ),
                    (
                        commands::SPLIT,
                        project.is_some_and(|p| {
                            timeline::split_at_playhead(p, selected, at).is_some()
                        }),
                    ),
                    (commands::SESSION_UNDO, session.undoable().is_some()),
                    (commands::ADD_MOTION, project.is_some()),
                    (
                        commands::REPLACE_SPINE,
                        project.is_some_and(|project| {
                            timeline::selected_media_overlay(project, layout.timeline.selected())
                                .is_some()
                        }),
                    ),
                ] {
                    palette.states[id.0 as usize].enabled = enabled;
                }
                for id in commands::EDIT_SHORTCUTS {
                    palette.states[id.0 as usize].enabled = project.is_some_and(|project| {
                        edit_shortcut_enabled(id, project, &layout.timeline, at)
                    });
                }
                for id in commands::CLIPBOARD_SHORTCUTS {
                    palette.states[id.0 as usize].enabled = project.is_some()
                        && timeline::ClipboardAction::command(id)
                            .is_some_and(|action| layout.timeline.clipboard_enabled(action));
                }
                for id in commands::SELECTION_SHORTCUTS {
                    palette.states[id.0 as usize].enabled = project.is_some_and(|project| {
                        selection_shortcut_enabled(id, project, &layout.timeline, end)
                    });
                }
            }
            accessibility.frame(&mut ui, input.wire.surface);
            let a11y_revision = ui.a11y_tree().revision;
            let mut unhandled_arrows = [false; 2];
            ui.frame(&input, &mut |ui| {
                let next = paint_viewer(
                    ui,
                    name.as_deref(),
                    (
                        store.as_deref().map(Store::project),
                        Some((&lab, surface.lab_images())),
                        Some(&palette),
                        store.as_deref().map(|store| (&session, store)),
                        keys.bindings(),
                    ),
                    EditorLayout {
                        timeline: intents.timeline.state.clone(),
                        program: intents.program_view.unwrap_or(layout.program),
                        inspector_focus: inspector_focus.take(),
                        ..layout.clone()
                    },
                    (
                        [surface.matte(), surface.image(), surface.crop_preview()],
                        std::mem::take(&mut host_events),
                    ),
                    (
                        &mut cache,
                        &previews,
                        program.as_ref().map(program::Program::evaluator),
                    ),
                    playbar::View {
                        at,
                        end,
                        fps,
                        play,
                        rate,
                        dragging,
                    },
                );
                if !edit_arrows.is_empty() && !next.divider_focused && !ui.text_input_focused() {
                    unhandled_arrows[0] |= ui.key_pressed(KeyCode::ArrowUp, Mods::ANY);
                    unhandled_arrows[1] |= ui.key_pressed(KeyCode::ArrowDown, Mods::ANY);
                }
                intents.merge(next);
            })
            .map_err(|error| viewer_error(&error))?;
            // The accessibility bridge publishes the previous frame's tree.
            repaint |= ui.a11y_tree().revision != a11y_revision;
            if escape_timeline && !ui.text_input_focused() {
                if intents.timeline.escape(&layout.timeline, at) {
                    layout.scope = inspector::Scope::Selection;
                }
                repaint = true;
            }
            layout.program_bounds = intents.program_bounds;
            layout.timeline_bounds = intents.timeline.bounds;
            repaint |= inspector::reveal_focus(&mut ui, &intents.inspector);
            platform
                .apply(window, &ui)
                .map_err(|error| viewer_error(&error))?;
            if surface.draw(window, &ui)? {
                presented = program.as_ref().and_then(program::Program::shown);
                if smoke && (program.is_none() || presented.is_some()) {
                    break None;
                }
            }
            palette.focus_search = false;
            if let Some(query) = intents.palette_query {
                palette.query = query;
                repaint = true;
            }
            if let Some(selected) = intents.palette_selection {
                palette.selected = selected;
                repaint = true;
            }
            if intents.dismiss_drawer {
                layout.drawer = None;
                repaint = true;
            }
            if intents.dismiss_sheet {
                layout.sheet = None;
                repaint = true;
            } else if let Some(sheet) = intents.update_sheet {
                layout.sheet = Some(sheet);
                repaint = true;
            }
            if let Some(program_view) = intents.program_view {
                layout.program = program_view;
                repaint = true;
            }
            let palette_crop = intents
                .command
                .filter(|action| action.id == commands::CROP)
                .map(|_| viewer_chrome::PaneAction::Crop);
            let inspector_crop = intents.inspector.crop.or(intents.timeline.crop);
            if let Some(pane) = pane_action
                .or(palette_crop)
                .or(inspector_crop.map(|_| viewer_chrome::PaneAction::Crop))
                .or(intents.pane)
            {
                match pane {
                    viewer_chrome::PaneAction::Media => {
                        layout.sheet = None;
                        layout.drawer = Some(viewer_drawers::Drawer::Media)
                    }
                    viewer_chrome::PaneAction::Effects => {
                        layout.sheet = None;
                        layout.drawer = Some(viewer_drawers::Drawer::Effects)
                    }
                    viewer_chrome::PaneAction::Zoom => {
                        layout.drawer = None;
                        layout.sheet = Some(EditorSheet::Zoom(None));
                    }
                    viewer_chrome::PaneAction::Captions => {
                        layout.drawer = None;
                        layout.sheet = Some(EditorSheet::Captions);
                    }
                    viewer_chrome::PaneAction::Crop => {
                        layout.drawer = None;
                        let draft = store.as_deref().and_then(|store| {
                            if let Some(id) = inspector_crop {
                                viewer_crop::Draft::new(store.project(), id)
                            } else {
                                crop_target(store.project(), layout.timeline.selection(), at)
                            }
                        });
                        if let (Some(draft), Some(store)) = (draft, store.as_deref()) {
                            if let Some(program) = &mut program {
                                program.set_rate(0);
                            }
                            match draft.preview(store.project(), store.path(), at) {
                                Ok((pixels, size)) => {
                                    surface.set_crop_preview(Some((&pixels, size)))?
                                }
                                Err(error) => {
                                    surface.set_crop_preview(None)?;
                                    eprintln!("splice: {error}");
                                }
                            }
                            intents
                                .timeline
                                .state
                                .set_selection(Some(draft.target()), None);
                            layout.scope = inspector::Scope::Selection;
                            layout.sheet = Some(EditorSheet::Crop(draft));
                        }
                    }
                }
                repaint = true;
            }
            if intents.drawer == Some(viewer_drawers::Action::Import) && media_dialog.is_none() {
                match window.show_dialog(&[], None, DialogKind::OpenFiles) {
                    Ok(dialog) => media_dialog = Some(dialog),
                    Err(error) => eprintln!("splice: {error}"),
                }
            }
            repaint |= layout.lab_scroll.settle(&mut ui, &intents.lab);
            let mut session_action = intents.lab.session;
            if intents.lab.changed() {
                call = lab.apply(intents.lab).map(|call| (call, ReplyTarget::Lab));
                repaint = true;
            }
            if let Some(action) = intents.titlebar {
                layout.drawer = None;
                match action {
                    editor_titlebar::Action::View(view) => {
                        layout.show_view(view);
                        layout.sheet = None;
                    }
                    editor_titlebar::Action::CommandPalette => {
                        palette.open();
                        layout.sheet = Some(EditorSheet::Palette)
                    }
                    editor_titlebar::Action::Shortcuts => {
                        layout.sheet = Some(EditorSheet::Shortcuts)
                    }
                    editor_titlebar::Action::Session(action) => session_action = Some(action),
                }
                repaint = true;
            }
            if let Some(action) = session_action {
                use editor_session::Action;
                match action {
                    Action::OpenActivity(id) => layout.show_activity(id),
                    Action::End(id) => {
                        let args = Json::Object(vec![
                            ("projectId".into(), Json::String("prj_1".into())),
                            ("sessionId".into(), Json::String(id.to_string())),
                        ]);
                        call = Some((
                            editor_lab::Call {
                                name: "session.end",
                                args,
                            },
                            ReplyTarget::Session,
                        ));
                    }
                    Action::UndoAll(id) => {
                        if let Some(store) = store.as_deref_mut()
                            && let Err(error) = undo_session(store, id)
                        {
                            eprintln!("splice: {error}");
                        }
                    }
                    Action::UndoBatch {
                        session,
                        revision,
                        if_revision,
                    } => {
                        if let Some(store) = store.as_deref_mut() {
                            let options = WriteOptions {
                                session: Some(session),
                                if_revision: Some(if_revision),
                                time: now_millis(),
                                ..WriteOptions::default()
                            };
                            if let Err(error) = store.undo(UndoScope::Batch(revision), &options) {
                                eprintln!("splice: {error}");
                            }
                        }
                    }
                }
                repaint = true;
            }
            if let Some(action) = intents.caption_action.or(intents
                .inspector
                .copy_srt
                .then_some(viewer_captions::Action::CopySrt))
            {
                use viewer_captions::Action;
                match action {
                    Action::EditLane(lane) => {
                        intents.timeline.state.set_selection(None, Some(lane));
                        layout.scope = inspector::Scope::Selection;
                    }
                    Action::CopySrt => {
                        let args = Json::Object(vec![
                            ("projectId".into(), Json::String("prj_1".into())),
                            ("captions".into(), Json::String("srt".into())),
                        ]);
                        call = Some((
                            editor_lab::Call {
                                name: "transcript.get",
                                args,
                            },
                            ReplyTarget::Clipboard,
                        ));
                    }
                    Action::SetStyle { lane, style } => {
                        intents.pane_op = Some(caption_appearance(
                            lane.to_string(),
                            "params",
                            Json::Object(vec![(
                                "preset".into(),
                                Json::String(style.label().into()),
                            )]),
                        ));
                    }
                    Action::SetVisibility { lane, enabled } => {
                        intents.pane_op = Some(caption_appearance(
                            lane.to_string(),
                            "enabled",
                            Json::Bool(enabled),
                        ));
                    }
                }
                repaint = true;
            }
            let mut run_parity = false;
            if let Some(mut selection) = intents.view_selection {
                run_parity = selection.take_parity_request();
                layout.view_selection = selection.resolve();
                repaint = true;
            }
            repaint |= cache.parity.advance(run_parity, keys.bindings());
            if let Some(action) = intents.command {
                if action.id == commands::SHORTCUTS {
                    layout.sheet = Some(EditorSheet::Shortcuts);
                } else if action.id != commands::CROP {
                    dispatch_editor(
                        action,
                        &mut layout,
                        store.as_deref_mut(),
                        history.as_mut(),
                        program.as_mut(),
                        &mut session,
                        &cache.timeline,
                    )?;
                    intents.timeline.state = layout.timeline.clone();
                }
                repaint = true;
            }
            if let Some(next_scope) = intents.inspector.scope {
                layout.scope = next_scope;
                repaint = true;
            }
            if let Some(clip) = intents.inspector.select_parent {
                intents
                    .timeline
                    .state
                    .set_selection(Some(splice_model::TimelineId::Clip(clip)), None);
                layout.scope = inspector::Scope::Selection;
                repaint = true;
            } else if let Some(lane) = intents.inspector.select_lane {
                intents.timeline.state.set_selection(None, Some(lane));
                layout.scope = inspector::Scope::Selection;
                repaint = true;
            } else if intents.timeline.component_selected {
                layout.scope = inspector::Scope::Selection;
                repaint = true;
            }
            if let Some(focus) = intents.timeline.inspector_focus {
                layout.inspector_focus = Some(focus);
                layout.scope = inspector::Scope::Selection;
                repaint = true;
            }
            let primary_ops = intents
                .inspector
                .op
                .or(intents.pane_op)
                .map(|op| vec![op])
                .or_else(|| {
                    intents
                        .timeline
                        .toolbar
                        .map(timeline::ToolbarCommand::operations)
                })
                .or_else(|| {
                    let (item, at) = intents.timeline.split?;
                    split_operation(store.as_deref()?.project(), item, at).map(|op| vec![op])
                })
                .or_else(|| {
                    intents
                        .timeline
                        .component
                        .map(|(clip, component, enabled)| {
                            vec![Json::Object(vec![
                                ("cmd".into(), Json::String("component.set".into())),
                                ("clip".into(), Json::String(clip.to_string())),
                                ("component".into(), Json::String(component.into())),
                                ("enabled".into(), Json::Bool(enabled)),
                            ])]
                        })
                });
            let mut appended = false;
            let mut bundled_sample = None;
            let operations = if let Some(ops) = primary_ops {
                Some(ops)
            } else if let Some(moved) = intents.timeline.moved {
                Some(move_operations(moved))
            } else {
                intents
                    .timeline
                    .trimmed
                    .map(trim_operation)
                    .or_else(|| {
                        let action = intents.drawer?;
                        let store = store.as_deref()?;
                        if !cache
                            .media_sources
                            .allows(store.project(), store.path(), &action)
                        {
                            eprintln!("splice: media source is unavailable");
                            return None;
                        }
                        use viewer_drawers::Action;
                        appended = matches!(action, Action::Append(_) | Action::Sample(_));
                        if let Action::Sample(sample) = action {
                            bundled_sample = Some(sample);
                        }
                        drawer_operation(action, store.project(), at)
                    })
                    .map(|op| vec![op])
            };
            let result = store.as_deref_mut().and_then(|store| {
                if let Some(sample) = bundled_sample {
                    Some(
                        cache
                            .media_sources
                            .append_sample(store, sample, now_millis()),
                    )
                } else {
                    operations.map(|ops| apply_editor_batch(store, &ops))
                }
            });
            if let Some(result) = result
                && let Some(store) = store.as_deref()
            {
                match result {
                    Ok(result) if appended => {
                        if let Some((clip, start, take)) =
                            viewer_drawers::appended_clip(store.project(), &result.report)
                        {
                            intents
                                .timeline
                                .state
                                .set_selection(Some(splice_model::TimelineId::Clip(clip)), None);
                            if take {
                                intents.timeline.state.expand(clip);
                            }
                            layout.scope = inspector::Scope::Selection;
                            media_seek = Some(start);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => eprintln!("splice: {error}"),
                }
                repaint = true;
            }
            if let Some(at) = intents.inspector.seek
                && let Some(program) = &mut program
            {
                program.set_rate(0);
                program.seek(at)?;
                repaint = true;
            }
            repaint |= layout.timeline != intents.timeline.state;
            layout.timeline = intents.timeline.state;
            if let Some(height) = intents.timeline_height {
                repaint |= layout.timeline_height != height;
                layout.timeline_height = height;
            }
            if intents.timeline_height_committed
                && layout.timeline_height != saved_timeline_height
                && let Some(file) = &timeline_preference
            {
                let height = layout.timeline_height;
                match remember_bytes(file, format!("{height:.0}").as_bytes()) {
                    Ok(()) => saved_timeline_height = height,
                    Err(error) => eprintln!("splice: could not save the timeline height: {error}"),
                }
            }
            if let Some((phase, at)) = intents.timeline.scrub
                && let Some(program) = &mut program
            {
                program.scrub(phase, at)?;
                repaint = true;
            }
            if let Some(action) = intents.timeline.clipboard
                && let Some(store) = store.as_deref_mut()
            {
                dispatch_clipboard(action, &mut layout, store, program.as_mut())?;
                repaint = true;
            }
            // Let focused widgets and the timeline settle before changing the playhead and pan.
            for (action, down) in edit_arrows.drain(..) {
                if unhandled_arrows[usize::from(down)] {
                    dispatch_editor(
                        action,
                        &mut layout,
                        store.as_deref_mut(),
                        history.as_mut(),
                        program.as_mut(),
                        &mut session,
                        &cache.timeline,
                    )?;
                    repaint = true;
                }
            }
            if let Some(action) = intents.transport
                && let Some(program) = &mut program
            {
                use viewer_chrome::TransportAction;
                match action {
                    TransportAction::Play => dispatch(
                        keys.play_button(),
                        store.as_deref_mut(),
                        history.as_mut(),
                        Some(program),
                    )?,
                    TransportAction::Back => program.step(-1)?,
                    TransportAction::Forward => program.step(1)?,
                    TransportAction::Start | TransportAction::End => {
                        program.set_rate(0);
                        let at = if action == TransportAction::Start {
                            0
                        } else {
                            program.end() - 1
                        };
                        program.seek(at)?;
                    }
                }
                repaint = true;
            }
        }
        if call.is_none() {
            let now = window.time();
            repaint |= lab.demo_deadline().is_some_and(|due| due <= now);
            let end = program.as_ref().map_or(0, program::Program::end);
            call = lab
                .poll_demo(now, store.as_deref(), end)
                .map(|call| (call, ReplyTarget::Demo));
        }
        if let Some((call, target)) = call
            && let Some(viewer) = viewer.as_deref_mut()
        {
            let body = Json::Object(vec![
                ("command".into(), Json::String("tool".into())),
                ("name".into(), Json::String(call.name.into())),
                ("args".into(), call.args),
                ("client".into(), Json::String("native-lab".into())),
            ]);
            let at = program.as_ref().map_or(0, program::Program::at);
            let reply = viewer.handle(&body, at, &mut |store, command| {
                apply_view_command(
                    &mut layout,
                    program.as_mut(),
                    store,
                    command,
                    Some((&lab, keys.bindings())),
                    Some((surface.device(), &previews, media_key)),
                )
            });
            let payload = reply
                .field("result")
                .and_then(|result| result.field("structuredContent"));
            if matches!(target, ReplyTarget::Lab | ReplyTarget::Demo) {
                lab.set_reply(
                    call.name,
                    body.field("args").expect("Lab call arguments"),
                    &reply,
                );
                if target == ReplyTarget::Demo {
                    lab.demo_replied(window.time(), &reply);
                }
            } else if let Some(error) = reply.field("error").or_else(|| payload?.field("error")) {
                eprintln!(
                    "splice: {}",
                    error
                        .field("message")
                        .and_then(Json::as_str)
                        .unwrap_or("Editor action failed.")
                );
            } else if target == ReplyTarget::Clipboard
                && let Some(srt) = payload
                    .and_then(|result| result.field("text"))
                    .and_then(Json::as_str)
                && let Err(error) = pxui_sdl::clipboard_set_text(srt)
            {
                eprintln!("splice: {error}");
            }
            repaint = true;
        }
        // Agent exports encode on this thread between frames (OGPU's handles live here).
        let exporting = viewer
            .as_deref_mut()
            .is_some_and(|viewer| viewer.tick(Duration::from_millis(6)));
        let waiting = exporting || program.as_ref().is_some_and(program::Program::wants_frame);
        if smoke && Instant::now() >= deadline {
            return Err(viewer_error(
                &"Timed out presenting the first program frame.",
            ));
        }
        let next = if waiting || repaint || smoke {
            Some(window.time() + 1.0 / 120.0)
        } else {
            ui.repaint_at()
        };
        let next = match (next, lab.demo_deadline()) {
            (Some(frame), Some(demo)) => Some(frame.min(demo)),
            (next, demo) => next.or(demo),
        };
        window.wait(next);
    };
    drop(accessibility);
    surface.close()?;
    Ok((presented, next_project))
}

/// Writes one offscreen viewer frame and returns the output path and revision.
pub fn snapshot_output(store: &Store, at: i32, output: &Path) -> Result<Json, StoreError> {
    write_snapshot_output(
        store,
        at,
        output,
        EditorLayout::new(VIEWER_SIZE.map(|extent| extent as f32)),
        None,
        None,
    )
}

fn write_snapshot_output(
    store: &Store,
    at: i32,
    output: &Path,
    layout: EditorLayout,
    agent: Option<(&editor_lab::Lab, &[commands::BindingDescription])>,
    media: Option<(&Device, &[timeline::MediaPreview])>,
) -> Result<Json, StoreError> {
    let mut offline = Vec::new();
    let file = splice_mcp::write_png(output, &mut || {
        let (png, fields) = snapshot::render_layout(store, at, layout.clone(), agent, media)?;
        offline = fields;
        Ok(png)
    })?;
    let mut fields = vec![
        ("output".into(), Json::String(file.to_string_lossy().into())),
        (
            "revision".into(),
            Json::String(format!("r{}", store.project().rev)),
        ),
        ("frame".into(), Json::Signed(i64::from(at))),
    ];
    fields.extend(offline.into_iter().map(|(key, value)| (key.into(), value)));
    Ok(Json::Object(fields))
}

fn apply_view_command(
    layout: &mut EditorLayout,
    program: Option<&mut program::Program>,
    store: &Store,
    command: ViewCommand,
    agent: Option<(&editor_lab::Lab, &[commands::BindingDescription])>,
    media: Option<(&Device, &[timeline::MediaPreview], Option<u64>)>,
) -> Result<Json, StoreError> {
    match command {
        ViewCommand::Show {
            at,
            select,
            expand,
            range,
            view,
            zoom,
        } => {
            if select.as_ref().is_some_and(|selection| {
                selection.len() > 1
                    && selection
                        .iter()
                        .any(|id| matches!(id, ViewSelection::Lane(_)))
            }) {
                return Err(StoreError::new(
                    "BAD_ARGS",
                    "Lane selection must name one lane on its own.",
                    "Select a lane by itself or pass clip, overlay, or marker IDs.",
                ));
            }
            if let Some(at) = at
                && let Some(program) = program
            {
                program.seek(at)?;
            }
            layout.timeline.retain_selection(store.project());
            if let Some(select) = select {
                match select.as_slice() {
                    [ViewSelection::Lane(lane)] => layout.timeline.set_selection(None, Some(*lane)),
                    [ViewSelection::Timeline(id)] => layout.timeline.set_selection(Some(*id), None),
                    [] => layout.timeline.set_selection(None, None),
                    _ => layout.timeline.set_selected(
                        select
                            .into_iter()
                            .filter_map(|id| match id {
                                ViewSelection::Timeline(id) => Some(id),
                                ViewSelection::Lane(_) => None,
                            })
                            .collect(),
                    ),
                }
            }
            if let Some(clip) = expand {
                layout.timeline.expand(clip);
            }
            if let Some(range) = range {
                layout.timeline.frame_range(range);
            }
            if let Some(view) = view {
                layout.show_view(match view {
                    ViewTab::Editor => editor_titlebar::View::Editor,
                    ViewTab::Agent => editor_titlebar::View::Agent,
                    ViewTab::Spec => editor_titlebar::View::Spec,
                });
                layout.sheet = None;
                layout.drawer = None;
            }
            if let Some(zoom) = zoom {
                layout.program = match zoom {
                    ViewerZoom::Fit => program::View::default(),
                    ViewerZoom::Factor(scale) => program::View::at_scale(scale),
                };
            }
            Ok(Json::Array(
                layout
                    .timeline
                    .selected()
                    .iter()
                    .map(|id| Json::String(id.to_string()))
                    .chain(
                        layout
                            .timeline
                            .lane()
                            .map(|id| Json::String(id.to_string())),
                    )
                    .collect(),
            ))
        }
        ViewCommand::Snapshot { at, output } => {
            layout.timeline.retain_selection(store.project());
            let media = media.map(|(device, previews, key)| {
                let previews = if key == Some(timeline_media_key(store.project())) {
                    previews
                } else {
                    &[]
                };
                (device, previews)
            });
            write_snapshot_output(store, at, &output, layout.clone(), agent, media)
        }
    }
}

fn dispatch(
    action: Invoked,
    store: Option<&mut Store>,
    history: Option<&mut commands::History>,
    program: Option<&mut program::Program>,
) -> Result<(), StoreError> {
    use commands::{
        BACK, BACK_SECOND, END, FORWARD, FORWARD_SECOND, PLAY, REDO, SHUTTLE_FORWARD,
        SHUTTLE_REVERSE, START, STOP, UNDO,
    };
    let enabled = program.as_ref().is_some_and(|program| !program.dragging());
    let playing = program.as_ref().is_some_and(|program| program.playing());
    let availability = store
        .as_deref()
        .map_or((false, false), Store::human_history_available);
    if !commands::states(enabled, playing, availability)[action.id.0 as usize].enabled {
        return Ok(());
    }
    match (action.id, program, store, history) {
        (PLAY, Some(program), _, _) => program.toggle(),
        (SHUTTLE_REVERSE, Some(program), _, _) => program.shuttle(-1),
        (SHUTTLE_FORWARD, Some(program), _, _) => program.shuttle(1),
        (STOP, Some(program), _, _) => program.shuttle(0),
        (BACK, Some(program), _, _) => program.step(-1),
        (FORWARD, Some(program), _, _) => program.step(1),
        (BACK_SECOND | FORWARD_SECOND, Some(program), store, _) => {
            let fps = store.map_or(30, |store| store.project().fps) as i32;
            program.step(if action.id == BACK_SECOND { -fps } else { fps })
        }
        (START | END, Some(program), _, _) => {
            program.set_rate(0);
            program.seek(if action.id == START {
                0
            } else {
                program.end() - 1
            })
        }
        (UNDO | REDO, _, Some(store), Some(history)) => {
            history.invoke(store, action.id, now_millis()).map(|_| ())
        }
        _ => Ok(()),
    }
}

fn dispatch_editor(
    action: Invoked,
    layout: &mut EditorLayout,
    mut store: Option<&mut Store>,
    history: Option<&mut commands::History>,
    program: Option<&mut program::Program>,
    session: &mut editor_session::State,
    timeline_cache: &timeline::Cache,
) -> Result<(), StoreError> {
    use commands::{
        ADD_MOTION, BLADE_TOOL, ESCAPE, FIT, LANE_DOWN, LANE_UP, LIFT_TO_OVERLAY, NEXT_EDIT,
        NUDGE_BACK, NUDGE_BACK_TEN, NUDGE_FORWARD, NUDGE_FORWARD_TEN, PREVIOUS_EDIT, REPLACE_SPINE,
        SELECT_LANE_ITEMS, SELECT_TOOL, SESSION_UNDO, SNAP, SPLIT, ZOOM_IN, ZOOM_OUT,
        ZOOM_SELECTION, ZOOM_TIMELINE_IN, ZOOM_TIMELINE_OUT,
    };
    use splice_model::TimelineId;
    use std::rc::Rc;
    if let Some(action) = timeline::ClipboardAction::command(action.id) {
        if let Some(store) = store {
            dispatch_clipboard(action, layout, store, program)?;
        }
        return Ok(());
    }
    let (at, end) = program
        .as_deref()
        .map_or((0, 0), |program| (program.at(), program.end()));
    match action.id {
        PREVIOUS_EDIT | NEXT_EDIT => {
            if let Some(store) = store.as_deref()
                && let Some(program) = program
                && !program.dragging()
                && let Some(target) =
                    timeline_cache.edit_point(store.project(), at, end, action.id == NEXT_EDIT)
            {
                program.seek(target)?;
                layout
                    .timeline
                    .follow(layout.size[0], target, end, store.project().fps);
            }
        }
        SELECT_TOOL => layout.timeline.select_tool(),
        BLADE_TOOL => layout.timeline.toggle_blade(),
        SNAP => layout.timeline.toggle_snap(),
        FIT if layout.program_active => layout.program = program::View::default(),
        FIT => layout.timeline.fit(),
        ZOOM_IN | ZOOM_OUT | ZOOM_TIMELINE_IN | ZOOM_TIMELINE_OUT => {
            if let Some(store) = store.as_deref() {
                let project = store.project();
                if layout.program_active && matches!(action.id, ZOOM_IN | ZOOM_OUT) {
                    let stage = [layout.program_bounds.w, layout.program_bounds.h];
                    let factor = if action.id == ZOOM_IN { 1.25 } else { 0.8 };
                    if stage.iter().all(|&side| side > 0.0) {
                        layout.program = layout.program.step_zoom(factor, stage, project.size);
                    }
                } else {
                    let factor = if matches!(action.id, ZOOM_IN | ZOOM_TIMELINE_IN) {
                        1.4
                    } else {
                        1.0 / 1.4
                    };
                    layout
                        .timeline
                        .zoom_about(factor, layout.size[0], at, end, project.fps);
                }
            }
        }
        ZOOM_SELECTION => {
            if let Some(store) = store.as_deref() {
                layout
                    .timeline
                    .zoom_selection(store.project(), layout.size[0], end);
            }
        }
        SELECT_LANE_ITEMS => {
            if let Some(store) = store.as_deref() {
                layout.timeline.select_current_lane(store.project());
                layout.scope = inspector::Scope::Selection;
            }
        }
        NUDGE_BACK | NUDGE_FORWARD | NUDGE_BACK_TEN | NUDGE_FORWARD_TEN => {
            let delta = match action.id {
                NUDGE_BACK => -1,
                NUDGE_FORWARD => 1,
                NUDGE_BACK_TEN => -10,
                _ => 10,
            };
            if let Some(store) = store
                && let Some(command) = layout.timeline.nudge(store.project(), delta)
            {
                apply_ui_ops(store, &[command.operation()]);
            } else if let Some(program) = program {
                program.seek(at.saturating_add(delta))?;
            }
        }
        LIFT_TO_OVERLAY | LANE_UP | LANE_DOWN => {
            if let Some(store) = store {
                let ids: Vec<_> = layout
                    .timeline
                    .command_selection()
                    .iter()
                    .copied()
                    .filter(|id| match action.id {
                        LIFT_TO_OVERLAY => matches!(id, TimelineId::Clip(_)),
                        _ => matches!(id, TimelineId::Item(_)),
                    })
                    .collect();
                if !ids.is_empty() {
                    let ids = Rc::from(ids);
                    let command = match action.id {
                        LIFT_TO_OVERLAY => timeline::ToolbarCommand::LiftGroup(ids),
                        LANE_UP => timeline::ToolbarCommand::LaneStep(ids, "up"),
                        _ => timeline::ToolbarCommand::LaneStep(ids, "down"),
                    };
                    apply_ui_ops(store, &[command.operation()]);
                }
            }
        }
        ESCAPE => {
            let mut intents = timeline::Intents::default();
            intents.state = layout.timeline.clone();
            if intents.escape(&layout.timeline, at) {
                layout.scope = inspector::Scope::Selection;
            }
            layout.timeline = intents.state;
            if let Some((phase, at)) = intents.scrub
                && let Some(program) = program
            {
                program.scrub(phase, at)?;
            }
        }
        SESSION_UNDO => {
            session.refresh(store.as_deref(), None);
            if let Some(store) = store.as_deref_mut()
                && let Some(id) = session.undoable()
            {
                undo_session(store, id)?;
            }
        }
        SPLIT => {
            if let Some(store) = store {
                let project = store.project();
                let selected = layout.timeline.selection();
                if let Some(item) = timeline::split_at_playhead(project, selected, at)
                    && let Some(op) = split_operation(project, item, at)
                {
                    apply_ui_ops(store, &[op]);
                }
            }
        }
        ADD_MOTION => {
            if let Some(store) = store {
                apply_ui_ops(
                    store,
                    &[timeline::ToolbarCommand::AddMotion(at).operation()],
                );
            }
        }
        REPLACE_SPINE => {
            if let Some(store) = store
                && let Some(item) =
                    timeline::selected_media_overlay(store.project(), layout.timeline.selected())
            {
                apply_ui_ops(
                    store,
                    &[timeline::ToolbarCommand::ReplaceSpine(item).operation()],
                );
            }
        }
        id if commands::EDIT_SHORTCUTS.contains(&id) => {
            if let Some(store) = store
                && let Some(start) = dispatch_edit_shortcut(id, store, &mut layout.timeline, at)
                && let Some(program) = program
            {
                program.seek(start)?;
            }
        }
        _ => return dispatch(action, store, history, program),
    }
    Ok(())
}

fn selection_shortcut_enabled(
    id: splice_kit::CommandId,
    project: &Project,
    state: &timeline::State,
    end: i32,
) -> bool {
    use commands::{
        ESCAPE, LANE_DOWN, LANE_UP, LIFT_TO_OVERLAY, NUDGE_BACK, NUDGE_BACK_TEN, NUDGE_FORWARD,
        NUDGE_FORWARD_TEN, SELECT_LANE_ITEMS, ZOOM_SELECTION, ZOOM_TIMELINE_IN, ZOOM_TIMELINE_OUT,
    };
    use splice_model::TimelineId;

    match id {
        NUDGE_BACK | NUDGE_FORWARD | NUDGE_BACK_TEN | NUDGE_FORWARD_TEN => {
            end > 0 || !state.command_selection().is_empty()
        }
        SELECT_LANE_ITEMS => {
            !project.clips.order().is_empty()
                || matches!(state.selection(), Some(TimelineId::Item(_)))
        }
        LIFT_TO_OVERLAY => state
            .command_selection()
            .iter()
            .any(|id| matches!(id, TimelineId::Clip(_))),
        LANE_UP | LANE_DOWN => state
            .command_selection()
            .iter()
            .any(|id| matches!(id, TimelineId::Item(_))),
        ZOOM_SELECTION => state.zoom_range(project).is_some(),
        ZOOM_TIMELINE_IN | ZOOM_TIMELINE_OUT => end > 0,
        ESCAPE => state.can_escape(),
        _ => false,
    }
}

fn edit_shortcut_enabled(
    id: splice_kit::CommandId,
    project: &Project,
    state: &timeline::State,
    at: i32,
) -> bool {
    let component_delete = state.component().map(|_| {
        matches!(id, commands::DELETE | commands::DELETE_GAP)
            && timeline::component_visibility(project, state, false).is_some()
    });
    commands::edit_enabled(id, project, state.selected(), component_delete, at)
}

fn dispatch_edit_shortcut(
    id: splice_kit::CommandId,
    store: &mut Store,
    state: &mut timeline::State,
    at: i32,
) -> Option<i32> {
    use commands::{ADD_MARKER, ADD_TITLE, DELETE, DELETE_GAP, DUPLICATE, TRIM_END, TRIM_START};
    use splice_model::TimelineId;
    use std::rc::Rc;
    use timeline::ToolbarCommand;

    if !edit_shortcut_enabled(id, store.project(), state, at) {
        return None;
    }
    let mut seek = None;
    let operation = match id {
        TRIM_START | TRIM_END => {
            let (clip, timing) = commands::trim_target(store.project(), at)?;
            let ripple = state.ripple();
            let (edge, boundary) = if id == TRIM_START {
                seek = ripple.then_some(timing.start);
                ("in", timing.start)
            } else {
                ("out", timing.end()?)
            };
            trim_operation((TimelineId::Clip(clip), edge, at - boundary, ripple))
        }
        _ => match id {
            ADD_TITLE => ToolbarCommand::AddTitle(at),
            ADD_MARKER => ToolbarCommand::AddMarker(at),
            DELETE | DELETE_GAP if state.component().is_some() => {
                timeline::component_visibility(store.project(), state, false)?.1
            }
            DELETE | DELETE_GAP => {
                timeline::selection_delete(state, id == DELETE && state.ripple())?
            }
            DUPLICATE => ToolbarCommand::Duplicate(Rc::from(state.selected())),
            _ => unreachable!("editing shortcut"),
        }
        .operation(),
    };
    let before = store.project().rev;
    apply_ui_ops(store, &[operation]);
    if store.project().rev == before {
        return None;
    }
    if id == ADD_MARKER
        && let Some(marker) = store.project().markers.iter().find(|m| m.at == at)
    {
        state.set_selection(Some(TimelineId::Marker(marker.id)), None);
    }
    seek
}

fn project_name(store: &Store) -> Cow<'_, str> {
    let name = store
        .path()
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy();
    let offline = store
        .project()
        .media
        .iter()
        .filter(|media| !store.path().join(&media.path).exists())
        .count();
    if offline == 0 {
        name
    } else {
        Cow::Owned(format!("{name} · {offline} media offline"))
    }
}

fn viewer_ui() -> Result<Ui, StoreError> {
    let fonts = [
        FontData(include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf")),
        FontData(include_bytes!(
            "../assets/fonts/BarlowSemiCondensed-Light.ttf"
        )),
    ];
    let mut ui = Ui::new(&UiDesc::DEFAULT.fonts(&fonts)).map_err(|error| viewer_error(&error))?;
    editor_palette::install(&mut ui).map_err(|error| viewer_error(&error))?;
    ui.text_service()
        .enable_system_fonts()
        .map_err(|error| viewer_error(&error))?;
    let mut tokens = Tokens::DEFAULT;
    tokens.panel = Rgba::rgb(15, 15, 16);
    tokens.foreground = Rgba::rgb(244, 243, 238);
    tokens.control = Rgba::rgb(20, 20, 21);
    tokens.input = Rgba::rgb(11, 11, 12);
    tokens.track = Rgba::rgb(37, 37, 39);
    tokens.accent = Rgba::rgb(255, 138, 92);
    tokens.dark = true;
    let mut theme = ThemeHandle::from_tokens(tokens);
    theme.animate_swap(false);
    ui.set_theme(&theme).map_err(|error| viewer_error(&error))?;
    Ok(ui)
}

const EDITOR_HEADING: Label = Label::BODY.font(1).weight(300);

#[derive(Clone)]
struct EditorLayout {
    size: [f32; 2],
    timeline: timeline::State,
    timeline_height: f32,
    program: program::View,
    program_bounds: Rect,
    timeline_bounds: Rect,
    program_active: bool,
    scope: inspector::Scope,
    inspector_focus: Option<inspector::Focus>,
    view: editor_titlebar::View,
    view_selection: editor_views::Selection,
    lab_scroll: editor_lab::Scroll,
    activity_session: Option<SessionId>,
    sheet: Option<EditorSheet>,
    drawer: Option<viewer_drawers::Drawer>,
}

impl EditorLayout {
    fn new(size: [f32; 2]) -> Self {
        Self {
            size,
            timeline: timeline::State::default(),
            timeline_height: DEFAULT_TIMELINE_HEIGHT,
            program: program::View::default(),
            program_bounds: Rect::default(),
            timeline_bounds: Rect::default(),
            program_active: false,
            scope: inspector::Scope::default(),
            inspector_focus: None,
            view: editor_titlebar::View::default(),
            view_selection: editor_views::Selection::default(),
            lab_scroll: editor_lab::Scroll::default(),
            activity_session: None,
            sheet: None,
            drawer: None,
        }
    }

    fn show_view(&mut self, view: editor_titlebar::View) {
        if self.view != view && view == editor_titlebar::View::Agent {
            self.lab_scroll.restore();
        }
        if view != editor_titlebar::View::Agent {
            self.activity_session = None;
        }
        self.view = view;
    }

    fn show_activity(&mut self, id: SessionId) {
        self.show_view(editor_titlebar::View::Agent);
        self.activity_session = Some(id);
        self.sheet = None;
        self.drawer = None;
    }

    fn activity_session(&self, store: Option<&Store>) -> Option<SessionId> {
        if self.view != editor_titlebar::View::Agent {
            return None;
        }
        self.activity_session.or_else(|| {
            store
                .and_then(|store| store.sessions().last())
                .map(|session| session.id)
        })
    }
}

#[derive(Clone, Copy)]
enum EditorSheet {
    Palette,
    Shortcuts,
    Crop(viewer_crop::Draft),
    Captions,
    Zoom(Option<Point>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplyTarget {
    Lab,
    Demo,
    Session,
    Clipboard,
}

struct PaletteState {
    query: String,
    selected: Option<splice_kit::CommandId>,
    focus_search: bool,
    states: [splice_kit::CommandState; commands::COUNT],
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected: None,
            focus_search: false,
            states: [splice_kit::CommandState::default(); commands::COUNT],
        }
    }
}

impl PaletteState {
    fn open(&mut self) {
        self.query.clear();
        self.selected = None;
        self.focus_search = true;
    }
}

#[derive(Default)]
struct ViewerIntents {
    timeline: timeline::Intents,
    titlebar: Option<editor_titlebar::Action>,
    pane: Option<viewer_chrome::PaneAction>,
    program_view: Option<program::View>,
    program_bounds: Rect,
    caption_action: Option<viewer_captions::Action>,
    drawer: Option<viewer_drawers::Action>,
    dismiss_drawer: bool,
    view_selection: Option<editor_views::Selection>,
    lab: editor_lab::Intents,
    dismiss_sheet: bool,
    update_sheet: Option<EditorSheet>,
    pane_op: Option<Json>,
    command: Option<Invoked>,
    palette_query: Option<String>,
    palette_selection: Option<Option<splice_kit::CommandId>>,
    timeline_height: Option<f32>,
    timeline_height_committed: bool,
    divider_focused: bool,
    transport: Option<viewer_chrome::TransportAction>,
    inspector: inspector::Intents,
}

impl ViewerIntents {
    fn merge(&mut self, next: Self) {
        // PXUI may rebuild after input to settle geometry. Those passes have no new events.
        self.timeline.merge(next.timeline);
        self.titlebar = self.titlebar.or(next.titlebar);
        self.pane = self.pane.or(next.pane);
        self.program_view = next.program_view.or(self.program_view);
        self.program_bounds = next.program_bounds;
        self.caption_action = self.caption_action.or(next.caption_action);
        self.drawer = self.drawer.take().or(next.drawer);
        self.dismiss_drawer |= next.dismiss_drawer;
        self.view_selection = self.view_selection.or(next.view_selection);
        self.lab.merge(next.lab);
        self.dismiss_sheet |= next.dismiss_sheet;
        self.update_sheet = next.update_sheet.or(self.update_sheet);
        self.pane_op = self.pane_op.take().or(next.pane_op);
        self.command = self.command.or(next.command);
        self.palette_query = self.palette_query.take().or(next.palette_query);
        self.palette_selection = self.palette_selection.or(next.palette_selection);
        self.timeline_height = self.timeline_height.or(next.timeline_height);
        self.timeline_height_committed |= next.timeline_height_committed;
        self.transport = self.transport.or(next.transport);
        self.inspector.scope = self.inspector.scope.or(next.inspector.scope);
        self.inspector.select_parent = self
            .inspector
            .select_parent
            .or(next.inspector.select_parent);
        self.inspector.crop = self.inspector.crop.or(next.inspector.crop);
        self.inspector.select_lane = self.inspector.select_lane.or(next.inspector.select_lane);
        self.inspector.copy_srt |= next.inspector.copy_srt;
        self.inspector.seek = self.inspector.seek.or(next.inspector.seek);
        self.inspector.focused_field = self
            .inspector
            .focused_field
            .or(next.inspector.focused_field);
        self.inspector.focus_scroll = self.inspector.focus_scroll.or(next.inspector.focus_scroll);
        self.inspector.op = self.inspector.op.take().or(next.inspector.op);
    }
}

#[derive(Default)]
struct ViewerCache {
    parity: editor_views::Parity,
    timeline: timeline::Cache,
    captions: inspector::captions::Cache,
    media_sources: viewer_drawers::MediaSources,
    pane_triggers: viewer_chrome::Triggers,
}

impl ViewerCache {
    fn for_app() -> Self {
        Self {
            media_sources: viewer_drawers::MediaSources::with_resources(
                splice_mcp::paths::resources_dir().ok().flatten(),
            ),
            ..Self::default()
        }
    }
}

fn paint_viewer(
    ui: &mut Ui,
    name: Option<&str>,
    (project, lab, palette, session, bindings): (
        Option<&Project>,
        Option<(&editor_lab::Lab, &[Option<Image>])>,
        Option<&PaletteState>,
        Option<(&editor_session::State, &Store)>,
        &[commands::BindingDescription],
    ),
    layout: EditorLayout,
    ([matte, image, crop_preview], events): ([Option<Image>; 3], &[Event]),
    (cache, previews, evaluator): (
        &mut ViewerCache,
        &[timeline::MediaPreview],
        Option<&splice_av::Evaluator>,
    ),
    view: playbar::View,
) -> ViewerIntents {
    let mut intents = ViewerIntents::default();
    let EditorLayout {
        size,
        timeline,
        timeline_height,
        program: viewport,
        scope,
        inspector_focus,
        view: active_view,
        view_selection,
        sheet,
        drawer,
        ..
    } = layout;
    let stacked = size[0] <= 820.0;
    let narrow = size[0] <= 480.0;
    let revision = project.map(|project| project.rev);
    let session_state = session.map(|(state, _)| state);
    let titlebar = editor_titlebar::Layout::new(size[0], revision, session_state);
    let editor_height = size[1] - titlebar.height();
    let timeline_limit = (editor_height - 186.0).max(82.0);
    let min_timeline = 150.0_f32.min(timeline_limit);
    let timeline_height = if stacked {
        420.0
    } else {
        timeline_height.clamp(min_timeline, timeline_limit)
    };
    let inspector_width = if stacked { size[0] } else { 268.0 };
    let viewer_width = if stacked {
        size[0]
    } else {
        size[0] - inspector_width - 1.0
    };
    let viewer_height = if stacked {
        364.0
    } else {
        (editor_height - timeline_height - 5.0).max(64.0)
    };
    let canvas = project.map(|project| project.size);
    let stage = [viewer_width, viewer_height - 64.0];
    let zoom_percent = canvas.map_or(0, |canvas| viewport.zoom_percent(stage, canvas));
    let stage_events = if sheet.is_some() || drawer.is_some() {
        &[]
    } else {
        events
    };
    // Build popups before the editor to shield its input. The trigger is from the retained
    // toolbar, while the vertical anchor uses this frame's titlebar and the page's painted offset.
    let toolbar_anchor = |ui: &Ui, trigger| {
        let scroll = if stacked {
            ui.box_of_key("compact-editor")
                .map_or(0.0, |page| ui.scroll(&page).painted)
        } else {
            0.0
        };
        Point::new(
            cache.pane_triggers.x(ui, trigger),
            (titlebar.height() + viewer_chrome::PANE_HEIGHT - scroll).max(4.0),
        )
    };
    if let Some(sheet) = sheet {
        match sheet {
            EditorSheet::Palette => {
                if let Some(palette) = palette {
                    let options = Overlay::DEFAULT
                        .w_px((size[0] - 32.0).clamp(180.0, 620.0))
                        .name("Command palette");
                    let modal = ui.modal("command-palette", &options, |ui| {
                        let result = editor_palette::paint(
                            ui,
                            &palette.query,
                            (&palette.states, bindings),
                            palette.selected,
                            palette.focus_search,
                        );
                        intents.palette_query = result.query;
                        if result.selected != palette.selected {
                            intents.palette_selection = Some(result.selected);
                        }
                        intents.command = result.command.map(|id| Invoked {
                            id,
                            source: splice_kit::Source::Palette,
                            target: 1,
                        });
                        intents.dismiss_sheet = !result.open || result.command.is_some();
                    });
                    intents.dismiss_sheet |= !modal.open;
                }
            }
            EditorSheet::Shortcuts => {
                intents.dismiss_sheet |= editor_shortcuts::paint(ui, bindings, size);
            }
            EditorSheet::Zoom(context) => {
                let (at, max_height) = if let Some(at) = context {
                    (at, size[1] - 8.0)
                } else {
                    let at = toolbar_anchor(ui, viewer_chrome::PaneAction::Zoom);
                    (at, size[1] - at.y - 4.0)
                };
                let result = viewer_zoom::paint(
                    ui,
                    at,
                    max_height,
                    canvas.map_or(1.0, |canvas| viewport.display_scale(stage, canvas)),
                    viewport.fitted(),
                );
                intents.dismiss_sheet = !result.open || result.action.is_some();
                if let Some(action) = result.action {
                    use viewer_zoom::Action;
                    intents.program_view = match action {
                        Action::Fit => Some(program::View::default()),
                        Action::In => canvas.map(|canvas| viewport.step_zoom(1.25, stage, canvas)),
                        Action::Out => canvas.map(|canvas| viewport.step_zoom(0.8, stage, canvas)),
                        Action::Percent(percent) => canvas
                            .map(|canvas| viewport.zoom_to(percent as f32 * 0.01, stage, canvas)),
                        Action::Crop => {
                            intents.pane = Some(viewer_chrome::PaneAction::Crop);
                            None
                        }
                    };
                }
            }
            EditorSheet::Crop(_) => {}
            EditorSheet::Captions => {
                let at = toolbar_anchor(ui, viewer_chrome::PaneAction::Captions);
                let result = viewer_captions::paint(ui, project, size, at);
                intents.dismiss_sheet =
                    !result.open || result.generate.is_some() || result.action.is_some();
                intents.caption_action = result.action;
                if let Some(settings) = result.generate {
                    let mut fields = vec![
                        ("cmd".into(), Json::String("captions.generate".into())),
                        ("maxWords".into(), Json::Unsigned(settings.max_words as u64)),
                        ("fillers".into(), Json::Bool(settings.fillers)),
                    ];
                    if let Some(style) = settings.style {
                        fields.push(("style".into(), Json::String(style.label().into())));
                    }
                    if let Some(lane) = settings.lane {
                        fields.push(("lane".into(), Json::String(lane.to_string())));
                    }
                    intents.pane_op = Some(Json::Object(fields));
                }
            }
        }
    } else if let Some(drawer) = drawer {
        let trigger = match drawer {
            viewer_drawers::Drawer::Media => viewer_chrome::PaneAction::Media,
            viewer_drawers::Drawer::Effects => viewer_chrome::PaneAction::Effects,
        };
        let at = toolbar_anchor(ui, trigger);
        let sources = session.map(|(_, store)| (store.path(), &mut cache.media_sources));
        let result = viewer_drawers::paint(
            ui,
            drawer,
            (project, sources),
            size,
            at,
            project.map_or_else(viewer_drawers::EffectTargets::default, |project| {
                drawer_targets(project, &timeline, view.at)
            }),
        );
        intents.dismiss_drawer = !result.open;
        intents.drawer = result.action;
    }
    let mut program_effect = None;
    let mut content = |ui: &mut Ui| {
        let mut paint_titlebar = |ui: &mut Ui| {
            intents.titlebar =
                editor_titlebar::paint(ui, name, revision, active_view, &titlebar, session_state);
        };
        if active_view == editor_titlebar::View::Spec {
            intents.view_selection = editor_views::paint_spec(
                ui,
                view_selection,
                (bindings, &cache.parity),
                &mut paint_titlebar,
            );
            intents.timeline.state = timeline.clone();
            return;
        }
        paint_titlebar(ui);
        if active_view != editor_titlebar::View::Editor {
            if active_view == editor_titlebar::View::Agent
                && let Some((lab, images)) = lab
            {
                intents.lab = editor_lab::paint(
                    ui,
                    project
                        .map(|project| (project.rev, session.and_then(|(state, _)| state.open()))),
                    lab,
                    images,
                    session,
                    titlebar.height(),
                );
            } else {
                intents.view_selection = editor_views::paint_with_parity(
                    ui,
                    project,
                    active_view,
                    view_selection,
                    bindings,
                    &cache.parity,
                );
            }
            intents.timeline.state = timeline.clone();
            return;
        }
        let component = timeline.component();
        let selection = if let Some((clip, part, key)) = component {
            inspector::Selection::Component(clip, part, key)
        } else {
            match timeline.selection() {
                Some(splice_model::TimelineId::Clip(id)) => inspector::Selection::Clip(id),
                Some(splice_model::TimelineId::Item(id)) => inspector::Selection::Item(id),
                Some(splice_model::TimelineId::Marker(id)) => inspector::Selection::Marker(id),
                None => timeline
                    .lane()
                    .map_or(inspector::Selection::None, inspector::Selection::Lane),
            }
        };
        cache.captions.refresh(
            ui,
            project
                .filter(|_| scope == inspector::Scope::Selection && timeline.selected().len() <= 1),
            selection,
        );
        let mut paint_program = |ui: &mut Ui, intents: &mut ViewerIntents| {
            ui.column(Node::DEFAULT.w(Size::Grow).h_px(viewer_height), |ui| {
                let aspect = project.map_or("9:16", |project| project.aspect.label());
                let (pane, triggers) =
                    viewer_chrome::pane(ui, aspect, viewport.fitted(), zoom_percent, narrow);
                cache.pane_triggers = triggers;
                intents.pane = intents.pane.or(pane);
                if let Some(EditorSheet::Crop(draft)) = sheet {
                    let name = project
                        .and_then(|project| draft.name(project))
                        .unwrap_or("Clip");
                    match viewer_crop::paint(ui, draft, name, crop_preview, stage) {
                        viewer_crop::Action::None => {}
                        viewer_crop::Action::Update(draft) => {
                            intents.update_sheet = Some(EditorSheet::Crop(draft));
                        }
                        viewer_crop::Action::Apply(op) => {
                            intents.pane_op = Some(op);
                            intents.dismiss_sheet = true;
                        }
                        viewer_crop::Action::Cancel => intents.dismiss_sheet = true,
                    }
                } else {
                    let guide = project.and_then(|project| {
                        viewer_captions::safe_guide(project, timeline.selection(), timeline.lane())
                    });
                    let (next, stage, context) =
                        program::paint(ui, canvas, matte, image, viewport, stage_events, guide);
                    intents.program_bounds = ui.rect(&stage);
                    if let Some(project) = project {
                        let target = if scope == inspector::Scope::Canvas {
                            Some(splice_core::commands::Target::Project)
                        } else {
                            clip_at(project, view.at).map(|clip| {
                                splice_core::commands::Target::Timeline(
                                    splice_model::TimelineId::Clip(clip),
                                )
                            })
                        };
                        program_effect = target.and_then(|target| {
                            viewer_drawers::effect_drop(ui, &stage, project, target)
                        });
                    }
                    if sheet.is_none() && drawer.is_none() {
                        if next != viewport {
                            intents.program_view = Some(next);
                        }
                        if let Some(context) = context {
                            intents.update_sheet = Some(EditorSheet::Zoom(Some(context)));
                        }
                    }
                }
                intents.transport = viewer_chrome::transport(ui, view, narrow);
            });
        };
        let paint_inspector = |ui: &mut Ui, intents: &mut ViewerIntents| {
            let events = project
                .zip(component)
                .and_then(|(project, (clip, part, _))| {
                    if part != splice_core::commands::Component::Keys {
                        return None;
                    }
                    let slot = project.clips.find(clip)?;
                    let (data, timing) =
                        project.clips.data(slot).zip(project.clips.timing(slot))?;
                    let events = &timeline::preview_for(previews, data.media?)?.events;
                    let clock = evaluator
                        .and_then(|evaluator| evaluator.clip_clock(clip, timing.start))
                        .or_else(|| {
                            timeline::affine_event_span(timing, data, project.fps, events.fps)
                                .map(|span| (span, &[][..]))
                        })?;
                    Some((events, clock))
                });
            intents.inspector = inspector::paint(
                ui,
                project,
                selection,
                scope,
                view.end,
                (inspector_width, stacked),
                (
                    timeline.selected(),
                    timeline.ripple(),
                    events,
                    Some((&cache.captions, view.at)),
                    inspector_focus,
                ),
            );
        };
        let mut paint_timeline = |ui: &mut Ui, intents: &mut ViewerIntents| {
            intents.timeline = timeline::paint(
                ui,
                project,
                timeline::View {
                    width: size[0],
                    height: timeline_height,
                    at: view.at,
                    end: view.end,
                    dragging: view.dragging,
                    history: session
                        .map_or((false, false), |(_, store)| store.human_history_available()),
                    state: timeline.clone(),
                },
                &mut cache.timeline,
                (previews, evaluator),
            );
            if let Some(id) = intents.timeline.history {
                intents.command = intents.command.or(Some(Invoked {
                    id,
                    source: splice_kit::Source::Control,
                    target: 1,
                }));
            }
        };
        if stacked {
            paint_program(ui, &mut intents);
            paint_timeline(ui, &mut intents);
            paint_inspector(ui, &mut intents);
            return;
        }
        ui.row(Node::DEFAULT.w(Size::Grow).h_px(viewer_height), |ui| {
            paint_program(ui, &mut intents);
            ui.node(
                "viewer-inspector-divider",
                Node::DEFAULT
                    .w_px(1.0)
                    .h(Size::Grow)
                    .look(Look::NONE.fill(Rgba::rgb(37, 37, 39))),
            );
            paint_inspector(ui, &mut intents);
        });
        ui.stack(Node::DEFAULT.w(Size::Grow).h_px(5.0), |ui| {
            let line = Look::NONE.fill(Rgba::rgb(37, 37, 39));
            let active = Look::NONE.fill(Rgba::rgb(56, 56, 59));
            let looks = States::of(line).hover(active).pressed(active).focus(active);
            let divider = ui.pressable(
                "timeline-divider",
                Node::DEFAULT.w(Size::Grow).h_px(5.0).role(Role::Separator),
                &Pressable::DEFAULT.look(&looks).focus_on_click(true),
                |_, _| {},
            );
            ui.a11y_name(
                &divider.handle,
                "Resize timeline with Up or Down. Double-click to reset",
            );
            let grip = Look::NONE.fill(Rgba::rgb(91, 91, 94));
            for (key, y) in [("grip-top", 1.0), ("grip-bottom", 3.0)] {
                ui.node(
                    key,
                    Node::DEFAULT
                        .at((size[0] - 28.0).max(0.0) * 0.5, y)
                        .w_px(28.0)
                        .h_px(1.0)
                        .look(grip),
                );
            }
            let signal = ui.signal(&divider.handle);
            intents.divider_focused = divider.focused;
            let key_delta = if divider.focused {
                let up = ui.key_pressed(KeyCode::ArrowUp, Mods::ANY);
                let down = ui.key_pressed(KeyCode::ArrowDown, Mods::ANY);
                f32::from(i8::from(up) - i8::from(down)) * 24.0
            } else {
                0.0
            };
            if signal.hovered || signal.held {
                ui.request_cursor(Cursor::VerticalResize);
            }
            if key_delta != 0.0 {
                intents.timeline_height =
                    Some((timeline_height + key_delta).clamp(min_timeline, timeline_limit));
                intents.timeline_height_committed = true;
            } else if signal.double_clicked || (divider.clicked && signal.button.is_none()) {
                intents.timeline_height = Some(DEFAULT_TIMELINE_HEIGHT);
                intents.timeline_height_committed = true;
            } else if signal.button == Some(Signal::BUTTON_PRIMARY) && signal.drag.1 != 0.0 {
                intents.timeline_height =
                    Some((timeline_height - signal.drag.1).clamp(min_timeline, timeline_limit));
            }
            intents.timeline_height_committed |= signal.released;
        });
        paint_timeline(ui, &mut intents);
    };
    let frame = Node::DEFAULT.w(Size::Grow).h(Size::Grow);
    if stacked && active_view == editor_titlebar::View::Editor {
        let scroll = ui.node(
            "compact-editor",
            frame
                .flags(Flags::SCROLL | Flags::CLIP | Flags::OVER_THUMB | Flags::BLOCKING)
                .bar(Bar::new(Rgba::rgb(56, 56, 59), 6.0).gap(2.0))
                .role(Role::ScrollArea),
        );
        ui.a11y_name(&scroll, "Editor");
        ui.within(&scroll, &mut content);
        if intents.inspector.focused_field.is_some() {
            intents.inspector.focus_scroll = Some(scroll);
        }
    } else {
        ui.column(frame, content);
    }
    if let Some(effect) = intents.timeline.effect.take().or(program_effect) {
        use splice_core::commands::Target;
        match effect.target {
            Target::Timeline(id) => {
                intents.timeline.state.set_selection(Some(id), None);
                intents.inspector.scope = Some(inspector::Scope::Selection);
            }
            Target::Lane(id) => {
                intents.timeline.state.set_selection(None, Some(id));
                intents.inspector.scope = Some(inspector::Scope::Selection);
            }
            Target::Project => intents.inspector.scope = Some(inspector::Scope::Canvas),
            _ => {}
        }
        intents.drawer = Some(viewer_drawers::Action::Effect(effect));
        intents.dismiss_drawer = true;
    }
    intents
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| u64::try_from(time.as_millis()).ok())
        .unwrap_or(0)
}

fn undo_session(store: &mut Store, session: SessionId) -> Result<(), StoreError> {
    let options = WriteOptions {
        session: Some(session),
        time: now_millis(),
        ..WriteOptions::default()
    };
    store.undo(UndoScope::Session, &options).map(|_| ())
}

fn move_operations(moved: timeline::MoveIntent) -> Vec<Json> {
    let mut operations =
        Vec::with_capacity(usize::from(moved.to.is_some()) + usize::from(moved.lane.is_some()));
    if let Some(to) = moved.to {
        operations.push(Json::Object(vec![
            ("cmd".into(), Json::String("item.move".into())),
            (
                "items".into(),
                Json::Array(vec![Json::String(moved.item.to_string())]),
            ),
            ("to".into(), Json::Unsigned(to.max(0) as u64)),
            ("ripple".into(), Json::Bool(moved.ripple)),
            ("carryOverlays".into(), Json::Bool(moved.carry_overlays)),
        ]));
    }
    if let Some(lane) = moved.lane {
        operations.push(Json::Object(vec![
            ("cmd".into(), Json::String("item.lane".into())),
            (
                "items".into(),
                Json::Array(vec![Json::String(moved.item.to_string())]),
            ),
            ("lane".into(), Json::String(lane.to_string())),
        ]));
    }
    operations
}

fn apply_editor_batch(
    store: &mut Store,
    ops: &[Json],
) -> Result<splice_core::history::EditResult, StoreError> {
    let options = WriteOptions {
        time: now_millis(),
        ..WriteOptions::default()
    };
    splice_mcp::apply_batch(store, ops, &options)
}

fn apply_ui_ops(store: &mut Store, ops: &[Json]) {
    if let Err(error) = apply_editor_batch(store, ops) {
        eprintln!("splice: {error}");
    }
}

fn dispatch_clipboard(
    action: timeline::ClipboardAction,
    layout: &mut EditorLayout,
    store: &mut Store,
    program: Option<&mut program::Program>,
) -> Result<(), StoreError> {
    let at = if let Some(program) = program {
        if let timeline::ClipboardAction::PasteHere(at) = action {
            program.seek(at)?;
        }
        program.at()
    } else {
        0
    };
    let Some(op) = layout
        .timeline
        .clipboard_operation(action, store.project(), at)
    else {
        return Ok(());
    };
    match apply_editor_batch(store, &[op]) {
        Ok(receipt) => {
            if layout
                .timeline
                .clipboard_committed(action, receipt.report.changes)
            {
                layout.scope = inspector::Scope::Selection;
            }
        }
        Err(error) => {
            layout.timeline.clipboard_failed(&error);
            eprintln!("splice: {error}");
        }
    }
    Ok(())
}

fn caption_appearance(target: String, field: &str, value: Json) -> Json {
    Json::Object(vec![
        ("cmd".into(), Json::String("effect.set".into())),
        ("target".into(), Json::String(target)),
        ("type".into(), Json::String("captions".into())),
        (field.into(), value),
    ])
}

fn trim_operation(
    (item, edge, delta, ripple): (splice_model::TimelineId, &'static str, i32, bool),
) -> Json {
    let delta = if delta < 0 {
        Json::Signed(i64::from(delta))
    } else {
        Json::Unsigned(delta as u64)
    };
    Json::Object(vec![
        ("cmd".into(), Json::String("item.trim".into())),
        ("item".into(), Json::String(item.to_string())),
        ("edge".into(), Json::String(edge.into())),
        ("delta".into(), delta),
        ("ripple".into(), Json::Bool(ripple)),
    ])
}

fn split_operation(project: &Project, item: splice_model::TimelineId, at: i32) -> Option<Json> {
    use splice_model::{ItemKind, TimelineId, timing};
    if let TimelineId::Item(id) = item {
        let slot = project.items.find(id)?;
        if let ItemKind::Caption { words, text, .. } = &project.items.data(slot)?.kind {
            if text.is_some() {
                return None;
            }
            let start = timing::anchor_start(project, project.items.timing(slot)?.anchor).ok()?;
            let near = clip_at(project, start);
            let word = words.iter().skip(1).find(|&&id| {
                timing::word_span(project, id, None, near)
                    .ok()
                    .flatten()
                    .is_some_and(|span| span.start >= at.saturating_sub(1))
            })?;
            return Some(inspector::captions::split_at(id, *word));
        }
    }
    Some(Json::Object(vec![
        ("cmd".into(), Json::String("item.split".into())),
        ("item".into(), Json::String(item.to_string())),
        ("at".into(), Json::Unsigned(at as u64)),
    ]))
}

fn drawer_targets(
    project: &Project,
    timeline: &timeline::State,
    at: i32,
) -> viewer_drawers::EffectTargets {
    use splice_core::commands::Target;
    use splice_model::TimelineId;

    if timeline.selected().len() > 1 {
        return viewer_drawers::EffectTargets::default();
    }
    let item = timeline
        .selection()
        .filter(|id| matches!(id, TimelineId::Clip(_) | TimelineId::Item(_)))
        .or_else(|| clip_at(project, at).map(TimelineId::Clip));
    let lane = timeline.lane();
    viewer_drawers::EffectTargets {
        selection: lane
            .map(Target::Lane)
            .or_else(|| item.map(Target::Timeline)),
        lane: lane.or_else(|| match item? {
            TimelineId::Clip(_) => Some(project.spine.id),
            TimelineId::Item(id) => Some(project.items.data(project.items.find(id)?)?.lane),
            TimelineId::Marker(_) => None,
        }),
    }
}

fn drawer_operation(action: viewer_drawers::Action, project: &Project, at: i32) -> Option<Json> {
    use viewer_drawers::Action;
    Some(match action {
        Action::Import | Action::Sample(_) => return None,
        Action::Append(id) => Json::Object(vec![
            ("cmd".into(), Json::String("clip.insert".into())),
            ("media".into(), Json::String(id.to_string())),
        ]),
        Action::Overlay(id) => Json::Object(vec![
            ("cmd".into(), Json::String("overlay.add".into())),
            ("kind".into(), Json::String("media".into())),
            ("media".into(), Json::String(id.to_string())),
            ("at".into(), Json::Unsigned(at.max(0) as u64)),
        ]),
        Action::Effect(effect) => return effect.operation(project),
    })
}

fn timeline_media_key(project: &Project) -> u64 {
    let mut hash = DefaultHasher::new();
    project.fps.hash(&mut hash);
    for media in &project.media {
        (media.id, &media.path, media.size, media.mtime).hash(&mut hash);
        (
            media.probe.frames,
            media.probe.fps.to_bits(),
            media.probe.width,
            media.probe.height,
        )
            .hash(&mut hash);
        std::mem::discriminant(&media.kind).hash(&mut hash);
        if let Some(take) = &media.take {
            (&take.screen, &take.camera, &take.mic, &take.system).hash(&mut hash);
        }
    }
    hash.finish()
}

fn crop_target(
    project: &Project,
    selection: Option<splice_model::TimelineId>,
    at: i32,
) -> Option<viewer_crop::Draft> {
    use splice_model::{TimelineId, timing};
    let draft = |id| viewer_crop::Draft::new(project, id);
    selection
        .and_then(draft)
        .or_else(|| draft(TimelineId::Clip(clip_at(project, at)?)))
        .or_else(|| {
            project.items.order().iter().find_map(|&slot| {
                let item = project.items.timing(slot)?;
                let start = timing::anchor_start(project, item.anchor).ok()?;
                if (start..start.saturating_add(item.length)).contains(&at) {
                    draft(TimelineId::Item(project.items.id(slot)?))
                } else {
                    None
                }
            })
        })
}

fn recall(file: &Path) -> Option<PathBuf> {
    if !fs::symlink_metadata(file).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 || bytes.contains(&0) {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    (path.is_absolute() && path.extension().is_some_and(|ext| ext == "splice") && path.is_dir())
        .then_some(path)
}

fn remember(file: &Path, project: &Path) -> io::Result<()> {
    remember_bytes(file, project.as_os_str().as_bytes())
}

fn recall_timeline_height(file: &Path) -> Option<f32> {
    if !fs::symlink_metadata(file).ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    fs::File::open(file)
        .ok()?
        .take(16)
        .read_to_string(&mut text)
        .ok()?;
    let height = text.trim().parse::<u16>().ok()?;
    (82..=16_384).contains(&height).then_some(f32::from(height))
}

fn remember_bytes(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = file.parent().expect("state file path");
    fs::create_dir_all(parent)?;
    let staged = file.with_extension(format!("pending-{}", std::process::id()));
    let mut temp = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staged)?;
    let result = temp
        .write_all(bytes)
        .and_then(|()| fs::rename(&staged, file));
    if result.is_err() {
        let _ = fs::remove_file(&staged);
    }
    result
}

fn viewer_error(error: &dyn Display) -> StoreError {
    StoreError::new(
        "VIEWER_FAILED",
        &error.to_string(),
        "Check the macOS window session and the project path.",
    )
}

#[cfg(test)]
mod viewer_tests {
    use super::*;
    use splice_model::{Clip, ClipData, ClipId, ClipTiming};

    #[test]
    fn timeline_clipboard_commands_commit_through_store_and_select_created_items() {
        use splice_core::commands::Target;
        use splice_model::TimelineId;
        use timeline::ClipboardAction;

        let folder = std::env::temp_dir().join(format!("splice-clipboard-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store =
            Store::create(&folder.join("Clipboard.splice"), &Project::default()).unwrap();
        let seed = splice_json::parse(
            r#"[{"cmd":"title.add","at":72,"length":12,"text":"Source"},
                {"cmd":"title.add","at":120,"length":12,"text":"Destination"}]"#,
        )
        .unwrap();
        let receipt = apply_editor_batch(&mut store, seed.as_array().unwrap()).unwrap();
        let id = |index: usize| {
            let Target::Timeline(id) = &receipt.report.changes[index].created[0] else {
                panic!("expected a title");
            };
            *id
        };
        let first = id(0);
        let second = id(1);
        let local = splice_json::parse(&format!(
            r#"{{"cmd":"effect.add","target":"{first}","type":"text"}}"#
        ))
        .unwrap();
        apply_editor_batch(&mut store, &[local]).unwrap();
        let slot = |project: &Project, id| {
            let TimelineId::Item(id) = id else {
                panic!("expected a title");
            };
            project.items.find(id).unwrap()
        };
        let start = |project: &Project, id| {
            let timing = project.items.timing(slot(project, id)).unwrap();
            splice_model::timing::anchor_start(project, timing.anchor).unwrap()
        };
        let effect = |project: &Project, id| {
            project
                .items
                .data(slot(project, id))
                .unwrap()
                .appearance
                .effects[0]
                .id
        };
        let source_effect = effect(store.project(), first);
        let rev = store.project().rev;
        let mut layout = EditorLayout::new([960.0, 640.0]);
        let cache = timeline::Cache::default();
        let mut session = editor_session::State::default();
        let mut command = |id, layout: &mut EditorLayout, store: &mut Store| {
            dispatch_editor(
                Invoked {
                    id,
                    source: splice_kit::Source::Key,
                    target: 1,
                },
                layout,
                Some(store),
                None,
                None,
                &mut session,
                &cache,
            )
            .unwrap();
        };
        command(commands::PASTE, &mut layout, &mut store);
        layout.timeline.set_selected(vec![first, second]);
        command(commands::COPY, &mut layout, &mut store);
        assert_eq!(store.project().rev, rev);

        layout.scope = inspector::Scope::Canvas;
        command(commands::PASTE, &mut layout, &mut store);
        let pasted = layout.timeline.selected();
        assert_eq!(store.project().rev, rev + 1);
        assert_eq!(pasted.len(), 2);
        assert!(!pasted.contains(&first) && !pasted.contains(&second));
        assert_eq!(start(store.project(), pasted[0]), 0);
        assert_eq!(start(store.project(), pasted[1]), 48);
        assert_ne!(effect(store.project(), pasted[0]), source_effect);
        assert!(layout.scope == inspector::Scope::Selection);
        assert!(layout.timeline.clipboard_enabled(ClipboardAction::Paste));
        store.human_undo(0).unwrap();
        assert_eq!(store.project().items.order().len(), 2);

        layout.timeline.set_selection(Some(second), None);
        command(commands::PASTE_EFFECTS, &mut layout, &mut store);
        assert_ne!(effect(store.project(), second), source_effect);
        assert_eq!(layout.timeline.selected(), &[second]);

        command(commands::CUT, &mut layout, &mut store);
        assert_eq!(start(store.project(), second), 120);
        layout.timeline.set_selection(Some(first), None);
        command(commands::PASTE, &mut layout, &mut store);
        assert_eq!(start(store.project(), second), 0);
        assert_eq!(layout.timeline.selected(), &[first]);
        assert!(!layout.timeline.clipboard_enabled(ClipboardAction::Paste));
        store.human_undo(0).unwrap();
        assert_eq!(start(store.project(), second), 120);

        command(commands::COPY, &mut layout, &mut store);
        let delete = splice_json::parse(&format!(
            r#"{{"cmd":"item.delete","items":["{first}"],"ripple":false}}"#
        ))
        .unwrap();
        apply_editor_batch(&mut store, &[delete]).unwrap();
        let rev = store.project().rev;
        command(commands::PASTE, &mut layout, &mut store);
        assert_eq!(store.project().rev, rev);
        assert_eq!(store.project().items.order().len(), 1);
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn selection_shortcuts_use_live_store_timing_and_lane_targets() {
        use splice_model::{ItemId, TimelineId, timing};

        let reference =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
        let (_, project) = splice_core::package::Package::open(&reference).unwrap();
        let folder = std::env::temp_dir().join(format!("splice-selection-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Selection.splice"), &project).unwrap();
        let hook: ItemId = "ovl_hook".parse().unwrap();
        let label: ItemId = "ovl_label".parse().unwrap();
        let marker = store.project().markers[1].id;
        let opener: ClipId = "clp_open".parse().unwrap();
        let place = |project: &Project, id| {
            let slot = project.items.find(id).unwrap();
            let anchor = project.items.timing(slot).unwrap().anchor;
            (
                timing::anchor_start(project, anchor).unwrap(),
                project.items.data(slot).unwrap().lane,
            )
        };
        let initial = place(store.project(), label);
        let cache = timeline::Cache::default();
        let mut layout = EditorLayout::new([960.0, 640.0]);
        let mut session = editor_session::State::default();
        let mut command = |id, layout: &mut EditorLayout, store: &mut Store| {
            dispatch_editor(
                Invoked {
                    id,
                    source: splice_kit::Source::Key,
                    target: 1,
                },
                layout,
                Some(store),
                None,
                None,
                &mut session,
                &cache,
            )
            .unwrap();
        };

        layout.scope = inspector::Scope::Canvas;
        layout
            .timeline
            .set_selection(Some(TimelineId::Item(label)), None);
        command(commands::SELECT_LANE_ITEMS, &mut layout, &mut store);
        assert_eq!(
            layout.timeline.selected(),
            &[TimelineId::Item(hook), TimelineId::Item(label)]
        );
        assert!(layout.scope == inspector::Scope::Selection);
        let rev = store.project().rev;
        command(commands::NUDGE_FORWARD_TEN, &mut layout, &mut store);
        assert_eq!(store.project().rev, rev + 1);
        assert_eq!(place(store.project(), hook).0, 10);
        assert_eq!(place(store.project(), label), (initial.0 + 10, initial.1));
        store.human_undo(0).unwrap();
        assert_eq!(place(store.project(), label), initial);

        layout
            .timeline
            .set_selection(Some(TimelineId::Marker(marker)), None);
        let at = store.project().markers[1].at;
        command(commands::NUDGE_BACK, &mut layout, &mut store);
        assert_eq!(store.project().markers[1].at, at - 1);
        assert_eq!(place(store.project(), label), initial);

        layout
            .timeline
            .set_selection(Some(TimelineId::Item(label)), None);
        command(commands::LANE_UP, &mut layout, &mut store);
        assert_ne!(place(store.project(), label).1, initial.1);
        command(commands::LANE_DOWN, &mut layout, &mut store);
        assert_eq!(place(store.project(), label), initial);

        command(commands::BLADE_TOOL, &mut layout, &mut store);
        command(commands::ESCAPE, &mut layout, &mut store);
        assert_eq!(layout.timeline.selected(), &[TimelineId::Item(label)]);
        command(commands::ESCAPE, &mut layout, &mut store);
        assert!(layout.timeline.selected().is_empty());
        command(commands::SELECT_LANE_ITEMS, &mut layout, &mut store);
        assert_eq!(
            layout.timeline.selected().len(),
            project.clips.order().len()
        );

        layout
            .timeline
            .set_selected(vec![TimelineId::Item(label), TimelineId::Clip(opener)]);
        command(commands::LIFT_TO_OVERLAY, &mut layout, &mut store);
        assert!(store.project().clips.find(opener).is_none());
        assert!(store.project().items.find(label).is_some());
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_timeline_drop_moves_time_and_lane_in_one_undoable_edit() {
        let folder = std::env::temp_dir().join(format!("splice-drop-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Drop.splice"), &Project::default()).unwrap();
        let ops = splice_json::parse(
            r#"[{"cmd":"title.add","at":30,"length":60,"text":"Drop"},
                {"cmd":"lane.create","kind":"graphics","name":"Destination"}]"#,
        )
        .unwrap();
        let opts = WriteOptions::default();
        splice_mcp::apply_batch(&mut store, ops.as_array().unwrap(), &opts).unwrap();
        let project = store.project();
        let id = project.items.id(project.items.order()[0]).unwrap();
        let lane = project
            .lanes
            .iter()
            .find(|lane| lane.name == "Destination")
            .unwrap()
            .id;
        let placement = |project: &Project| {
            let slot = project.items.find(id).unwrap();
            let anchor = project.items.timing(slot).unwrap().anchor;
            let at = splice_model::timing::anchor_start(project, anchor).unwrap();
            (at, project.items.data(slot).unwrap().lane)
        };
        let original = placement(project);
        let rev = project.rev;
        let movement = timeline::MoveIntent {
            item: splice_model::TimelineId::Item(id),
            to: Some(100),
            ripple: false,
            lane: Some(lane),
            carry_overlays: false,
        };
        splice_mcp::apply_batch(&mut store, &move_operations(movement), &opts).unwrap();
        assert_eq!(store.project().rev, rev + 1);
        assert_eq!(placement(store.project()), (100, lane));
        store.human_undo(0).unwrap();
        assert_eq!(placement(store.project()), original);
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn live_view_tool_updates_editor_state_without_writing_the_project() {
        let mut project = Project::default();
        let id = ClipId::from_serial(1);
        let second = ClipId::from_serial(2);
        for (id, start) in [(id, 0), (second, 300)] {
            project
                .clips
                .insert(Clip {
                    id,
                    timing: ClipTiming {
                        start,
                        length: 300,
                        source: 0,
                        speed: 1.0,
                    },
                    data: ClipData::default(),
                })
                .unwrap();
        }
        let folder =
            std::env::temp_dir().join(format!("splice-native-view-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut viewer = Viewer::new(
            Store::create(&folder.join("View.splice"), &project).unwrap(),
            None,
        );
        let mut layout = EditorLayout::new([960.0, 640.0]);
        let request = Json::Object(vec![
            ("command".into(), Json::String("tool".into())),
            ("name".into(), Json::String("ui.show".into())),
            ("args".into(), splice_json::parse(&format!(
                r#"{{"projectId":"prj_1","select":["{id}","{second}"],"range":{{"from":20,"to":70}},"view":"mcp","viewer":{{"zoom":2}}}}"#
            )).unwrap()),
        ]);
        let reply = viewer.handle(&request, 0, &mut |store, command| {
            apply_view_command(&mut layout, None, store, command, None, None)
        });
        let payload = reply
            .field("result")
            .unwrap()
            .field("structuredContent")
            .unwrap();
        assert_eq!(payload.field("applied").and_then(Json::as_bool), Some(true));
        assert_eq!(payload.field("playhead").and_then(Json::as_i64), Some(20));
        assert_eq!(layout.view, editor_titlebar::View::Agent);
        let selected = [id, second].map(splice_model::TimelineId::Clip);
        assert_eq!(layout.timeline.selected(), &selected);
        assert_eq!(
            payload.field("selection").and_then(Json::as_array).unwrap(),
            &[
                Json::String(id.to_string()),
                Json::String(second.to_string())
            ],
        );
        assert_eq!(
            layout.program.zoom_percent([300.0, 200.0], [1920, 1080]),
            200
        );
        assert_eq!(viewer.store().project().rev, 0);
        viewer.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn agent_lab_run_renders_a_live_dry_run_without_committing() {
        use pxui::{Event, testing};

        let folder = std::env::temp_dir().join(format!("splice-lab-run-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut viewer = Viewer::new(
            Store::create(&folder.join("Lab.splice"), &Project::default()).unwrap(),
            None,
        );
        let dispatch = |viewer: &mut Viewer, name: &str, args: Json| {
            let request = Json::Object(vec![
                ("command".into(), Json::String("tool".into())),
                ("name".into(), Json::String(name.into())),
                ("args".into(), args),
                ("client".into(), Json::String("native-lab".into())),
            ]);
            viewer.handle(&request, 0, &mut |_, _| panic!("unexpected viewer command"))
        };
        let start = dispatch(
            &mut viewer,
            "session.start",
            splice_json::parse(r#"{"projectId":"prj_1","agent":"lab test"}"#).unwrap(),
        );
        let session: SessionId = start
            .field("result")
            .and_then(|result| result.field("structuredContent"))
            .and_then(|result| result.field("sessionId"))
            .and_then(Json::as_str)
            .unwrap()
            .parse()
            .unwrap();
        let mut lab = editor_lab::Lab::new();
        lab.set_draft(format!(
            r#"{{"projectId":"prj_1","sessionId":"{session}","ifRevision":"r0","dryRun":true,"ops":[{{"cmd":"marker.add","at":8,"name":"Beat"}}]}}"#
        ));
        let mut ui = viewer_ui().unwrap();
        ui.set_a11y(true);
        let frame = FrameIn::new(960.0, 606.0);
        let draw = |ui: &mut Ui, lab: &mut editor_lab::Lab, events: &[Event]| {
            let mut intents = editor_lab::Intents::default();
            ui.frame(&frame.events(events), &mut |ui| {
                intents.merge(editor_lab::paint(
                    ui,
                    Some((0, Some(session))),
                    lab,
                    &[],
                    None,
                    0.0,
                ));
            })
            .unwrap();
            lab.apply(intents)
        };
        for _ in 0..2 {
            assert!(draw(&mut ui, &mut lab, &[]).is_none());
        }
        let run = testing::find(&ui, Role::Button, "Run call").unwrap();
        let x = f64::from(run.x + run.w / 2.0);
        let y = f64::from(run.y + run.h / 2.0);
        assert!(
            draw(
                &mut ui,
                &mut lab,
                &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]
            )
            .is_none()
        );
        let call = draw(
            &mut ui,
            &mut lab,
            &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)],
        )
        .expect("Run dispatches the Lab request");
        let reply = dispatch(&mut viewer, call.name, call.args.clone());
        lab.set_reply(call.name, &call.args, &reply);
        assert!(draw(&mut ui, &mut lab, &[]).is_none());
        assert!(
            testing::find(&ui, Role::Label, "Dry run: Added marker mrk_1 at frame 8").is_some()
        );
        assert!(testing::find(&ui, Role::Label, "mrk_1 · — → 8–8 f").is_some());
        assert_eq!(viewer.store().project().rev, 0);
        assert!(viewer.store().project().markers.is_empty());
        viewer.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn editor_transport_survives_settling_and_inspector_stacks_on_narrow_windows() {
        use pxui::{Event, Role, testing};

        let project = Project::default();
        for (size, stacked) in [([960.0, 640.0], false), ([480.0, 1_250.0], true)] {
            let mut ui = viewer_ui().unwrap();
            ui.set_a11y(true);
            let mut cache = ViewerCache::default();
            let mut paint = |ui: &mut Ui, events: &[Event]| {
                let mut intents = ViewerIntents::default();
                ui.frame(&FrameIn::new(size[0], size[1]).events(events), &mut |ui| {
                    let next = paint_viewer(
                        ui,
                        Some("Editor fixture"),
                        (Some(&project), None, None, None, &[]),
                        EditorLayout::new(size),
                        ([None, None, None], &[]),
                        (&mut cache, &[], None),
                        playbar::View {
                            at: 0,
                            end: 600,
                            fps: 30,
                            play: commands::states(true, false, (false, false))
                                [commands::PLAY.0 as usize],
                            rate: 0,
                            dragging: false,
                        },
                    );
                    intents.merge(next);
                })
                .unwrap();
                intents
            };
            paint(&mut ui, &[]);
            paint(&mut ui, &[]);
            let transport = testing::find(&ui, Role::Button, "Go to end").unwrap();
            let inspector = testing::find(&ui, Role::Button, "Selection").unwrap();
            assert_eq!(inspector.y > transport.y, stacked);
            let x = f64::from(transport.x + transport.w / 2.0);
            let y = f64::from(transport.y + transport.h / 2.0);
            assert_eq!(
                paint(&mut ui, &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]).transport,
                None,
            );
            assert_eq!(
                paint(&mut ui, &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)]).transport,
                Some(viewer_chrome::TransportAction::End),
            );
            assert_eq!(paint(&mut ui, &[]).transport, None);
            if !stacked {
                let divider = testing::find(
                    &ui,
                    Role::Separator,
                    "Resize timeline with Up or Down. Double-click to reset",
                )
                .unwrap();
                let x = f64::from(divider.x + divider.w / 2.0);
                let y = f64::from(divider.y + divider.h / 2.0);
                paint(&mut ui, &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)]);
                assert_eq!(
                    paint(&mut ui, &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)])
                        .timeline_height,
                    None,
                );
                let mut encoded_keys = Events::default();
                let resized = paint(
                    &mut ui,
                    encoded_keys.encode(&[input::Event::KeyPressed(input::Key::ArrowUp)]),
                );
                assert_eq!(resized.timeline_height, Some(384.0));
                assert!(resized.timeline_height_committed);
            }
        }
    }

    #[test]
    fn compact_editor_scrolls_the_titlebar_with_the_inspector_and_uses_toolbar_history() {
        use pxui::{Event, Role, testing};

        let folder =
            std::env::temp_dir().join(format!("splice-compact-editor-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Compact.splice"), &Project::default()).unwrap();
        let marker = splice_json::parse(r#"{"cmd":"marker.add","at":8,"name":"Beat"}"#).unwrap();
        store.apply(&[marker], &WriteOptions::default()).unwrap();
        let mut history = commands::History::new(&store);
        let session = editor_session::State::default();
        let size = [480.0, 640.0];
        let mut layout = EditorLayout::new(size);
        let mut ui = viewer_ui().unwrap();
        ui.set_a11y(true);
        let mut cache = ViewerCache::default();
        let mut tick = 0;
        let mut draw = |ui: &mut Ui, store: &Store, events: &[Event]| {
            tick += 1;
            let mut intents = ViewerIntents::default();
            let input = FrameIn::new(size[0], size[1])
                .time(f64::from(tick) / 60.0)
                .events(events);
            ui.frame(&input, &mut |ui| {
                intents.merge(paint_viewer(
                    ui,
                    None,
                    (
                        Some(store.project()),
                        None,
                        None,
                        Some((&session, store)),
                        &[],
                    ),
                    EditorLayout::new(size),
                    ([None, None, None], &[]),
                    (&mut cache, &[], None),
                    playbar::View {
                        at: 0,
                        end: 60,
                        fps: 30,
                        play: commands::states(true, false, (false, false))
                            [commands::PLAY.0 as usize],
                        rate: 0,
                        dragging: false,
                    },
                ));
            })
            .unwrap();
            intents
        };
        draw(&mut ui, &store, &[]);
        let initial = draw(&mut ui, &store, &[]);
        assert_eq!(initial.program_bounds.w, 480.0);
        assert!(testing::find(&ui, Role::Button, "Add scale animation").is_some());
        assert!(testing::find(&ui, Role::Slider, "Timeline zoom").is_none());
        let undo = testing::find(&ui, Role::Button, "Undo").unwrap();
        assert!((undo.x - 298.0).abs() <= 1.0, "{undo:?}");
        assert!((undo.y - 400.5).abs() <= 1.0, "{undo:?}");
        for (label, id, count) in [("Undo", commands::UNDO, 0), ("Redo", commands::REDO, 1)] {
            let bounds = testing::find(&ui, Role::Button, label).unwrap();
            let x = f64::from(bounds.x + bounds.w * 0.5);
            let y = f64::from(bounds.y + bounds.h * 0.5);
            let pressed = draw(
                &mut ui,
                &store,
                &[Event::pointer(Event::POINTER_PRESSED, 0, x, y)],
            );
            assert!(pressed.command.is_none());
            let released = draw(
                &mut ui,
                &store,
                &[Event::pointer(Event::POINTER_RELEASED, 0, x, y)],
            );
            let action = released.command.unwrap();
            assert_eq!(action.id, id);
            dispatch_editor(
                action,
                &mut layout,
                Some(&mut store),
                Some(&mut history),
                None,
                &mut editor_session::State::default(),
                &timeline::Cache::default(),
            )
            .unwrap();
            assert_eq!(store.project().markers.len(), count);
            draw(&mut ui, &store, &[]);
        }
        let transport = testing::find(&ui, Role::Button, "Go to end").unwrap();
        draw(
            &mut ui,
            &store,
            &[
                Event::pointer(Event::POINTER_MOVED, 0, 240.0, 40.0),
                Event::pointer(Event::SCROLLED, 0, 0.0, -550.0),
            ],
        );
        for _ in 0..24 {
            draw(&mut ui, &store, &[]);
        }
        let inspector = testing::find(&ui, Role::Button, "Selection").unwrap();
        assert!((0.0..640.0).contains(&inspector.y), "{inspector:?}");
        assert!(
            testing::find(&ui, Role::Button, "Go to end").is_none_or(|after| after.y < transport.y)
        );
        assert!(testing::find(&ui, Role::ScrollArea, "Inspector").is_none());
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn viewer_popups_follow_the_toolbar_when_wrapped_and_scrolled() {
        use pxui::{Event, testing};
        use viewer_chrome::PaneAction;
        use viewer_drawers::Drawer;

        let folder =
            std::env::temp_dir().join(format!("splice-viewer-popups-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let mut store = Store::create(&folder.join("Popups.splice"), &Project::default()).unwrap();
        store.start_session("fixture", 1).unwrap();
        let mut session = editor_session::State::default();
        session.refresh(Some(&store), None);
        mem_sys::set_peer(pxui::library_counter);
        for (size, banner, wrapped, scroll) in [
            ([960.0, 640.0], Some(&session), true, false),
            ([960.0, 640.0], None, false, false),
            ([600.0, 240.0], Some(&session), true, true),
            ([480.0, 640.0], Some(&session), false, true),
        ] {
            let mut ui = viewer_ui().unwrap();
            ui.set_a11y(true);
            let mut cache = ViewerCache::default();
            let mut tick = 0;
            let mut draw = |ui: &mut Ui, layout: &EditorLayout, events: &[Event]| {
                tick += 1;
                let mut intents = ViewerIntents::default();
                let input = FrameIn::new(size[0], size[1])
                    .time(f64::from(tick) / 60.0)
                    .events(events);
                ui.frame(&input, &mut |ui| {
                    intents.merge(paint_viewer(
                        ui,
                        Some("Popups"),
                        (
                            Some(store.project()),
                            None,
                            None,
                            banner.map(|state| (state, &store)),
                            &[],
                        ),
                        layout.clone(),
                        ([None; 3], &[]),
                        (&mut cache, &[], None),
                        playbar::View {
                            at: 0,
                            end: 0,
                            fps: 30,
                            play: splice_kit::CommandState::default(),
                            rate: 0,
                            dragging: false,
                        },
                    ));
                })
                .unwrap();
                intents
            };
            let mut layout = EditorLayout::new(size);
            draw(&mut ui, &layout, &[]);
            let initial = draw(&mut ui, &layout, &[]).program_bounds;
            if wrapped {
                assert!(initial.y > 64.0, "{size:?} {initial:?}");
            } else {
                assert_eq!(initial.y, 64.0, "{size:?} {initial:?}");
            }
            if scroll {
                draw(
                    &mut ui,
                    &layout,
                    &[
                        Event::pointer(Event::POINTER_MOVED, 0, 240.0, 12.0),
                        Event::pointer(Event::SCROLLED, 0, 0.0, -1.0),
                    ],
                );
                for _ in 0..24 {
                    draw(&mut ui, &layout, &[]);
                }
            }
            let stage = draw(&mut ui, &layout, &[]).program_bounds;
            if scroll {
                assert!(stage.y < initial.y - 1.0, "{size:?} {stage:?}");
            }
            for (label, action, role) in [
                ("Media", PaneAction::Media, Role::Window),
                ("Effects", PaneAction::Effects, Role::Window),
                ("Captions", PaneAction::Captions, Role::Menu),
                ("Viewer zoom", PaneAction::Zoom, Role::Menu),
            ] {
                let button = testing::find(&ui, Role::Button, label).expect(label);
                let x = f64::from(button.x + button.w / 2.0);
                let y = f64::from(button.y + button.h / 2.0);
                assert!(
                    x < f64::from(size[0]) && y > 0.0,
                    "{size:?} {label} {button:?}"
                );
                let press = Event::pointer(Event::POINTER_PRESSED, 0, x, y);
                let release = Event::pointer(Event::POINTER_RELEASED, 0, x, y);
                assert!(draw(&mut ui, &layout, &[press]).pane.is_none());
                assert_eq!(draw(&mut ui, &layout, &[release]).pane, Some(action));
                match action {
                    PaneAction::Media => layout.drawer = Some(Drawer::Media),
                    PaneAction::Effects => layout.drawer = Some(Drawer::Effects),
                    PaneAction::Captions => layout.sheet = Some(EditorSheet::Captions),
                    PaneAction::Zoom => layout.sheet = Some(EditorSheet::Zoom(None)),
                    PaneAction::Crop => unreachable!(),
                }
                draw(&mut ui, &layout, &[]);
                let popup = testing::find(&ui, role, label).expect(label);
                assert!(
                    popup.y >= stage.y && popup.y - stage.y < 1.0 && popup.y + popup.h <= size[1],
                    "{size:?} {label} {popup:?} above or away from {stage:?}",
                );
                if role == Role::Window && size[0] <= 480.0 {
                    assert_eq!(popup.x, 12.0);
                } else if action != PaneAction::Zoom {
                    assert_eq!(popup.x, button.x);
                } else {
                    assert!(popup.x <= button.x && button.x < popup.x + popup.w);
                }
                splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
                    .warm(&mut || {
                        for _ in 0..4 {
                            draw(&mut ui, &layout, &[]);
                        }
                    });
                let dismissed = draw(&mut ui, &layout, &[press]);
                assert!(dismissed.pane.is_none());
                assert!(dismissed.dismiss_drawer || dismissed.dismiss_sheet);
                layout.drawer = None;
                layout.sheet = None;
                assert!(draw(&mut ui, &layout, &[release]).pane.is_none());
            }
            if wrapped && !scroll {
                let point = Point::new(stage.x + 250.0, stage.y + 40.0);
                let press = Event::pointer(
                    Event::POINTER_PRESSED,
                    u32::from(Signal::BUTTON_SECONDARY),
                    f64::from(point.x),
                    f64::from(point.y),
                );
                let update = draw(&mut ui, &layout, &[press]).update_sheet;
                assert!(matches!(update, Some(EditorSheet::Zoom(Some(at))) if at == point));
                layout.sheet = update;
                draw(&mut ui, &layout, &[]);
                let popup = testing::find(&ui, Role::Menu, "Viewer zoom").unwrap();
                assert_eq!((popup.x, popup.y), (point.x, point.y));
            }
        }
        store.close().unwrap();
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn warmed_editor_playhead_frames_do_not_allocate() {
        mem_sys::set_peer(pxui::library_counter);
        let mut project = Project::default();
        project
            .clips
            .insert(Clip {
                id: ClipId::from_serial(1),
                timing: ClipTiming {
                    start: 0,
                    length: 600,
                    source: 0,
                    speed: 1.0,
                },
                data: ClipData::default(),
            })
            .unwrap();
        let mut ui = viewer_ui().unwrap();
        let size = VIEWER_SIZE.map(|extent| extent as f32);
        let input = FrameIn::new(size[0], size[1]);
        let mut cache = ViewerCache::default();
        let mut draw = || {
            for at in 180..220 {
                ui.frame(&input, &mut |ui| {
                    paint_viewer(
                        ui,
                        Some("Editor fixture"),
                        (Some(&project), None, None, None, &[]),
                        EditorLayout::new(size),
                        ([None, None, None], &[]),
                        (&mut cache, &[], None),
                        playbar::View {
                            at,
                            end: 600,
                            fps: 30,
                            play: commands::states(true, true, (false, false))
                                [commands::PLAY.0 as usize],
                            rate: 1,
                            dragging: false,
                        },
                    );
                })
                .unwrap();
            }
        };
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset)
            .warm(&mut draw);
    }
}
