use crate::{Json, object, text};
use apple_sys::capture::{Rect, Source};

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
}
