//! Native Splice viewer, hosted by PXUI.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

use input::Event;
use pxui::{FrameIn, Label, Ui, UiDesc};
use pxui_host::{Accessibility, Graphics, Platform, Window};
use pxui_sdl::{DialogFilter, DialogKind};
use splice_core::history::{Store, StoreError};
use std::{
    ffi::OsString,
    fmt::Display,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
};

/// Opens a viewer on the application thread. With no project, presents the native open panel.
pub fn open(path: Option<&Path>, state: Option<&Path>) -> Result<(), StoreError> {
    run(path, state, false)
}

/// Exercises the same window and renderer while keeping the window hidden.
pub fn smoke_test(path: Option<&Path>, state: Option<&Path>) -> Result<(), StoreError> {
    run(path, state, true)
}

fn run(path: Option<&Path>, state: Option<&Path>, smoke: bool) -> Result<(), StoreError> {
    let mut project = path.map(Store::open).transpose()?;
    let mut window = Window::open_with("Splice", 960, 640, !smoke, smoke, !smoke)
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
            return window.destroy().map_err(|error| viewer_error(&error));
        };
        project = Some(Store::open(&selected)?);
    }
    if let (Some(file), Some(store)) = (&last, &project)
        && let Err(error) = remember(file, store.path())
    {
        eprintln!("splice: could not remember the last project: {error}");
    }
    let name = project.as_ref().map(|store| {
        store
            .path()
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
    });
    let result = run_surface(&mut window, &mut events, name.as_deref(), smoke);
    window.destroy().map_err(|error| viewer_error(&error))?;
    result?;
    if let Some(store) = project
        && store.problem().is_none()
    {
        store.close()?;
    }
    Ok(())
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
    smoke: bool,
) -> Result<(), StoreError> {
    let mut ui = Ui::new(&UiDesc::DEFAULT).map_err(|error| viewer_error(&error))?;
    let mut graphics = Graphics::open(window, &ui).map_err(|error| viewer_error(&error))?;
    let mut accessibility = Accessibility::open(window).map_err(|error| viewer_error(&error))?;
    let mut platform = Platform::default();
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
        let (width, height) = window.size();
        let input = FrameIn::new(width as f32, height as f32)
            .scale(window.scale() as f32)
            .time(window.time())
            .environment(platform.environment())
            .keyboard(platform.keyboard());
        accessibility.frame(&mut ui);
        ui.frame(&input, &mut |ui| {
            if let Some(name) = name {
                ui.label(name, Label::DEFAULT);
            }
        })
        .map_err(|error| viewer_error(&error))?;
        platform
            .apply(window, &ui)
            .map_err(|error| viewer_error(&error))?;
        graphics
            .draw(window, &ui, None)
            .map_err(|error| viewer_error(&error))?;
        if smoke {
            break;
        }
        window.wait(None);
    }
    drop(accessibility);
    graphics.close().map_err(|error| viewer_error(&error))
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
