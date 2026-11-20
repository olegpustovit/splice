use crate::{Json, object, text};
use apple_sys::capture::{self, Rect, Source, Target};
use splice_core::history::StoreError;

use crate::record_options::Source as Selector;

/// What `source` captures: one target, or every window of a multi-window take (T46.6) in
/// order. Named windows resolve against the shareable windows now.
pub(super) fn targets(source: &Selector) -> Result<Vec<Target>, StoreError> {
    let listed = matches!(
        source,
        Selector::Window(_) | Selector::NamedWindow { .. } | Selector::Windows(_)
    );
    let sources = if listed {
        capture::sources(true).map_err(super::capture_source_error)?
    } else {
        Vec::new()
    };
    resolve(source, &sources)
}

/// `targets` against the shareable `sources`: each window must match exactly one window and
/// a multi-window take lists each window once; `CAPTURE_SOURCE` names the open windows.
pub(super) fn resolve(source: &Selector, sources: &[Source]) -> Result<Vec<Target>, StoreError> {
    let one = |selector: &Selector| -> Result<Target, StoreError> {
        Ok(match selector {
            Selector::Display(id) => Target::Display(*id),
            Selector::Window(id) => {
                if !sources.iter().any(
                    |source| matches!(source, Source::Window { id: listed, .. } if listed == id),
                ) {
                    let mut error: StoreError = splice_core::commands::CommandError::rule(
                        "CAPTURE_SOURCE",
                        "window",
                        Some(text(&format!("win_{id}"))),
                        "a shareable window's win_ ID",
                        &format!(
                            "Pick an open window by app (source:{{window:{{app}}}}) or by ID. Open windows: {}.",
                            open_windows(sources)
                        ),
                    )
                    .into();
                    error.message = format!("No shareable window has the ID win_{id}.");
                    return Err(error);
                }
                Target::Window(*id)
            }
            Selector::Area(display, [x, y, width, height]) => Target::Area {
                display: *display,
                rect: Rect {
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                },
            },
            Selector::NamedWindow { app, title } => {
                Target::Window(window(sources, app, title.as_deref())?)
            }
            Selector::Windows(_) => unreachable!("windows hold window selectors"),
        })
    };
    let Selector::Windows(windows) = source else {
        return one(source).map(|target| vec![target]);
    };
    let targets = windows.iter().map(one).collect::<Result<Vec<_>, _>>()?;
    if let Some(twice) = targets
        .iter()
        .enumerate()
        .find_map(|(n, target)| targets[..n].contains(target).then_some(target))
    {
        let Target::Window(id) = twice else {
            unreachable!("window targets")
        };
        let mut error: StoreError = splice_core::commands::CommandError::rule(
            "CAPTURE_SOURCE",
            "source.windows",
            Some(text(&format!("win_{id}"))),
            "different windows",
            "List each window once: add title (or use win_ IDs from record.start {dryRun:true, titles:true}) to tell an app's windows apart.",
        )
        .into();
        error.message = format!("Window win_{id} is listed twice.");
        return Err(error);
    }
    Ok(targets)
}

fn window(sources: &[Source], app: &str, title: Option<&str>) -> Result<u32, StoreError> {
    let mut matches = sources.iter().filter_map(|source| match source {
        Source::Window { id, app: owner, title: name, pid, .. }
            if title.is_none_or(|title| name == title) && (owner == app || sources.iter().any(|source| {
                matches!(source, Source::Application { pid: owner, name, .. } if owner == pid && name == app)
            })) => Some(*id),
        _ => None,
    });
    match (matches.next(), matches.next()) {
        (Some(id), None) => Ok(id),
        (first, _) => {
            let mut error: StoreError = splice_core::commands::CommandError::rule(
                "CAPTURE_SOURCE",
                "window.app",
                Some(text(app)),
                "an app with one open window (or add title)",
                &format!(
                    "Use source:{{window:{{app, title}}}} with the window's title, or a win_ ID from record.start {{dryRun:true, titles:true}}. Open windows: {}.",
                    open_windows(sources)
                ),
            )
            .into();
            error.message = if first.is_none() {
                format!("No shareable window of \"{app}\" matches this selector.")
            } else {
                format!("Several shareable windows of \"{app}\" match this selector.")
            };
            Err(error)
        }
    }
}

/// The shareable windows as `win_<id> (App)`, at most 16; titles stay private (D19).
fn open_windows(sources: &[Source]) -> String {
    let name = |pid: &u32, bundle: &str| {
        sources
            .iter()
            .find_map(|source| match source {
                Source::Application {
                    pid: owner, name, ..
                } if owner == pid => Some(name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| bundle.to_owned())
    };
    let windows: Vec<String> = sources
        .iter()
        .filter_map(|source| match source {
            Source::Window { id, pid, app, .. } => Some(format!("win_{id} ({})", name(pid, app))),
            _ => None,
        })
        .take(16)
        .collect();
    if windows.is_empty() {
        "none (open the app's window, or grant Screen Recording)".into()
    } else {
        windows.join(", ")
    }
}

pub(super) fn payload(sources: &[Source]) -> Json {
    let mut displays = Vec::new();
    let mut windows = Vec::new();
    let mut applications = Vec::new();
    for source in sources {
        match source {
            Source::Display { id, main, rect } => {
                let id = if *main {
                    "main".to_owned()
                } else {
                    id.to_string()
                };
                let mut fields = bounds(rect);
                fields.push(("id", Json::String(id)));
                displays.push(object(fields));
            }
            Source::Window {
                id,
                rect,
                pid,
                app,
                title,
            } => {
                let mut fields = bounds(rect);
                fields.extend([
                    ("id", Json::String(format!("win_{id}"))),
                    ("app", text(app)),
                    ("title", text(title)),
                    ("pid", Json::Unsigned(u64::from(*pid))),
                ]);
                windows.push(object(fields));
            }
            Source::Application { pid, bundle, name } => applications.push(object(vec![
                ("pid", Json::Unsigned(u64::from(*pid))),
                ("bundleId", text(bundle)),
                ("name", text(name)),
            ])),
        }
    }
    object(vec![
        ("displays", Json::Array(displays)),
        ("windows", Json::Array(windows)),
        ("applications", Json::Array(applications)),
    ])
}

fn bounds(rect: &Rect) -> Vec<(&'static str, Json)> {
    vec![
        ("x", Json::Float(rect.x)),
        ("y", Json::Float(rect.y)),
        ("width", Json::Float(rect.width)),
        ("height", Json::Float(rect.height)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_agent_sources_keep_stable_display_and_window_selectors() {
        let rect = Rect {
            x: -1920.0,
            y: 12.0,
            width: 1920.0,
            height: 1080.0,
        };
        let report = payload(&[
            Source::Display {
                id: 4,
                main: false,
                rect,
            },
            Source::Display {
                id: 2,
                main: true,
                rect,
            },
            Source::Window {
                id: 12,
                rect,
                pid: 37,
                app: "com.example.fixture".into(),
                title: "Test window".into(),
            },
            Source::Application {
                pid: 37,
                bundle: "com.example.fixture".into(),
                name: "Fixture App".into(),
            },
        ]);
        let displays = report.field("displays").unwrap().as_array().unwrap();
        assert_eq!(displays[0].field("id").and_then(Json::as_str), Some("4"));
        assert_eq!(displays[1].field("id").and_then(Json::as_str), Some("main"));
        assert_eq!(displays[0].field("x").and_then(Json::as_f64), Some(-1920.0));
        let windows = report.field("windows").unwrap().as_array().unwrap();
        assert_eq!(
            windows[0].field("id").and_then(Json::as_str),
            Some("win_12")
        );
        assert_eq!(
            windows[0].field("title").and_then(Json::as_str),
            Some("Test window")
        );
    }

    #[test]
    fn capture_agent_window_selection_rejects_ambiguous_titles() {
        let sources = [
            Source::Window {
                id: 37,
                pid: 4,
                rect: Rect::default(),
                app: "com.example.fixture".into(),
                title: "Test window".into(),
            },
            Source::Window {
                id: 38,
                pid: 4,
                rect: Rect::default(),
                app: "com.example.fixture".into(),
                title: "Test window".into(),
            },
            Source::Application {
                pid: 4,
                bundle: "com.example.fixture".into(),
                name: "Fixture App".into(),
            },
        ];
        assert_eq!(
            window(&sources[1..], "Fixture App", Some("Test window")).unwrap(),
            38
        );
        assert_eq!(window(&sources[1..], "Fixture App", None).unwrap(), 38);
        assert_eq!(
            window(&sources, "com.example.fixture", None)
                .unwrap_err()
                .code,
            "CAPTURE_SOURCE"
        );
        let missing = window(&sources, "Fixture App", Some("Other window")).unwrap_err();
        assert_eq!(missing.code, "CAPTURE_SOURCE");
        // The fix names the open windows by ID and app, never by title (D19).
        assert!(
            missing
                .fix
                .contains("win_37 (Fixture App), win_38 (Fixture App)"),
            "{}",
            missing.fix
        );
        assert!(!missing.fix.contains("Test window"));
    }

    #[test]
    fn capture_agent_resolves_every_window_of_a_multi_window_take() {
        let window = |id: u32, pid: u32, app: &str| Source::Window {
            id,
            pid,
            rect: Rect::default(),
            app: app.into(),
            title: format!("Window {id}"),
        };
        let sources = [
            window(10, 1, "com.apple.Safari"),
            window(11, 2, "com.apple.Notes"),
            Source::Application {
                pid: 1,
                bundle: "com.apple.Safari".into(),
                name: "Safari".into(),
            },
            Source::Application {
                pid: 2,
                bundle: "com.apple.Notes".into(),
                name: "Notes".into(),
            },
        ];
        let named = |app: &str| Selector::NamedWindow {
            app: app.into(),
            title: None,
        };
        assert_eq!(
            resolve(
                &Selector::Windows(vec![named("Notes"), Selector::Window(10)]),
                &sources
            )
            .unwrap(),
            [Target::Window(11), Target::Window(10)]
        );
        let closed = resolve(
            &Selector::Windows(vec![named("Notes"), Selector::Window(99)]),
            &sources,
        )
        .unwrap_err();
        assert_eq!(closed.code, "CAPTURE_SOURCE");
        assert!(closed.fix.contains("win_10 (Safari)"), "{}", closed.fix);
        let unknown = resolve(
            &Selector::Windows(vec![named("Safari"), named("Xcode")]),
            &sources,
        )
        .unwrap_err();
        assert_eq!(unknown.code, "CAPTURE_SOURCE");
        assert!(unknown.message.contains("\"Xcode\""), "{}", unknown.message);
        assert!(
            unknown.fix.contains("win_10 (Safari), win_11 (Notes)"),
            "{}",
            unknown.fix
        );
        let twice = resolve(
            &Selector::Windows(vec![named("com.apple.Safari"), Selector::Window(10)]),
            &sources,
        )
        .unwrap_err();
        assert_eq!(twice.code, "CAPTURE_SOURCE");
        assert_eq!(twice.rule.unwrap().param, "source.windows");
    }
}
