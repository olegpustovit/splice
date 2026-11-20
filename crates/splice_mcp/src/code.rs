//! `code.run` (T14): agent JavaScript that calls the tools through one bounded QuickJS run.
use super::{Json, object, text, tools, wire::*};
use quickjs_sys::{Kind, Limits};
use splice_core::{
    commands::{COMMANDS, Tier},
    history::StoreError,
};
use std::time::{Duration, Instant};

pub(super) const LIMITS: Limits = Limits {
    memory: 32 << 20,
    stack: 1 << 20,
    wall: Duration::from_secs(2),
    calls: 200,
    source: 64 << 10,
    output: 32 << 10,
};

/// The global `splice`: `call`, `describe`, `op.<group>.<name>` builders and one
/// `<group>.<name>` function per tool. `__CONTEXT__` is replaced with the run's JSON context.
const PRELUDE: &str = r#"(() => {
  const g = globalThis, host = g.__host, ctx = __CONTEXT__;
  delete g.__host;
  class SpliceError extends Error {
    constructor(e) { super(e.message); this.name = "SpliceError"; Object.assign(this, e); }
  }
  const call = (name, args) => {
    const a = Object.assign({}, args), t = ctx.tools[name];
    if (t && t[0] && a.projectId === undefined && a.path === undefined && ctx.projectId) a.projectId = ctx.projectId;
    if (t && t[1] && a.sessionId === undefined && !ctx.sessionId && ctx.projectId && name !== "project.info") {
      // A session opened outside this program (record.start starts one) is the default too.
      const info = call("project.info", { projectId: ctx.projectId });
      if (info && typeof info.openSession === "string") ctx.sessionId = info.openSession;
    }
    if (t && t[1] && a.sessionId === undefined && ctx.sessionId) a.sessionId = ctx.sessionId;
    // ifRevision defaults to the newest revision this program saw of its project, never to one
    // it did not read: the precondition still guards against changes made by someone else.
    if (t && t[2] && a.ifRevision === undefined && ctx.revision && (a.projectId === undefined || a.projectId === ctx.projectId)) a.ifRevision = ctx.revision;
    let r;
    try { r = host(String(name), JSON.stringify(a)); }
    catch (e) { throw e && typeof e === "object" && !(e instanceof Error) ? new SpliceError(e) : e; }
    // The first project opened and session started become the defaults for later calls.
    if (r && typeof r === "object") {
      if (!ctx.projectId && typeof r.projectId === "string") ctx.projectId = r.projectId;
      if (!ctx.sessionId && typeof r.sessionId === "string") ctx.sessionId = r.sessionId;
      if (!ctx.sessionId && typeof r.openSession === "string") ctx.sessionId = r.openSession;
      const pid = typeof r.projectId === "string" ? r.projectId : a.projectId || ctx.projectId;
      if (typeof r.revision === "string" && pid === ctx.projectId) ctx.revision = r.revision;
    }
    return r;
  };
  const describe = (query) => call("commands.describe", { format: "ts", query: query === undefined ? "" : String(query) }).text;
  const splice = { get projectId() { return ctx.projectId; }, get sessionId() { return ctx.sessionId; }, call, describe, op: {} };
  const place = (root, name, value) => {
    const [group, method] = name.split(".");
    (root[group] = root[group] || {})[method] = value;
  };
  for (const name of Object.keys(ctx.tools)) place(splice, name, (args) => call(name, args));
  for (const cmd of ctx.commands) place(splice.op, cmd, (args) => Object.assign({ cmd }, args));
  g.SpliceError = SpliceError;
  g.splice = splice;
})();
"#;

/// Tools code can call: everything but recording (it needs a human-facing job) and code.run.
fn callable(name: &str) -> bool {
    name != "code.run" && tools::known(name, false)
}

/// Runs `args.code`; `call` dispatches one tool exactly like a top-level `tools/call`.
#[cfg(test)]
pub(super) fn run(
    args: &Json,
    call: &mut dyn FnMut(&str, &Json) -> Result<Json, splice_json::NonFiniteNumber>,
) -> Result<Json, StoreError> {
    run_with_images(args, call, &mut Vec::new())
}

/// Most frame images a run passes back (the same ≤ 640 px previews `view.frames` returns
/// directly), so agents see frames without reading full-size PNGs from disk.
const MAX_IMAGES: usize = 6;

/// Runs `code`, collecting the image content of its tool calls into `images`.
pub(super) fn run_with_images(
    args: &Json,
    call: &mut dyn FnMut(&str, &Json) -> Result<Json, splice_json::NonFiniteNumber>,
    images: &mut Vec<Json>,
) -> Result<Json, StoreError> {
    let code = args.string("code")?;
    let project = args.optional_string("projectId")?;
    let session = args.optional_string("sessionId")?;
    let prelude = PRELUDE.replace("__CONTEXT__", &context(project, session)?);
    let mut calls = Vec::new();
    let mut revisions = Revisions::default();
    let mut jobs = Vec::new();
    let outcome = quickjs_sys::run(&prelude, code, &LIMITS, &mut |name, args| {
        host(
            name,
            args,
            call,
            &mut calls,
            &mut revisions,
            &mut jobs,
            images,
        )
    });
    let parse = |json: &str| splice_json::parse(json).unwrap_or(Json::Null);
    let mut fields = vec![
        ("result", parse(&outcome.result)),
        (
            "logs",
            Json::Array(outcome.logs.into_iter().map(Json::String).collect()),
        ),
        ("calls", Json::Array(calls.iter().map(Call::json).collect())),
        ("revisions", revisions.json()),
    ];
    // Jobs the code started survive a failed or timed-out program, so they can still be polled.
    if !jobs.is_empty() {
        fields.push(("jobs", Json::Array(jobs)));
    }
    if let Some(failure) = outcome.error {
        fields.push(("error", error(&failure, &parse(&failure.data))));
    }
    Ok(object(fields))
}

/// Sets `isError` when the code failed, keeping its logs and calls in the payload.
pub(super) fn reply(
    payload: Result<Json, StoreError>,
    images: Vec<Json>,
) -> Result<Json, splice_json::NonFiniteNumber> {
    let failed = payload
        .as_ref()
        .is_ok_and(|payload| payload.field("error").is_some());
    let mut reply = tools::reply(payload, images)?;
    if failed && let Json::Object(fields) = &mut reply {
        for (key, value) in fields {
            if key == "isError" {
                *value = Json::Bool(true);
            }
        }
    }
    Ok(reply)
}

fn context(project: Option<&str>, session: Option<&str>) -> Result<String, StoreError> {
    let optional = |value: Option<&str>| value.map_or(Json::Null, text);
    let tools = tools::TOOLS
        .iter()
        .filter(|tool| callable(tool.name))
        .map(|tool| {
            let accepts = |field: &str| Json::Bool(tool.input.iter().any(|(key, _)| *key == field));
            (
                tool.name.to_string(),
                Json::Array(vec![
                    accepts("projectId"),
                    accepts("sessionId"),
                    accepts("ifRevision"),
                ]),
            )
        })
        .collect();
    let context = object(vec![
        ("projectId", optional(project)),
        ("sessionId", optional(session)),
        ("revision", Json::Null),
        ("tools", Json::Object(tools)),
        (
            "commands",
            Json::Array(
                COMMANDS
                    .iter()
                    .filter(|row| row.tier != Tier::Planned)
                    .map(|row| text(row.id))
                    .collect(),
            ),
        ),
    ]);
    let mut json = String::new();
    splice_json::write(&context, &mut json).map_err(|_| bad("projectId", "a string"))?;
    Ok(json)
}

/// One trace entry: consecutive calls to the same tool with the same outcome collapse into one
/// (a polling loop reports `count` instead of hundreds of identical rows).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    tool: String,
    ok: bool,
    ms: u64,
    count: u32,
}

impl Call {
    fn json(&self) -> Json {
        let mut fields = vec![
            ("tool", text(&self.tool)),
            ("ok", Json::Bool(self.ok)),
            ("ms", Json::Unsigned(self.ms)),
        ];
        if self.count > 1 {
            fields.push(("count", Json::Unsigned(u64::from(self.count))));
        }
        object(fields)
    }
}

fn host(
    name: &str,
    args: &str,
    call: &mut dyn FnMut(&str, &Json) -> Result<Json, splice_json::NonFiniteNumber>,
    calls: &mut Vec<Call>,
    revisions: &mut Revisions,
    jobs: &mut Vec<Json>,
    images: &mut Vec<Json>,
) -> Result<String, String> {
    let began = Instant::now();
    let result = dispatch(name, args, call, revisions, images);
    if let Some(job) = result
        .as_ref()
        .ok()
        .and_then(|value| value.field("jobId"))
        .filter(|job| !jobs.contains(job))
    {
        jobs.push(job.clone());
    }
    let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    match calls.last_mut() {
        Some(last) if last.tool == name && last.ok == result.is_ok() => {
            last.count += 1;
            last.ms = last.ms.saturating_add(ms);
        }
        _ => calls.push(Call {
            tool: name.into(),
            ok: result.is_ok(),
            ms,
            count: 1,
        }),
    }
    let (Ok(value) | Err(value)) = &result;
    let mut json = String::new();
    if splice_json::write(value, &mut json).is_err() {
        return Err(r#"{"code":"INTERNAL","message":"The tool returned a non-finite number.","fix":"Report this as friction."}"#.into());
    }
    match result {
        Ok(_) => Ok(json),
        Err(_) => Err(json),
    }
}

/// Tool names agents try that Splice spells differently.
pub(super) const ALIASES: &[(&str, &str)] = &[
    ("project.open", "project.info"),
    ("project.load", "project.info"),
    ("project.create", "project.info"),
    ("job.status", "jobs.status"),
    ("record.status", "jobs.status"),
    ("timeline.inspect", "items.inspect"),
    ("item.inspect", "items.inspect"),
    ("edit.batch", "edit.apply"),
    ("history.revert", "history.undo"),
];

/// One tool call: its structured result, or its error payload.
fn dispatch(
    name: &str,
    args: &str,
    call: &mut dyn FnMut(&str, &Json) -> Result<Json, splice_json::NonFiniteNumber>,
    revisions: &mut Revisions,
    images: &mut Vec<Json>,
) -> Result<Json, Json> {
    if !callable(name) {
        let valid = tools::TOOLS.iter().filter(|tool| callable(tool.name));
        // Names agents reach for first, then the nearest spelling.
        let near = ALIASES
            .iter()
            .find(|(alias, _)| *alias == name)
            .map(|(_, tool)| (*tool).to_owned())
            .or_else(|| {
                splice_core::commands::closest(
                    name,
                    &mut tools::TOOLS
                        .iter()
                        .filter(|tool| callable(tool.name))
                        .map(|tool| tool.name),
                )
            });
        let message = match &near {
            Some(near) => format!("Code cannot call {name:?}. Did you mean {near:?}?"),
            None => format!("Code cannot call {name:?}."),
        };
        let fix = match (name, &near) {
            ("session.end" | "session.close" | "session.stop", _) => "Sessions need no end: the next session.start opens a new one, and history.undo {session} reverts a whole session.".to_owned(),
            (_, Some(near)) => format!("Call {near}, or one of the tools in `valid`."),
            (_, None) => "Call one of the tools in `valid`.".to_owned(),
        };
        return Err(object(vec![
            ("code", text("UNKNOWN_TOOL")),
            ("message", Json::String(message)),
            ("fix", Json::String(fix)),
            (
                "valid",
                Json::Array(valid.map(|tool| text(tool.name)).collect()),
            ),
        ]));
    }
    let args = splice_json::parse(args)
        .ok()
        .filter(|args| args.as_object().is_some())
        .ok_or_else(|| error_payload(bad("args", "an object")))?;
    let invalid = || {
        error_payload(failure(
            "INTERNAL",
            "The tool returned invalid JSON.",
            "Report this as friction.",
        ))
    };
    let reply = call(name, &args).map_err(|_| invalid())?;
    let content = reply
        .field("content")
        .and_then(Json::as_array)
        .unwrap_or(&[]);
    for image in content
        .iter()
        .filter(|part| part.field("type").and_then(Json::as_str) == Some("image"))
    {
        if images.len() < MAX_IMAGES {
            images.push(image.clone());
        }
    }
    let payload = reply.field("structuredContent").ok_or_else(invalid)?;
    if let Some(error) = payload.field("error") {
        return Err(error.clone());
    }
    revisions.see(&args, payload);
    Ok(payload.clone())
}

/// Each project the code touched: its revision before the first call and after the last
/// (T23.2). The first project's range is the top-level `from`/`to`; others are listed by ID.
#[derive(Default)]
struct Revisions {
    seen: Vec<(String, Json, Json)>,
}

impl Revisions {
    fn see(&mut self, args: &Json, payload: &Json) {
        let Some(project) = args
            .field("projectId")
            .or_else(|| payload.field("projectId"))
            .and_then(Json::as_str)
        else {
            return;
        };
        let Some(revision) = payload.field("revision") else {
            return;
        };
        match self.seen.iter_mut().find(|(id, ..)| id == project) {
            Some((_, _, to)) => *to = revision.clone(),
            // An edit's precondition is the state it started from.
            None => self.seen.push((
                project.into(),
                args.field("ifRevision").unwrap_or(revision).clone(),
                revision.clone(),
            )),
        }
    }

    fn json(self) -> Json {
        let mut seen = self.seen.into_iter();
        let Some((_, from, to)) = seen.next() else {
            return Json::Null;
        };
        let mut fields = vec![("from".to_owned(), from), ("to".to_owned(), to)];
        let others: Vec<(String, Json)> = seen
            .map(|(id, from, to)| (id, object(vec![("from", from), ("to", to)])))
            .collect();
        if !others.is_empty() {
            fields.push(("projects".to_owned(), Json::Object(others)));
        }
        Json::Object(fields)
    }
}

fn error(failure: &quickjs_sys::Failure, data: &Json) -> Json {
    let line = failure
        .line
        .map_or(Json::Null, |line| Json::Unsigned(u64::from(line)));
    let at = failure
        .line
        .map_or_else(String::new, |line| format!(" at line {line}"));
    let (code, fix) = match failure.kind {
        Kind::Timeout => (
            "CODE_TIMEOUT",
            "Keep JavaScript under 2 s (tool time is not counted): remove unbounded loops.".into(),
        ),
        Kind::Memory => (
            "CODE_MEMORY",
            "Stay under 32 MB: page with limit and cursor instead of collecting everything.".into(),
        ),
        Kind::Stack => (
            "CODE_STACK",
            format!("Replace the deep recursion{at} with a loop."),
        ),
        Kind::Calls => (
            "CODE_CALLS",
            "Use at most 200 tool calls per run: batch ops into one edit.apply.".into(),
        ),
        Kind::Source => (
            "CODE_TOO_LARGE",
            "Send at most 64 KB of code; split the work into several code.run calls.".into(),
        ),
        Kind::Output => (
            "CODE_OUTPUT",
            "Return a smaller summary (at most 32 KB of JSON): pick the fields you need.".into(),
        ),
        Kind::Runtime => (
            "CODE_RUNTIME",
            "Retry the call; report it as friction if it repeats.".into(),
        ),
        Kind::Exception => match data.field("code").and_then(Json::as_str) {
            // An uncaught SpliceError keeps the tool's own code, fix and choices.
            Some(_) if data.field("fix").is_some() => {
                let mut fields = vec![
                    ("code", data.field("code").cloned().unwrap_or(Json::Null)),
                    (
                        "message",
                        text(
                            failure
                                .message
                                .strip_prefix("SpliceError: ")
                                .unwrap_or(&failure.message),
                        ),
                    ),
                    ("fix", data.field("fix").cloned().unwrap_or(Json::Null)),
                    ("line", line),
                ];
                for key in ["valid", "current", "changedSince", "op"] {
                    if let Some(value) = data.field(key) {
                        fields.push((key, value.clone()));
                    }
                }
                return object(fields);
            }
            _ => (
                "CODE_ERROR",
                format!("Fix the JavaScript{at} and run it again."),
            ),
        },
    };
    object(vec![
        ("code", text(code)),
        ("message", text(&failure.message)),
        ("fix", Json::String(fix)),
        ("line", line),
    ])
}
