//! Native Splice viewer, hosted by PXUI.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

use input::Event;
use pxui::{FrameIn, Image, Label, Node, Ui, UiDesc, ui::Size};
use pxui_host::{Accessibility, Events, Platform, Window};
use pxui_sdl::{DialogFilter, DialogKind};
use splice_core::history::{Store, StoreError};
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
    time::{Duration, Instant},
};

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
    let name = project.as_ref().map(project_name);
    let mut program = project
        .as_ref()
        .map(program::Program::prepare)
        .transpose()?
        .flatten();
    let result = run_surface(
        &mut window,
        &mut events,
        name.as_deref(),
        program.as_mut(),
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
    name: Option<&str>,
    mut program: Option<&mut program::Program>,
    smoke: bool,
) -> Result<Option<i32>, StoreError> {
    let mut ui = Ui::new(&UiDesc::DEFAULT).map_err(|error| viewer_error(&error))?;
    let mut surface =
        surface::Surface::open(window, &ui, program.as_deref().map(program::Program::size))?;
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
        if surface.begin(window)? {
            if let Some(program) = program.as_deref_mut() {
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
            accessibility.frame(&mut ui);
            ui.frame(&input, &mut |ui| {
                paint_viewer(
                    ui,
                    name,
                    program.as_deref().and_then(program::Program::contents),
                    surface.image(),
                );
            })
            .map_err(|error| viewer_error(&error))?;
            platform
                .apply(window, &ui)
                .map_err(|error| viewer_error(&error))?;
            if surface.draw(window, &ui)? {
                presented = program.as_deref().and_then(program::Program::shown);
                if smoke && (program.is_none() || presented.is_some()) {
                    break;
                }
            }
        }
        let waiting = program.is_some() && presented.is_none();
        if smoke && Instant::now() >= deadline {
            return Err(viewer_error(
                &"Timed out presenting the first program frame.",
            ));
        }
        window.wait((waiting || smoke).then(|| window.time() + 1.0 / 120.0));
    }
    drop(accessibility);
    surface.close()?;
    Ok(presented)
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
) {
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
        },
    );
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
