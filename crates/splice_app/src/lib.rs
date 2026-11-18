//! Native Splice viewer, hosted by PXUI.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

use input::Event;
use pxui::{FrameIn, Image, Label, Node, Ui, UiDesc, ui::Size};
use pxui_host::{Accessibility, Events, Platform, Window};
use pxui_sdl::{DialogFilter, DialogKind};
use splice_core::history::{Store, StoreError};
use splice_kit::Invoked;
use std::{
    borrow::Cow,
    ffi::OsString,
    fmt::Display,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod commands;
mod playbar;
mod program;
mod snapshot;
mod surface;

pub use snapshot::render as snapshot;

const VIEWER_SIZE: [u32; 2] = [960, 640];

/// Opens a viewer on the application thread. With no project, presents the native open panel.
pub fn open(path: Option<&Path>, state: Option<&Path>) -> Result<(), StoreError> {
    run(path, state, false).map(|_| ())
}

/// Presents the first available program frame to a hidden window.
pub fn smoke_test(path: Option<&Path>, state: Option<&Path>) -> Result<Option<i32>, StoreError> {
    run(path, state, true)
}

fn run(path: Option<&Path>, state: Option<&Path>, smoke: bool) -> Result<Option<i32>, StoreError> {
    let mut project = path.map(Store::open).transpose()?;
    let [width, height] = VIEWER_SIZE.map(i64::from);
    let mut window = Window::open_with("Splice", width, height, !smoke, smoke, !smoke)
        .map_err(|error| viewer_error(&error))?;
    window
        .set_resizable(true)
        .map_err(|error| viewer_error(&error))?;
    let mut events = Vec::with_capacity(input::EVENT_RESERVE);
    let last = state.map(|dir| dir.join("last-project"));
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
        project = Some(Store::open(&selected)?);
    }
    if let (Some(file), Some(store)) = (&last, &project)
        && let Err(error) = remember(file, store.path())
    {
        eprintln!("splice: could not remember the last project: {error}");
    }
    let keymap = state.map(|dir| dir.join("keymap"));
    let result = run_surface(
        &mut window,
        &mut events,
        project.as_mut(),
        keymap.as_deref(),
        smoke,
    );
    window.destroy().map_err(|error| viewer_error(&error))?;
    let shown = result?;
    if let Some(store) = project
        && store.problem().is_none()
    {
        store.close()?;
    }
    Ok(shown)
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
    mut store: Option<&mut Store>,
    keymap: Option<&Path>,
    smoke: bool,
) -> Result<Option<i32>, StoreError> {
    let name = store
        .as_deref()
        .map(|store| project_name(store).into_owned());
    let mut program = store
        .as_deref()
        .map(program::Program::prepare)
        .transpose()?
        .flatten();
    let mut history = store.as_deref().map(commands::History::new);
    let mut revision = store.as_deref().map(|store| store.project().rev);
    let mut keys = commands::Keys::new(keymap);
    let mut ui = Ui::new(&UiDesc::DEFAULT).map_err(|error| viewer_error(&error))?;
    let mut surface =
        surface::Surface::open(window, &ui, program.as_ref().map(program::Program::size))?;
    let mut accessibility = Accessibility::open(window).map_err(|error| viewer_error(&error))?;
    let mut platform = Platform::default();
    let mut encoded = Events::default();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut presented = None;
    loop {
        events.clear();
        let open = platform
            .pump(window, events)
            .map_err(|error| viewer_error(&error))?;
        if let Some(error) = window.take_failure() {
            return Err(viewer_error(&error));
        }
        if !open {
            break;
        }
        events.retain(|event| {
            !keys.route(event, &mut |action| {
                if let Err(error) = dispatch(
                    action,
                    store.as_deref_mut(),
                    history.as_mut(),
                    program.as_mut(),
                ) {
                    eprintln!("splice: {error}");
                }
            })
        });
        if let Some(store) = store.as_deref()
            && revision != Some(store.project().rev)
        {
            revision = Some(store.project().rev);
            if let Some(history) = &mut history {
                history.sync(store);
            }
            let at = program.as_ref().map_or(0, program::Program::at);
            let playing = program.as_ref().is_some_and(program::Program::playing);
            let mut next = program::Program::prepare(store)?;
            if let Some(next) = &mut next {
                next.seek(at)?;
                if playing && next.at() < next.end() - 1 {
                    next.play(true);
                }
            }
            let size = next.as_ref().map(program::Program::size);
            if size == program.as_ref().map(program::Program::size) {
                surface.clear_program();
            } else {
                surface.close()?;
                surface = surface::Surface::open(window, &ui, size)?;
            }
            program = next;
            presented = None;
        }
        let mut repaint = false;
        if surface.begin(window)? {
            if let Some(program) = &mut program {
                program.sample(&mut surface)?;
            }
            let (width, height) = window.size();
            let mut input = FrameIn::new(width as f32, height as f32)
                .scale(window.scale() as f32)
                .time(window.time())
                .environment(platform.environment())
                .keyboard(platform.keyboard());
            let packets = encoded.encode(events);
            input.wire.events = packets.as_ptr();
            input.wire.event_count = packets.len();
            let mut intents = playbar::Intents::default();
            let (at, end, playing, dragging) =
                program.as_ref().map_or((0, 0, false, false), |program| {
                    (
                        program.at(),
                        program.end(),
                        program.playing(),
                        program.dragging(),
                    )
                });
            let fps = store.as_deref().map_or(30, |store| store.project().fps);
            let play = commands::states(end > 0 && !dragging, playing, (false, false))
                [commands::PLAY.0 as usize];
            accessibility.frame(&mut ui);
            ui.frame(&input, &mut |ui| {
                intents = paint_viewer(
                    ui,
                    name.as_deref(),
                    program.as_ref().and_then(program::Program::contents),
                    surface.image(),
                    playbar::View {
                        at,
                        end,
                        fps,
                        play,
                        dragging,
                    },
                );
            })
            .map_err(|error| viewer_error(&error))?;
            platform
                .apply(window, &ui)
                .map_err(|error| viewer_error(&error))?;
            if surface.draw(window, &ui)? {
                presented = program.as_ref().and_then(program::Program::shown);
                if smoke && (program.is_none() || presented.is_some()) {
                    break;
                }
            }
            if let Some((phase, at)) = intents.scrub
                && let Some(program) = &mut program
            {
                program.scrub(phase, at)?;
                repaint = true;
            }
            if intents.play {
                dispatch(
                    keys.play_button(),
                    store.as_deref_mut(),
                    history.as_mut(),
                    program.as_mut(),
                )?;
                repaint = true;
            }
        }
        let waiting = program.as_ref().is_some_and(program::Program::wants_frame);
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
        window.wait(next);
    }
    drop(accessibility);
    surface.close()?;
    Ok(presented)
}

fn dispatch(
    action: Invoked,
    store: Option<&mut Store>,
    history: Option<&mut commands::History>,
    program: Option<&mut program::Program>,
) -> Result<(), StoreError> {
    use commands::{BACK, FORWARD, PLAY, REDO, UNDO};
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
        (BACK, Some(program), _, _) => program.step(-1),
        (FORWARD, Some(program), _, _) => program.step(1),
        (UNDO | REDO, _, Some(store), Some(history)) => {
            let time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|time| u64::try_from(time.as_millis()).ok())
                .unwrap_or(0);
            history.invoke(store, action.id, time).map(|_| ())
        }
        _ => Ok(()),
    }
}

fn project_name(store: &Store) -> Cow<'_, str> {
    store
        .path()
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
}

fn paint_viewer(
    ui: &mut Ui,
    name: Option<&str>,
    contents: Option<(i32, &Rc<splice_av::Composition>)>,
    image: Option<Image>,
    view: playbar::View,
) -> playbar::Intents {
    let mut intents = playbar::Intents::default();
    ui.column(
        Node {
            w: Size::Grow,
            h: Size::Grow,
            ..Node::DEFAULT
        },
        |ui| {
            if let Some(name) = name {
                ui.label(name, Label::DEFAULT);
            }
            program::paint(ui, contents, image);
            intents = playbar::paint(ui, view);
        },
    );
    intents
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
    let parent = file.parent().expect("state file path");
    fs::create_dir_all(parent)?;
    let staged = parent.join(format!(".last-project-{}", std::process::id()));
    let mut temp = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&staged)?;
    let result = temp
        .write_all(project.as_os_str().as_bytes())
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
