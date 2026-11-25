use super::GESTURES;
use crate::commands::{self, BindingDescription};
use splice_core::{
    commands::{COMMANDS, CommandError, Tier, decode, lookup},
    edit,
    history::{EditResult, Store, StoreError, WriteOptions},
    package::{Package, journal::Source},
};
use splice_json::Value as Json;
use splice_mcp::tool_catalog;
use splice_model::{Media, MediaId, MediaKind, Probe, Project};
use std::{
    fs, io,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub(crate) enum Parity {
    #[default]
    Idle,
    Running,
    Complete(Report),
}

impl Parity {
    /// Run outside the PXUI frame. A request schedules one frame with the running state.
    pub(crate) fn advance(&mut self, requested: bool, bindings: &[BindingDescription]) -> bool {
        match self {
            Self::Running => *self = Self::Complete(Report::run(bindings)),
            _ if requested => *self = Self::Running,
            _ => return false,
        }
        true
    }
}

pub(crate) struct Report {
    pub(super) examples: Vec<Example>,
    pub(super) problems: Vec<Problem>,
    pub(super) keymap: usize,
    pub(super) tools: usize,
    pub(super) revision: Option<u64>,
    pub(super) milliseconds: u128,
}

pub(super) struct Example {
    pub(super) command: &'static str,
    pub(super) result: Result<(String, usize), (&'static str, String)>,
}

pub(super) struct Problem {
    pub(super) rule: &'static str,
    pub(super) surface: &'static str,
    pub(super) name: &'static str,
    pub(super) detail: &'static str,
}

impl Report {
    pub(super) fn run(bindings: &[BindingDescription]) -> Self {
        let started = Instant::now();
        let mut report = Self {
            examples: Vec::new(),
            problems: Vec::new(),
            keymap: 0,
            tools: 0,
            revision: None,
            milliseconds: 0,
        };
        let catalog = commands::catalog();
        for (index, binding) in bindings.iter().enumerate() {
            if binding.keys.is_empty() {
                continue;
            }
            report.keymap += 1;
            let repeated = bindings[..index]
                .iter()
                .any(|other| other.id == binding.id && !other.keys.is_empty());
            if repeated
                || !catalog.iter().any(|command| {
                    command.id == binding.id
                        && command.title == binding.title
                        && command.group == binding.group
                })
            {
                report.problems.push(Problem {
                    rule: "1",
                    surface: "Keymap",
                    name: binding.title,
                    detail: "Effective binding has no unique native command.",
                });
            }
        }
        for &(gesture, dispatch) in GESTURES {
            let command = dispatch.split(' ').next().unwrap_or("");
            if lookup(command).is_none()
                && !catalog.iter().any(|row| row.name == command)
                && !tool_catalog().iter().any(|tool| tool.name == command)
            {
                report.problems.push(Problem {
                    rule: "1",
                    surface: "Gesture",
                    name: gesture,
                    detail: "Dispatch has no native command or view tool.",
                });
            }
        }
        let tools = tool_catalog();
        for (index, tool) in tools.iter().enumerate() {
            report.tools += 1;
            let unique = !tools[..index].iter().any(|other| tool.name == other.name);
            let required = tool
                .required
                .iter()
                .all(|key| tool.input.iter().any(|(field, _)| field == key));
            let success = tool
                .success
                .iter()
                .flat_map(|keys| keys.iter())
                .all(|key| tool.output.iter().any(|(field, _)| field == key));
            if !unique || !required || !success || tool.description.is_empty() {
                report.problems.push(Problem {
                    rule: "4",
                    surface: "MCP schema",
                    name: tool.name,
                    detail: "Tool has a duplicate ID, missing description, or undeclared schema field.",
                });
            }
        }
        let mut sample = match load_sample() {
            Ok(project) => {
                report.revision = Some(project.rev);
                Some(project)
            }
            Err(error) => {
                report.examples.push(Example {
                    command: "Bundled sample",
                    result: Err(("FIXTURE_LOAD", error)),
                });
                None
            }
        };
        for (index, command) in COMMANDS.iter().enumerate() {
            let registered = !COMMANDS[..index].iter().any(|other| other.id == command.id)
                && lookup(command.id).is_some_and(|row| row.id == command.id);
            let decoded = splice_json::parse(command.example).ok().filter(|example| {
                example.field("cmd").and_then(splice_json::Value::as_str) == Some(command.id)
                    && decode(example).is_ok()
            });
            if !registered || command.title.is_empty() || decoded.is_none() {
                report.problems.push(Problem {
                    rule: "4",
                    surface: "Registry",
                    name: command.id,
                    detail: "Command has a duplicate ID, missing title, or invalid example.",
                });
            }
            if command.tier == Tier::Planned {
                continue;
            }
            if !command.ui.iter().any(|entry| !entry.is_empty()) {
                report.problems.push(Problem {
                    rule: "3",
                    surface: "Registry UI entries",
                    name: command.id,
                    detail: "No UI entry point is documented beyond palette discovery.",
                });
            }
            if let Some(sample) = sample.as_mut() {
                let result = match decoded {
                    Some(op) => run_example(sample, command.id, op),
                    None => Err(("BAD_EXAMPLE", "The registry example did not decode.".into())),
                };
                report.examples.push(Example {
                    command: command.id,
                    result,
                });
            }
        }
        report.milliseconds = started.elapsed().as_millis();
        report
    }

    pub(super) fn passed(&self) -> usize {
        self.examples
            .iter()
            .filter(|row| row.result.is_ok())
            .count()
    }
}

/// Execute setup in the same dry run so batch refs resolve exactly as documented.
pub(super) fn run_example(
    sample: &mut Project,
    command: &'static str,
    op: Json,
) -> Result<(String, usize), (&'static str, String)> {
    let mut ops = match splice_json::parse(setup(command)) {
        Ok(Json::Array(ops)) => ops,
        _ => {
            return Err((
                "BAD_FIXTURE",
                "Fixture setup must be an array of operations.".into(),
            ));
        }
    };
    let index = ops.len();
    ops.push(op);
    let preview = if command == "html.add" {
        preview_html(sample, &ops).map(|run| run.report)
    } else {
        edit::apply(sample, &ops, Source::Agent, true).map_err(StoreError::from)
    };
    let report = preview.map_err(|error| {
        let message = if let Some(op) = error.op.filter(|&op| op < index) {
            let cmd = ops[op]
                .field("cmd")
                .and_then(Json::as_str)
                .unwrap_or("operation");
            format!("Setup {cmd}: {}", error.message)
        } else {
            error.message
        };
        (error.code, message)
    })?;
    let change = report
        .changes
        .into_iter()
        .find(|row| row.op == index && row.cmd == command)
        .ok_or_else(|| {
            (
                "BAD_FIXTURE",
                format!("The dry run did not report an edit for {command}."),
            )
        })?;
    Ok((change.summary, report.side_effects.len()))
}

fn setup(command: &str) -> &'static str {
    match command {
        "motion.set" => {
            r#"[{"cmd":"motion.add","at":180,"length":150,"from":1,"to":1.16,"ref":"motion"}]"#
        }
        "clip.join" => r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"}]"#,
        "clip.ungroup" => {
            r#"[{"cmd":"item.split","item":"clp_take","at":300,"ref":"cut"},{"cmd":"clip.join","items":["clp_take","$cut.right"]}]"#
        }
        "clip.restore" => r#"[{"cmd":"cut.range","from":200,"to":230}]"#,
        "mask.set" | "mask.track" => r#"[{"cmd":"mask.add","item":"clp_take","ref":"mask"}]"#,
        "audio.link" | "audio.sync" => {
            r#"[{"cmd":"audio.unlink","item":"clp_take","ref":"voice"}]"#
        }
        "clip.uncompound" => {
            r#"[{"cmd":"clip.compound","items":["clp_take","clp_end"],"ref":"compound"}]"#
        }
        "transition.set" | "transition.remove" => r#"[{"cmd":"transition.add","item":"clp_end"}]"#,
        "template.set" => r#"[{"cmd":"template.add","name":"lower-third","at":150,"ref":"lower"}]"#,
        _ => "[]",
    }
}

// A three-second QuickTime PNG movie of the registry page, rendered in Chromium
// at quarter scale. Its transparent frames include the full one-second CSS fade.
const HTML_PAGE: &str =
    r#"<h1 style='color:white;animation:a 1s'>Hi</h1><style>@keyframes a{from{opacity:0}}</style>"#;
const HTML_PATH: &str = "media/html-5f1c.mov";
const HTML_MOVIE: &[u8] = include_bytes!("html-5f1c.mov");

pub(super) fn preview_html(sample: &Project, ops: &[Json]) -> Result<EditResult, StoreError> {
    if !ops.last().is_some_and(|op| {
        op.field("html").and_then(Json::as_str) == Some(HTML_PAGE)
            && op.field("movie").and_then(Json::as_str) == Some(HTML_PATH)
            && op.field("length").and_then(Json::as_i64) == Some(90)
    }) {
        return Err(StoreError::new(
            "MEDIA_OFFLINE",
            "No bundled rendered movie matches this HTML example.",
            "Provide the rendered movie for the example's page and duration.",
        ));
    }

    fn fixture_io(error: io::Error) -> StoreError {
        StoreError::new(
            "FIXTURE_IO",
            &error.to_string(),
            "Check the temporary directory.",
        )
    }

    let dir = temporary_path("html");
    fs::create_dir(&dir).map_err(fixture_io)?;
    let result = (|| {
        let mut store = Store::create(&dir.join("sample.splice"), sample)?;
        let movie_path = store.path().join(HTML_PATH);
        let media = store.path().join("media");
        fs::create_dir(&media).map_err(fixture_io)?;
        fs::write(&movie_path, HTML_MOVIE).map_err(fixture_io)?;
        let session = store.start_session("Spec parity check", 0)?;
        let options = WriteOptions {
            source: Source::Agent,
            session: Some(session.id),
            if_revision: Some(sample.rev),
            dry_run: true,
            ..WriteOptions::default()
        };
        // The descriptor comes from the bundled movie's verified track and time base.
        let mut load = |path: &str| {
            if path != HTML_PATH {
                return edit::missing_media(path);
            }
            let metadata = fs::metadata(&movie_path).map_err(|error| {
                CommandError::new(
                    "MEDIA_OFFLINE",
                    error.to_string(),
                    "Restore the bundled rendered movie to the sample package.".into(),
                )
            })?;
            Ok(Media {
                id: MediaId::from_serial(0),
                kind: MediaKind::Video,
                path: HTML_PATH.into(),
                bookmark: None,
                size: metadata.len(),
                mtime: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |time| time.as_secs() as i64),
                probe: Probe {
                    fps: 30.0,
                    frames: 90,
                    width: 270,
                    height: 480,
                    audio: Vec::new(),
                    codec: Some("png".into()),
                    loudness: None,
                },
                take: None,
            })
        };
        store.apply_with_media(ops, &options, &mut load)
    })();
    let cleanup = fs::remove_dir_all(&dir);
    let preview = result?;
    cleanup.map_err(fixture_io)?;
    Ok(preview)
}

fn temporary_path(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splice-parity-{name}-{}-{nonce}.splice",
        std::process::id()
    ))
}

pub(super) fn load_sample() -> Result<Project, String> {
    let path = temporary_path("sample");
    fs::create_dir(&path).map_err(|error| error.to_string())?;
    let loaded = (|| -> Result<Project, Box<dyn std::error::Error>> {
        fs::create_dir(path.join("transcripts"))?;
        for (name, bytes) in [
            (
                "project.json",
                include_bytes!("../../../../../fixtures/projects/sample.splice/project.json")
                    .as_slice(),
            ),
            (
                "media.json",
                include_bytes!("../../../../../fixtures/projects/sample.splice/media.json")
                    .as_slice(),
            ),
            (
                "transcripts/med_screen.json",
                include_bytes!(
                    "../../../../../fixtures/projects/sample.splice/transcripts/med_screen.json"
                )
                .as_slice(),
            ),
        ] {
            fs::write(path.join(name), bytes)?;
        }
        Ok(Package::open(&path)?.1)
    })();
    let cleanup = fs::remove_dir_all(&path);
    let project = loaded.map_err(|error| error.to_string())?;
    cleanup.map_err(|error| error.to_string())?;
    Ok(project)
}
