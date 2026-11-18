//! Command-line entry point for the Splice project store and MCP server.

use splice_core::{
    commands,
    history::{Store, StoreError, WriteOptions},
    package,
};
use splice_json::Value as Json;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const HELP: &str = "Usage: splice [<project.splice>]
       splice open [<project.splice>]
       splice open <project.splice> --snapshot <out.png> [--at <frame>]
       splice new <project.splice>
       splice mcp
       splice apply <ops.json|-> [<project.splice>] [--project <project.splice>]
                    [--dry-run] [--if-revision <revision>] [--idempotency-key <key>]
       splice snapshot [<project.splice>]
       splice check [<project.splice>]
       splice describe [<command>|--query <text>]
       splice export [<project.splice>] [--preset <social-h264|social-hevc>]
                     [--output <path.mp4>] [--overwrite]
       splice frame --at <frame> <out.png> [<project.splice>]
       splice friction [list [--open]|show <id>|repro <id>|resolve <id> <sha>]
       splice version
";

fn main() {
    let mut stdout = io::stdout().lock();
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string().map_err(|_| {
                bad_args(
                    "A command argument is not valid UTF-8.",
                    "Pass UTF-8 command arguments.",
                )
            })
        })
        .collect::<Result<Vec<_>, _>>();
    if let Err(error) = args.and_then(|args| run(&args, &mut stdout)) {
        let status = if error.code == "BAD_ARGS" { 2 } else { 1 };
        let _ = write_json(&splice_mcp::error_payload(error), &mut io::stderr().lock());
        std::process::exit(status);
    }
}

fn run(args: &[String], output: &mut dyn Write) -> Result<(), StoreError> {
    let Some((command, tail)) = args.split_first() else {
        return viewer(&[], false, output);
    };
    let value = match (command.as_str(), tail) {
        ("--help" | "-h", []) => return output.write_all(HELP.as_bytes()).map_err(io_error),
        ("version" | "--version", []) => {
            return writeln!(output, "splice {}", env!("CARGO_PKG_VERSION")).map_err(io_error);
        }
        ("mcp", []) => {
            return match splice_mcp::serve(&mut io::stdin().lock(), output) {
                Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
                result => result.map_err(io_error),
            };
        }
        ("friction", args) => {
            return splice_mcp::friction::command(args, output).map_err(io_error);
        }
        ("new", [path]) => project_summary(&Store::create(Path::new(path), &Default::default())?),
        ("apply", args) => apply(args)?,
        ("snapshot", args) => {
            let store = Store::open(&project_arg(args)?)?;
            let snapshot = package::snapshot(store.project());
            store.close()?;
            snapshot
        }
        ("check", args) => {
            let store = Store::open(&project_arg(args)?)?;
            let state = store.verify()?;
            let Json::Object(mut fields) = project_summary(&store) else {
                unreachable!("project summary")
            };
            fields.push(("ok".into(), Json::Bool(true)));
            fields.push(("stateHash".into(), hex(state)));
            fields.push((
                "batches".into(),
                Json::Unsigned(store.records().len() as u64),
            ));
            Json::Object(fields)
        }
        ("describe", []) => commands::describe(None, None)?,
        ("describe", [id]) if !id.starts_with('-') => commands::describe(Some(id), None)?,
        ("describe", [flag, query]) if flag == "--query" => commands::describe(None, Some(query))?,
        ("open", args) => return open(args, output),
        ("--smoke-test", args) => return viewer(args, true, output),
        ("export", args) => export(args)?,
        ("frame", args) => frame(args)?,
        (path, [])
            if Path::new(path)
                .extension()
                .is_some_and(|ext| ext == "splice") =>
        {
            return viewer(args, false, output);
        }
        _ => {
            return Err(bad_args(
                "Invalid command or arguments.",
                "Run splice --help for usage.",
            ));
        }
    };
    write_json(&value, output)
}

fn open(args: &[String], output: &mut dyn Write) -> Result<(), StoreError> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--snapshot" | "--at"))
    {
        let error = || {
            bad_args(
                "Expected a project and PNG filename for the viewer snapshot.",
                "Run splice open project.splice --snapshot out.png [--at N].",
            )
        };
        let [project, snapshot, file, trailing @ ..] = args else {
            return Err(error());
        };
        if snapshot != "--snapshot"
            || project.is_empty()
            || project.starts_with('-')
            || file.is_empty()
            || file.starts_with("--")
        {
            return Err(error());
        }
        let at = match trailing {
            [] => 0,
            [flag, at] if flag == "--at" => frame_number(at)?,
            _ => return Err(error()),
        };
        return write_json(
            &viewer_snapshot(Path::new(project), Path::new(file), at)?,
            output,
        );
    }
    viewer(args, false, output)
}

#[cfg(not(target_os = "macos"))]
fn viewer_snapshot(_: &Path, _: &Path, _: i32) -> Result<Json, StoreError> {
    Err(unavailable("Viewer snapshots"))
}

#[cfg(target_os = "macos")]
fn viewer_snapshot(project: &Path, file: &Path, at: i32) -> Result<Json, StoreError> {
    let store = Store::open(project)?;
    let revision = store.project().rev;
    let output = splice_mcp::write_png(file, &mut || splice_app::snapshot(&store, at))?;
    Ok(Json::Object(vec![
        (
            "output".into(),
            Json::String(output.to_string_lossy().into()),
        ),
        ("revision".into(), Json::String(format!("r{revision}"))),
        ("frame".into(), Json::Signed(i64::from(at))),
    ]))
}

#[cfg(not(target_os = "macos"))]
fn viewer(_: &[String], _: bool, _: &mut dyn Write) -> Result<(), StoreError> {
    Err(unavailable("The project viewer"))
}

#[cfg(target_os = "macos")]
fn viewer(args: &[String], smoke: bool, output: &mut dyn Write) -> Result<(), StoreError> {
    let path = if args.is_empty() {
        None
    } else {
        Some(project_arg(args)?)
    };
    let state = splice_mcp::friction::state_dir().ok();
    if smoke {
        let frame = splice_app::smoke_test(path.as_deref(), state.as_deref())?;
        let mut fields = vec![("ok".into(), Json::Bool(true))];
        if let Some(frame) = frame {
            fields.push(("programFrame".into(), Json::Signed(i64::from(frame))));
        }
        write_json(&Json::Object(fields), output)
    } else {
        splice_app::open(path.as_deref(), state.as_deref())
    }
}

fn apply(args: &[String]) -> Result<Json, StoreError> {
    let (file, project, options) = apply_args(args)?;
    let input = if file == Path::new("-") {
        let mut input = String::new();
        io::stdin()
            .lock()
            .read_to_string(&mut input)
            .map_err(io_error)?;
        input
    } else {
        fs::read_to_string(&file).map_err(|error| {
            StoreError::new(
                "IO",
                &format!("{}: {error}", file.display()),
                "Use a readable UTF-8 operations file.",
            )
        })?
    };
    let value = splice_json::parse(&input).map_err(|error| {
        bad_args(
            &format!("Invalid operations JSON: {error}"),
            "Pass the JSON array of operations to apply.",
        )
    })?;
    let ops = match &value {
        Json::Array(ops) => Some(ops.as_slice()),
        Json::Object(fields) if fields.len() == 1 => value.field("ops").and_then(Json::as_array),
        _ => None,
    };
    let ops = ops.filter(|ops| !ops.is_empty()).ok_or_else(|| {
        bad_args(
            "The operations input must contain a nonempty array.",
            "Pass an array or an object with only the ops array.",
        )
    })?;
    let mut store = Store::open(&project)?;
    let result = splice_mcp::apply_batch(&mut store, ops, &options)?;
    // An append already synced by Store::apply remains committed if the closing checkpoint fails.
    let checkpoint = if result.dry_run || result.replayed {
        None
    } else {
        store.close().err()
    };
    let mut reply = splice_mcp::edit_result_json(result);
    if let Some(warning) = checkpoint
        && reply.field("warning").is_none()
        && let Json::Object(fields) = &mut reply
    {
        fields.push(("warning".into(), splice_mcp::error_payload(warning)));
    }
    Ok(reply)
}

fn apply_args(args: &[String]) -> Result<(PathBuf, PathBuf, WriteOptions), StoreError> {
    let mut options = WriteOptions {
        time: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| u64::try_from(time.as_millis()).ok())
            .unwrap_or(0),
        ..WriteOptions::default()
    };
    let mut input = None;
    let mut project = None;
    let mut tail = args.iter();
    while let Some(arg) = tail.next() {
        let mut value = || {
            tail.next()
                .map(String::as_str)
                .filter(|value| !value.is_empty() && !value.starts_with("--"))
                .ok_or_else(|| {
                    bad_args(
                        &format!("{arg} requires a value."),
                        "Run splice --help for apply usage.",
                    )
                })
        };
        match arg.as_str() {
            "--dry-run" => options.dry_run = true,
            "--if-revision" if options.if_revision.is_none() => {
                let raw = value()?;
                let digits = raw.strip_prefix('r').unwrap_or(raw);
                options.if_revision = Some(
                    digits
                        .parse::<u64>()
                        .ok()
                        .filter(|_| digits.bytes().all(|b| b.is_ascii_digit()))
                        .ok_or_else(|| {
                            bad_args("Invalid revision.", "Pass a revision such as r12.")
                        })?,
                );
            }
            "--idempotency-key" if options.key.is_none() => options.key = Some(value()?.to_owned()),
            "--project" if project.is_none() => project = Some(PathBuf::from(value()?)),
            arg if (arg == "-" || !arg.starts_with('-')) && input.is_none() => {
                input = Some(PathBuf::from(arg))
            }
            arg if !arg.starts_with('-') && project.is_none() => project = Some(PathBuf::from(arg)),
            _ => {
                return Err(bad_args(
                    &format!("Unexpected apply argument {arg:?}."),
                    "Pass one operations input, one project, and each option at most once.",
                ));
            }
        }
    }
    let input = input
        .ok_or_else(|| bad_args("Missing operations input.", "Pass ops.json or - for stdin."))?;
    let project = project.map_or_else(current_project, Ok)?;
    Ok((input, project, options))
}

fn project_arg(args: &[String]) -> Result<PathBuf, StoreError> {
    match args {
        [] => current_project(),
        [path] if !path.starts_with('-') => Ok(PathBuf::from(path)),
        [flag, path] if flag == "--project" => Ok(PathBuf::from(path)),
        _ => Err(bad_args(
            "Expected one project path.",
            "Pass a .splice path or run inside the project package.",
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn export(_: &[String]) -> Result<Json, StoreError> {
    Err(unavailable("Export"))
}

#[cfg(target_os = "macos")]
fn export(args: &[String]) -> Result<Json, StoreError> {
    let mut project = None;
    let mut preset = None;
    let mut output = None;
    let mut overwrite = false;
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .filter(|value| !value.starts_with("--"))
                .ok_or_else(|| {
                    bad_args(
                        &format!("Missing value for {arg}."),
                        "Run splice --help for export usage.",
                    )
                })
        };
        match arg {
            "--project" if project.is_none() => project = Some(PathBuf::from(value()?)),
            "--preset" if preset.is_none() => preset = Some(value()?),
            "--output" if output.is_none() => output = Some(value()?),
            "--overwrite" if !overwrite => overwrite = true,
            path if !path.starts_with('-') && project.is_none() => {
                project = Some(PathBuf::from(path))
            }
            _ => {
                return Err(bad_args(
                    &format!("Unexpected export argument {arg:?}."),
                    "Pass one project and each export option at most once.",
                ));
            }
        }
    }
    let project = project.map_or_else(current_project, Ok)?;
    splice_mcp::export_project(Store::open(&project)?, preset, output, overwrite)
}

#[cfg(not(target_os = "macos"))]
fn frame(_: &[String]) -> Result<Json, StoreError> {
    Err(unavailable("Frame rendering"))
}

#[cfg(target_os = "macos")]
fn frame(args: &[String]) -> Result<Json, StoreError> {
    let [flag, at, file, project @ ..] = args else {
        return Err(bad_args(
            "Expected an exact frame and PNG filename.",
            "Run splice frame --at N out.png [project.splice].",
        ));
    };
    if flag != "--at" {
        return Err(bad_args(
            "Invalid frame number.",
            "Pass --at followed by a nonnegative integer frame.",
        ));
    }
    let at = frame_number(at)?;
    splice_mcp::grab_frame(Store::open(&project_arg(project)?)?, at, Path::new(file))
}

fn frame_number(at: &str) -> Result<i32, StoreError> {
    if at.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(frame) = at.parse::<i32>()
    {
        return Ok(frame);
    }
    Err(bad_args(
        "Invalid frame number.",
        "Pass --at followed by a nonnegative integer frame.",
    ))
}

fn current_project() -> Result<PathBuf, StoreError> {
    std::env::current_dir()
        .map_err(io_error)?
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "splice"))
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            bad_args(
                "No project was supplied.",
                "Pass a .splice path or run inside the project package.",
            )
        })
}

fn project_summary(store: &Store) -> Json {
    let project = store.project();
    Json::Object(vec![
        (
            "path".into(),
            Json::String(store.path().to_string_lossy().into()),
        ),
        ("revision".into(), Json::String(format!("r{}", project.rev))),
        ("hash".into(), hex(project.hash)),
    ])
}

fn hex(value: u64) -> Json {
    Json::String(format!("{value:016x}"))
}

fn write_json(value: &Json, output: &mut dyn Write) -> Result<(), StoreError> {
    let mut wire = String::new();
    splice_json::write(value, &mut wire).map_err(|error| {
        StoreError::new(
            "INVARIANT",
            &error.to_string(),
            "Check the project's numeric values.",
        )
    })?;
    wire.push('\n');
    output.write_all(wire.as_bytes()).map_err(io_error)
}

fn bad_args(message: &str, fix: &str) -> StoreError {
    StoreError::new("BAD_ARGS", message, fix)
}

fn io_error(error: io::Error) -> StoreError {
    StoreError::new(
        "IO",
        &error.to_string(),
        "Check the command's paths and stream permissions.",
    )
}

#[cfg(not(target_os = "macos"))]
fn unavailable(feature: &str) -> StoreError {
    StoreError::new(
        "UNSUPPORTED_PLATFORM",
        &format!("{feature} requires the native macOS build."),
        "Run this command with Splice.app on macOS.",
    )
}
