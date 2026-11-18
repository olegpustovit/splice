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
            "--display" | "--window" | "--source" | "--duration" | "--countdown" => {
                let (value, remaining) = flags
                    .split_first()
                    .filter(|(value, _)| !value.starts_with("--"))
                    .ok_or_else(|| option_error(&format!("{flag} needs a value.")))?;
                flags = remaining;
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
        "--display" | "--window" if !value.is_empty() => (
            "source",
            Json::Object(vec![(flag[2..].into(), Json::String(value.into()))]),
        ),
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

fn option_error(message: &str) -> StoreError {
    bad_args(message, "Run splice --help for recording options.")
}

fn enabled(args: &Json, key: &str) -> bool {
    args.field(key).and_then(Json::as_bool) == Some(true)
}
