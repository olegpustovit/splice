use crate::{Json, object, text};
use apple_sys::capture::{self, Rect, Source, Target};
use splice_core::history::StoreError;

pub(super) fn target(source: &crate::record_options::Source) -> Result<Target, StoreError> {
    use crate::record_options::Source as Selector;
    Ok(match source {
        Selector::Display(id) => Target::Display(*id),
        Selector::Window(id) => Target::Window(*id),
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
            let sources = capture::sources(true).map_err(super::capture_source_error)?;
            Target::Window(window(&sources, app, title.as_deref())?)
        }
    })
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
        (first, _) => Err(StoreError::new(
            "CAPTURE_SOURCE",
            if first.is_none() {
                "No shareable window matches this selector."
            } else {
                "Multiple shareable windows match this selector."
            },
            "Supply the window title or use a window ID from record.start with dryRun:true.",
        )),
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
        assert_eq!(
            window(&sources, "Fixture App", Some("Other window"))
                .unwrap_err()
                .code,
            "CAPTURE_SOURCE"
        );
    }
}
