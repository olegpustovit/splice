use super::{
    Json, history, media, object, platform, project::Projects, text, timeline, typescript, wire::*,
};
use splice_core::{commands, history::StoreError};

const REVISION: &str =
    r#"{"anyOf":[{"type":"integer","minimum":0},{"type":"string","pattern":"^r[0-9]+$"}]}"#;
const IDS: &str = r#"{"type":"array","items":{"type":"string"}}"#;
const FRAME: &str = r#"{"type":"integer","minimum":0}"#;
const FRAMES: &str =
    r#"{"type":"array","items":{"type":"integer","minimum":0},"minItems":1,"maxItems":16}"#;
const FILMSTRIP: &str = r#"{"type":"integer","minimum":1,"maximum":16}"#;
const LIMIT: &str = r#"{"type":"integer","minimum":1}"#;
const RANGE: &str = r#"{"type":"object","properties":{"from":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}},"required":["from","to"]}"#;
const ERROR_SCHEMA: &str = r#"{"type":"object","properties":{"code":{"type":"string"},"message":{"type":"string"},"fix":{"type":"string"},"valid":{"type":"array"},"current":{},"changedSince":{"type":"array"},"op":{"type":"integer","minimum":0}},"required":["code","message","fix"]}"#;

pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) description: &'static str,
    read_only: bool,
    pub(super) input: &'static [(&'static str, &'static str)],
    pub(super) required: &'static [&'static str],
    pub(super) output: &'static [(&'static str, &'static str)],
    pub(super) success: &'static [&'static [&'static str]],
}

/// The code profile's runner (`splice mcp --code`); the default profile does not list it.
const CODE_RUN: Tool = Tool {
    name: "code.run",
    description: "Run JavaScript that calls Splice tools synchronously through the global `splice`, e.g. `const s = splice.timeline.snapshot({}); return s.spine.length;`. Call commands.describe {format:\"ts\"} for the API index, then {format:\"ts\", query:\"clip.crop,effect.add\"} for the types of the commands you use. The return value enters your context: return only the fields you need (ids, ranges, revision), not whole snapshots.",
    read_only: false,
    input: &[
        ("code", r#"{"type":"string","maxLength":65536}"#),
        ("projectId", "string"),
        ("sessionId", "string"),
    ],
    required: &["code"],
    output: &[
        ("result", "{}"),
        ("logs", r#"{"type":"array","items":{"type":"string"}}"#),
        ("calls", "array"),
        ("revisions", r#"{"type":["object","null"]}"#),
        ("jobs", r#"{"type":"array","items":{"type":"string"}}"#),
    ],
    success: &[&["result", "logs", "calls", "revisions"]],
};

pub(super) const TOOLS: &[Tool] = &[
    Tool {
        name: "project.info",
        description: "Open a .splice project by path or inspect an open project by ID. Use create:true to make a new project.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("path", "string"),
            ("create", "boolean"),
        ],
        required: &[],
        output: &[
            ("projectId", "string"),
            ("revision", REVISION),
            ("fps", "integer"),
            ("size", "array"),
            ("aspect", "string"),
            ("durationFrames", FRAME),
            ("access", "string"),
            ("lanes", "array"),
            ("media", "array"),
            ("offlineMedia", "array"),
            ("transcript", "object"),
            ("effects", "object"),
            ("openSession", r#"{"type":["string","null"]}"#),
        ],
        success: &[&[
            "projectId",
            "revision",
            "fps",
            "durationFrames",
            "access",
            "lanes",
            "media",
        ]],
    },
    Tool {
        name: "timeline.snapshot",
        description: "List the spine, drawing lanes and markers with resolved frame ranges. detail:full adds every item field and is large; prefer concise, then items.inspect for the IDs you need.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("from", FRAME),
            ("to", FRAME),
            ("detail", r#"{"enum":["concise","full"]}"#),
            ("limit", LIMIT),
            ("cursor", "string"),
        ],
        required: &["projectId"],
        output: &[
            ("revision", REVISION),
            ("fps", "integer"),
            ("range", RANGE),
            ("lanes", "array"),
            ("spine", "array"),
            ("markers", "array"),
            ("total", "integer"),
            ("nextCursor", r#"{"type":["string","null"]}"#),
        ],
        success: &[&["revision", "lanes", "spine", "markers"]],
    },
    Tool {
        name: "items.inspect",
        description: "Inspect up to 50 objects by ID, including anchors, resolved frames, children and inherited effects.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            (
                "ids",
                r#"{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":50}"#,
            ),
        ],
        required: &["projectId", "ids"],
        output: &[("revision", REVISION), ("items", "array")],
        success: &[&["revision", "items"]],
    },
    Tool {
        name: "timeline.find",
        description: "Find items, markers, words and edit points by text, kind, lane, color or frame range.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("text", "string"),
            ("kind", "string"),
            ("lane", "string"),
            ("color", "string"),
            ("from", FRAME),
            ("to", FRAME),
            ("limit", LIMIT),
        ],
        required: &["projectId"],
        output: &[
            ("revision", REVISION),
            ("matches", "array"),
            ("total", "integer"),
        ],
        success: &[&["revision", "matches", "total"]],
    },
    Tool {
        name: "takes.events",
        description: "Read the take's cursor, clicks and keys in timeline frames, with keystrokes redacted by project policy.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("clip", "string"),
            ("from", FRAME),
            ("to", FRAME),
        ],
        required: &["projectId", "clip"],
        output: &[
            ("revision", REVISION),
            ("clip", "string"),
            ("cursor", "array"),
            ("clicks", "array"),
            ("keys", "array"),
        ],
        success: &[&["revision", "clip", "clicks", "keys"]],
    },
    Tool {
        name: "transcript.get",
        description: "Page through words by ID or get caption cues as JSON, SRT or WebVTT. Use the returned hash with word anchors. With transcribe:true (and sessionId), first recognizes the clip's speech on device and saves the words.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("clip", "string"),
            ("transcribe", "boolean"),
            ("language", "string"),
            ("sessionId", "string"),
            ("cursor", "string"),
            ("limit", LIMIT),
            ("captions", r#"{"enum":["json","srt","vtt"]}"#),
            ("lane", "string"),
        ],
        required: &["projectId"],
        output: &[
            ("revision", REVISION),
            ("hash", r#"{"type":["string","null"]}"#),
            ("clip", "string"),
            ("words", "array"),
            ("fillers", IDS),
            ("nextCursor", r#"{"type":["string","null"]}"#),
            ("lanes", "array"),
            ("cues", r#"{"type":["array","integer"]}"#),
            ("format", r#"{"enum":["srt","vtt"]}"#),
            ("text", "string"),
        ],
        success: &[
            &["revision", "hash", "words"],
            &["revision", "hash", "cues"],
            &["revision", "format", "text"],
        ],
    },
    Tool {
        name: "audio.summary",
        description: "Read loudness, peaks and silence or pause ranges for a clip's audio components.",
        read_only: true,
        input: &[("projectId", "string"), ("clip", "string")],
        required: &["projectId", "clip"],
        output: &[
            ("revision", REVISION),
            ("clip", "string"),
            ("components", "array"),
            ("silences", "array"),
        ],
        success: &[&["revision", "clip", "components", "silences"]],
    },
    Tool {
        name: "view.frames",
        description: "Render PNG frames from the export composition. Pass explicit frame numbers or a filmstrip count. macOS builds only; elsewhere it fails with UNSUPPORTED_PLATFORM.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("at", FRAMES),
            ("count", FILMSTRIP),
        ],
        required: &["projectId"],
        output: &[("revision", REVISION), ("frames", "array")],
        success: &[&["revision", "frames"]],
    },
    Tool {
        name: "commands.describe",
        description: "Search the command registry. Pass an exact ID to see parameters, release tier and an example.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("id", "string"),
            ("query", "string"),
            ("format", r#"{"enum":["json","ts"]}"#),
        ],
        required: &[],
        output: &[
            ("commands", "array"),
            ("id", "string"),
            ("title", "string"),
            ("group", "string"),
            ("risk", "string"),
            ("cut", "string"),
            ("available", "boolean"),
            ("planned", "boolean"),
            ("params", "object"),
            ("shortcut", r#"{"type":["string","null"]}"#),
            ("ui", "array"),
            ("example", "object"),
            ("captionModel", "string"),
            ("presets", IDS),
            ("effectTypes", "object"),
            ("resolution", "string"),
            ("format", r#"{"enum":["ts"]}"#),
            ("text", "string"),
        ],
        success: &[
            &["commands"],
            &["id", "params", "example"],
            &["format", "text"],
        ],
    },
    Tool {
        name: "session.start",
        description: "Start an agent editing session and get its checkpoint revision and session ID.",
        read_only: false,
        input: &[("projectId", "string"), ("agent", "string")],
        required: &["projectId"],
        output: &[
            ("sessionId", "string"),
            ("checkpoint", REVISION),
            ("revision", REVISION),
            ("access", "string"),
            ("resumed", "boolean"),
        ],
        success: &[&["sessionId", "checkpoint", "revision"]],
    },
    Tool {
        name: "edit.apply",
        description: "Apply one atomic batch of registry commands. Use dryRun to preview and idempotencyKey for a safe retry.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
            (
                "ops",
                r#"{"type":"array","items":{"type":"object","properties":{"cmd":{"type":"string"},"ref":{"type":"string"}},"required":["cmd"]},"minItems":1}"#,
            ),
            ("summary", "string"),
            ("dryRun", "boolean"),
            ("idempotencyKey", "string"),
        ],
        required: &["projectId", "sessionId", "ifRevision", "ops"],
        output: &[
            ("revision", REVISION),
            ("dryRun", "boolean"),
            ("replayed", "boolean"),
            ("changes", "array"),
            ("sideEffects", "array"),
            ("diff", "array"),
        ],
        success: &[&["revision", "changes", "sideEffects", "diff"]],
    },
    Tool {
        name: "history.list",
        description: "List revision labels, sources and affected IDs. Filter by session to inspect its batch history.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("sessionId", "string"),
            ("limit", LIMIT),
            ("cursor", "string"),
        ],
        required: &["projectId"],
        output: &[
            ("sessionId", "string"),
            ("revision", REVISION),
            ("batches", "array"),
            ("open", "boolean"),
            ("checkpoint", REVISION),
            ("humanEditsSince", "integer"),
            ("nextCursor", r#"{"type":["string","null"]}"#),
        ],
        success: &[&["revision", "batches"]],
    },
    Tool {
        name: "history.undo",
        description: "Create an undo revision for the session's last edit, the whole session, or an exact earlier revision.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
            ("scope", r#"{"enum":["last","session","to"]}"#),
            ("revision", REVISION),
            ("dryRun", "boolean"),
        ],
        required: &["projectId", "sessionId", "ifRevision", "scope"],
        output: &[
            ("revision", REVISION),
            ("skipped", "array"),
            ("replayed", "integer"),
            ("reverted", "array"),
            ("restored", r#"{"type":["string","null"]}"#),
        ],
        success: &[&["revision"]],
    },
    Tool {
        name: "ui.show",
        description: "Set playhead, selection, range or zoom in an open viewer. Headless calls return applied:false.",
        read_only: true,
        input: &[
            ("projectId", "string"),
            ("at", r#"{"type":["integer","object"]}"#),
            ("select", IDS),
            ("range", RANGE),
            ("expand", "string"),
            ("view", "string"),
            ("viewer", "object"),
        ],
        required: &["projectId"],
        output: &[
            ("applied", "boolean"),
            ("reason", "string"),
            ("revision", REVISION),
            ("playhead", FRAME),
            ("selection", IDS),
        ],
        success: &[&["applied"]],
    },
    Tool {
        name: "export.start",
        description: "Start an MP4 export job. Use overwrite:true to replace an existing output file. macOS builds only; elsewhere it fails with UNSUPPORTED_PLATFORM.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
            (
                "preset",
                r#"{"enum":["social","social-h264","social-hevc"]}"#,
            ),
            ("path", "string"),
            ("overwrite", "boolean"),
        ],
        required: &["projectId", "sessionId", "ifRevision"],
        output: &[
            ("jobId", "string"),
            ("state", "string"),
            ("settings", "object"),
        ],
        success: &[&["jobId", "state"]],
    },
    Tool {
        name: "jobs.status",
        description: "Check the progress and result of an export, transcription or recording job. Each call advances a running export by up to 50 ms; poll until state is done. Inside code.run (2 s limit) return the jobId and poll from later calls.",
        read_only: true,
        input: &[("projectId", "string"), ("jobId", "string")],
        required: &["jobId"],
        output: &[
            ("jobId", "string"),
            ("id", "string"),
            ("kind", "string"),
            ("state", "string"),
            ("progress", "number"),
            ("settings", "object"),
            ("output", "string"),
            ("elapsed", "number"),
            ("bytes", "integer"),
            ("droppedFrames", "integer"),
            ("revision", REVISION),
        ],
        success: &[&["state", "jobId"], &["state", "id"]],
    },
    Tool {
        name: "jobs.cancel",
        description: "Cancel a running or pending job.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("jobId", "string"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
        ],
        required: &["jobId"],
        output: &[("jobId", "string"), ("id", "string"), ("state", "string")],
        success: &[&["state", "jobId"], &["state", "id"]],
    },
    Tool {
        name: "record.start",
        description: "Start headless capture of a display, window or area. Use dryRun to list capture sources and permissions.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("path", "string"),
            ("create", "boolean"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
            ("source", "object"),
            ("display", r#"{"type":["string","integer"]}"#),
            ("window", r#"{"type":["string","integer","object"]}"#),
            ("area", "object"),
            ("mic", "boolean"),
            ("systemAudio", "boolean"),
            ("camera", "boolean"),
            ("events", "boolean"),
            ("insert", "boolean"),
            ("still", "boolean"),
            ("countdown", FRAME),
            ("dryRun", "boolean"),
            ("duration", r#"{"type":"number","exclusiveMinimum":0}"#),
        ],
        required: &[],
        output: &[
            ("jobId", "string"),
            ("state", "string"),
            ("projectId", "string"),
            ("sessionId", "string"),
            ("settings", "object"),
            ("sources", "object"),
            ("permissions", "object"),
            ("identity", "object"),
        ],
        success: &[&["jobId", "state"], &["sources", "permissions"]],
    },
    Tool {
        name: "record.stop",
        description: "Finish a recording and insert the take as one undoable edit. Use insert:false to keep it in media only.",
        read_only: false,
        input: &[
            ("projectId", "string"),
            ("jobId", "string"),
            ("sessionId", "string"),
            ("ifRevision", REVISION),
            ("insert", "boolean"),
        ],
        required: &["jobId"],
        output: &[
            ("jobId", "string"),
            ("id", "string"),
            ("state", "string"),
            ("revision", REVISION),
            ("mediaId", "string"),
            ("clipId", r#"{"type":["string","null"]}"#),
            ("output", "string"),
        ],
        success: &[&["state", "jobId"], &["state", "id"]],
    },
];

/// The code profile lists only `code.run` and `commands.describe`.
pub(super) fn known(name: &str, code: bool) -> bool {
    if code {
        matches!(name, "code.run" | "commands.describe")
    } else {
        TOOLS.iter().any(|tool| tool.name == name)
    }
}

pub(super) fn list(code: bool) -> Json {
    let tools = TOOLS
        .iter()
        .filter(|tool| !code || tool.name == "commands.describe");
    let code_run = code.then_some(&CODE_RUN);
    Json::Array(
        code_run
            .into_iter()
            .chain(tools)
            .map(Tool::description)
            .collect(),
    )
}

impl Tool {
    fn description(&self) -> Json {
        let mut input = properties(self.input);
        input.push(("friction".into(), schema("string")));
        let mut output = properties(self.output);
        output.push(("error".into(), schema(ERROR_SCHEMA)));
        let mut alternatives = vec![object(vec![("required", strings(&["error"]))])];
        alternatives.extend(
            self.success
                .iter()
                .map(|fields| object(vec![("required", strings(fields))])),
        );
        object(vec![
            ("name", text(self.name)),
            ("description", text(self.description)),
            (
                "inputSchema",
                object(vec![
                    ("type", text("object")),
                    ("properties", Json::Object(input)),
                    ("required", strings(self.required)),
                ]),
            ),
            (
                "outputSchema",
                object(vec![
                    ("type", text("object")),
                    ("properties", Json::Object(output)),
                    ("anyOf", Json::Array(alternatives)),
                ]),
            ),
            (
                "annotations",
                object(vec![("readOnlyHint", Json::Bool(self.read_only))]),
            ),
        ])
    }
}

fn properties(fields: &[(&str, &str)]) -> Vec<(String, Json)> {
    fields
        .iter()
        .map(|(key, spec)| ((*key).into(), schema(spec)))
        .collect()
}

fn schema(spec: &str) -> Json {
    if spec.starts_with('{') {
        splice_json::parse(spec).expect("checked tool schema")
    } else {
        object(vec![("type", text(spec))])
    }
}

fn strings(values: &[&str]) -> Json {
    Json::Array(values.iter().map(|value| text(value)).collect())
}

pub(super) fn call(
    projects: &mut Projects,
    name: &str,
    args: &Json,
) -> Result<Json, splice_json::NonFiniteNumber> {
    let mut content = Vec::new();
    let result = execute(projects, name, args, &mut content);
    reply(result, content)
}

pub(super) fn reply(
    result: Result<Json, StoreError>,
    mut content: Vec<Json>,
) -> Result<Json, splice_json::NonFiniteNumber> {
    let is_error = result.is_err();
    let (payload, mirror) = match result {
        Ok(payload) => {
            let mut mirror = String::new();
            splice_json::write(&payload, &mut mirror)?;
            (payload, mirror)
        }
        Err(error) => {
            content.clear();
            let mirror = error.to_string();
            (object(vec![("error", error_payload(error))]), mirror)
        }
    };
    content.insert(
        0,
        object(vec![("type", text("text")), ("text", Json::String(mirror))]),
    );
    Ok(object(vec![
        ("content", Json::Array(content)),
        ("structuredContent", payload),
        ("isError", Json::Bool(is_error)),
    ]))
}

fn execute(
    projects: &mut Projects,
    name: &str,
    args: &Json,
    content: &mut Vec<Json>,
) -> Result<Json, StoreError> {
    args.optional_string("friction")?;
    match name {
        "commands.describe" => {
            args.optional_string("projectId")?;
            let (id, query) = (args.optional_string("id")?, args.optional_string("query")?);
            match args.optional_string("format")? {
                None | Some("json") => Ok(commands::describe(id, query)?),
                Some("ts") => Ok(object(vec![
                    ("format", text("ts")),
                    ("text", Json::String(typescript::declarations(id.or(query)))),
                ])),
                Some(_) => Err(bad("format", "json or ts")),
            }
        }
        "project.info" => projects.info(args),
        "ui.show" => {
            if args.field("projectId").is_some() {
                projects.lookup(args)?;
            }
            Ok(object(vec![
                ("applied", Json::Bool(false)),
                ("reason", text("headless")),
            ]))
        }
        "timeline.snapshot" => timeline::snapshot(projects.lookup(args)?.project(), args),
        "items.inspect" => timeline::inspect(projects.lookup(args)?.project(), args),
        "timeline.find" => timeline::find(projects.lookup(args)?.project(), args),
        "transcript.get" if args.flag("transcribe")? => {
            platform::transcribe(projects.lookup(args)?, args)
        }
        "transcript.get" => media::transcript(projects.lookup(args)?.project(), args),
        "takes.events" => media::events(projects.lookup(args)?, args),
        "audio.summary" => media::audio(projects.lookup(args)?, args),
        "session.start" => history::start(projects.lookup(args)?, args),
        "edit.apply" => history::apply(projects.lookup(args)?, args),
        "history.list" => history::list(projects.lookup(args)?, args),
        "history.undo" => history::undo(projects.lookup(args)?, args),
        "view.frames" => platform::frames(projects.lookup(args)?, args, content),
        "export.start" => platform::export(projects, args),
        "jobs.status" | "jobs.cancel" => platform::job(projects, name, args),
        "record.start" => platform::record_start(projects, args),
        "record.stop" => platform::record_stop(projects, args),
        _ => unreachable!("RPC only dispatches advertised tools"),
    }
}
