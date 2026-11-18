use crate::{Json, bad_args, exit_error};
use splice_core::history::StoreError;
use std::{
    io::{self, IsTerminal, Write},
    time::Duration,
};

pub(super) fn command(args: &[String]) -> Result<Json, StoreError> {
    let args = arguments(args)?;
    let stdin = io::stdin();
    let manual =
        !enabled(&args, "dryRun") && !enabled(&args, "still") && args.field("duration").is_none();
    if cfg!(target_os = "macos") && manual && !stdin.is_terminal() {
        return Err(bad_args(
            "A manual recording needs a terminal.",
            "Pass --duration <seconds> for an unattended recording.",
        ));
    }
    match splice_mcp::record_project(&args, &mut || {
        writeln!(
            io::stderr().lock(),
            "Recording. Press Return to save or Control-D to discard."
        )?;
        stdin.read_line(&mut String::new()).map(|bytes| bytes != 0)
    }) {
        Ok(value) => Ok(value),
        Err(error) => exit_error(&error),
    }
}

fn arguments(args: &[String]) -> Result<Json, StoreError> {
    let Some((path, mut flags)) = args.split_first() else {
        return Err(option_error("Supply a project path to record into."));
    };
    if path.is_empty() || path.starts_with("--") {
        return Err(option_error(
            "The recording project must be a .splice path.",
        ));
    }
    let mut fields = vec![
        ("path".into(), Json::String(path.clone())),
        ("create".into(), Json::Bool(true)),
    ];
    let mut title = None;
    while let Some((flag, remaining)) = flags.split_first() {
        flags = remaining;
        let (key, value) = match flag.as_str() {
            "--mic" => ("mic", Json::Bool(true)),
            "--system-audio" => ("systemAudio", Json::Bool(true)),
            "--camera" => ("camera", Json::Bool(true)),
            "--no-events" => ("events", Json::Bool(false)),
            "--no-insert" => ("insert", Json::Bool(false)),
            "--still" => ("still", Json::Bool(true)),
            "--dry-run" => ("dryRun", Json::Bool(true)),
            "--area" => {
                let [display, x, y, width, height, remaining @ ..] = flags else {
                    return Err(option_error(
                        "Use --area <display> <x> <y> <width> <height>.",
                    ));
                };
                flags = remaining;
                ("source", area(display, [x, y, width, height])?)
            }
            "--display" | "--window" | "--title" | "--source" | "--duration" | "--countdown" => {
                let (value, remaining) = flags
                    .split_first()
                    .filter(|(value, _)| !value.starts_with("--"))
                    .ok_or_else(|| option_error(&format!("{flag} needs a value.")))?;
                flags = remaining;
                if flag == "--title" {
                    if title.replace(value).is_some() {
                        return Err(option_error("Use --title once."));
                    }
                    continue;
                }
                value_option(flag, value)?
            }
            _ => return Err(option_error(&format!("Unknown recording option {flag}."))),
        };
        if fields.iter().any(|(name, _)| name == key) {
            return Err(option_error(&format!(
                "{flag} duplicates a recording option."
            )));
        }
        fields.push((key.into(), value));
    }
    if let Some(title) = title {
        let error = || option_error("Use --title to select a window from the named application.");
        let Some(Json::Object(source)) = fields
            .iter_mut()
            .find_map(|(name, value)| (name == "source").then_some(value))
        else {
            return Err(error());
        };
        let [(key, Json::Object(window))] = source.as_mut_slice() else {
            return Err(error());
        };
        if key != "window" || window.iter().any(|(name, _)| name == "title") {
            return Err(error());
        }
        window.push(("title".into(), Json::String(title.clone())));
    }
    let args = Json::Object(fields);
    if enabled(&args, "still")
        && (args.field("duration").is_some()
            || ["mic", "systemAudio", "camera"]
                .iter()
                .any(|key| enabled(&args, key)))
    {
        return Err(option_error(
            "--still captures one image and accepts no duration or audio and camera tracks.",
        ));
    }
    Ok(args)
}

fn value_option(flag: &str, value: &str) -> Result<(&'static str, Json), StoreError> {
    let invalid = || option_error(&format!("Invalid value for {flag}."));
    Ok(match flag {
        "--display" if display(value) => {
            ("source", selector("display", Json::String(value.into())))
        }
        "--window" if !value.is_empty() => {
            let id = value.strip_prefix("win_").unwrap_or(value);
            let window = if positive_id(id) {
                Json::String(value.into())
            } else if value.starts_with("win_") {
                return Err(invalid());
            } else {
                selector("app", Json::String(value.into()))
            };
            ("source", selector("window", window))
        }
        "--source" => {
            let source = splice_json::parse(value).map_err(|_| invalid())?;
            let [(_, _)] = source.as_object().ok_or_else(invalid)? else {
                return Err(invalid());
            };
            ("source", source)
        }
        "--duration" => {
            let seconds = value
                .parse::<f64>()
                .ok()
                .filter(|&secs| {
                    Duration::try_from_secs_f64(secs).is_ok_and(|duration| !duration.is_zero())
                })
                .ok_or_else(invalid)?;
            ("duration", Json::Float(seconds))
        }
        "--countdown" => {
            let seconds = value
                .parse::<u32>()
                .ok()
                .filter(|&secs| secs <= i32::MAX as u32)
                .ok_or_else(invalid)?;
            ("countdown", Json::Unsigned(u64::from(seconds)))
        }
        _ => return Err(invalid()),
    })
}

fn area(display_id: &str, coordinates: [&str; 4]) -> Result<Json, StoreError> {
    let [x, y, width, height] =
        coordinates.map(|value| value.parse::<f64>().ok().filter(|v| v.is_finite()));
    let (Some(x), Some(y), Some(width), Some(height)) = (x, y, width, height) else {
        return Err(option_error("Pass numeric display points to --area."));
    };
    if !display(display_id) || x < 0.0 || y < 0.0 || width <= 0.0 || height <= 0.0 {
        return Err(option_error(
            "Use an active display, nonnegative x and y, and positive width and height.",
        ));
    }
    let rect = Json::Object(
        [("x", x), ("y", y), ("width", width), ("height", height)]
            .into_iter()
            .map(|(key, value)| (key.into(), Json::Float(value)))
            .collect(),
    );
    Ok(selector(
        "area",
        Json::Object(vec![
            ("display".into(), Json::String(display_id.into())),
            ("rect".into(), rect),
        ]),
    ))
}

fn selector(key: &str, value: Json) -> Json {
    Json::Object(vec![(key.into(), value)])
}

fn display(value: &str) -> bool {
    value == "main" || positive_id(value)
}

fn positive_id(value: &str) -> bool {
    value.parse::<u32>().is_ok_and(|id| id > 0)
}

fn option_error(message: &str) -> StoreError {
    bad_args(message, "Run splice --help for recording options.")
}

fn enabled(args: &Json, key: &str) -> bool {
    args.field(key).and_then(Json::as_bool) == Some(true)
}
