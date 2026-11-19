use super::{Json, friction::Recorder, object, paths, project::Projects, text, tools};
use splice_core::history::StoreError;
use std::io::{self, BufRead, Read, Write};
use std::panic;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

const LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";
const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;
/// `<crate version>+<git short sha>[-dirty]` (`+unknown` outside a checkout), from build.rs.
const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("SPLICE_BUILD"));
/// Jobs watched for progress and cancellation at once; the oldest is forgotten first.
const MAX_WATCHED: usize = 64;
/// Encoding time per turn of the serve loop while an export runs, so requests still get answers.
#[cfg(target_os = "macos")]
pub(super) const EXPORT_SLICE: std::time::Duration = std::time::Duration::from_millis(50);

struct Connection {
    initialized: bool,
    /// The code profile (`splice mcp --code`): only `code.run` and `commands.describe`.
    code: bool,
    projects: Projects,
    friction: Recorder,
    #[cfg(target_os = "macos")]
    routes: super::live::proxy::Routes,
    /// Handoff sockets for the packages this headless session holds (T10.1).
    #[cfg(target_os = "macos")]
    holders: Vec<super::live::Server>,
    #[cfg(target_os = "macos")]
    events: Option<Sender<Event>>,
    /// Jobs started by requests (T23.3): job ID, the request ID, the client's progress token
    /// and the last progress sent.
    watched: Vec<(String, Json, Option<Json>, f64)>,
}

enum Event {
    Line(Vec<u8>),
    End,
    Failed(io::Error),
    #[cfg(target_os = "macos")]
    Handoff(PathBuf, super::live::Request),
}

#[cfg(target_os = "macos")]
pub(super) fn record_project(
    args: &Json,
    stop: &mut dyn FnMut() -> io::Result<bool>,
) -> Result<Json, Json> {
    let mut connection = Connection {
        initialized: true,
        code: false,
        projects: Projects::default(),
        friction: Recorder::new(paths::state_dir().ok()),
        routes: Default::default(),
        holders: Vec::new(),
        events: None,
        watched: Vec::new(),
    };
    let reply = super::record_cli::run(args, stop, &mut |name, arguments| {
        let reply = connection
            .call(name, arguments, None)
            .map_err(|error| super::record_cli::io_error(&error))?;
        if let Some(result) = reply.field("structuredContent") {
            if let Some(error) = result.field("error") {
                return Err(error.clone());
            }
            Ok(result.clone())
        } else {
            Err(super::record_cli::io_error(&"Missing recording reply."))
        }
    });
    let closed = connection
        .projects
        .close()
        .map_err(|error| super::record_cli::io_error(&error));
    reply.and_then(|value| closed.map(|()| value))
}

/// Serves one MCP connection, flushing each reply and returning on end of input.
/// `code` selects the code profile (`code.run` and `commands.describe`).
pub fn serve(
    input: &mut (dyn BufRead + Send),
    output: &mut dyn Write,
    code: bool,
) -> io::Result<()> {
    let global = match paths::state_dir() {
        Ok(dir) => Some(dir),
        Err(error) => {
            eprintln!("splice friction: {error}");
            None
        }
    };
    serve_with_log(input, output, global, code)
}

pub(super) fn serve_with_log(
    input: &mut (dyn BufRead + Send),
    output: &mut dyn Write,
    global: Option<PathBuf>,
    code: bool,
) -> io::Result<()> {
    let (events, incoming) = mpsc::channel();
    let mut connection = Connection {
        initialized: false,
        code,
        projects: Projects::default(),
        friction: Recorder::new(global),
        #[cfg(target_os = "macos")]
        routes: Default::default(),
        #[cfg(target_os = "macos")]
        holders: Vec::new(),
        #[cfg(target_os = "macos")]
        events: Some(events.clone()),
        watched: Vec::new(),
    };
    // Stdin is read on its own thread so a viewer's handoff request is served while idle.
    thread::scope(|scope| {
        scope.spawn(move || read_lines(input, &events));
        let mut wire = String::new();
        loop {
            // Exports encode on this thread (OGPU's Metal handles live here) between requests.
            #[cfg(target_os = "macos")]
            for note in connection.progress() {
                wire.clear();
                splice_json::write(&note, &mut wire).map_err(io::Error::other)?;
                wire.push('\n');
                output.write_all(wire.as_bytes())?;
                output.flush()?;
            }
            #[cfg(target_os = "macos")]
            let next = if connection.projects.tick_exports(EXPORT_SLICE) {
                match incoming.recv_timeout(std::time::Duration::ZERO) {
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    other => other.map_err(|_| mpsc::RecvError),
                }
            } else {
                incoming.recv()
            };
            #[cfg(not(target_os = "macos"))]
            let next = incoming.recv();
            let line = match next {
                Ok(Event::Line(line)) => line,
                Ok(Event::End) | Err(_) => return connection.close(),
                Ok(Event::Failed(error)) => return Err(error),
                #[cfg(target_os = "macos")]
                Ok(Event::Handoff(path, request)) => {
                    connection.hand_off(&path, request);
                    continue;
                }
            };
            let response = if line.len() as u64 > MAX_LINE_BYTES {
                Some(rpc_error(&Json::Null, -32700, "Message exceeds 16 MiB"))
            } else {
                handle_line(&line, &mut connection)
            };
            #[cfg(target_os = "macos")]
            connection.hold();
            if let Some(response) = response {
                wire.clear();
                splice_json::write(&response, &mut wire).map_err(io::Error::other)?;
                wire.push('\n');
                output.write_all(wire.as_bytes())?;
                output.flush()?;
            }
        }
    })
}

fn read_lines(input: &mut dyn BufRead, events: &Sender<Event>) {
    loop {
        let mut line = Vec::new();
        let event = match (&mut *input)
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)
        {
            Err(error) => Event::Failed(error),
            Ok(_) if line.is_empty() => Event::End,
            Ok(_) if line.len() as u64 > MAX_LINE_BYTES && !line.ends_with(b"\n") => {
                match input.skip_until(b'\n') {
                    Ok(_) => Event::Line(line),
                    Err(error) => Event::Failed(error),
                }
            }
            Ok(_) => Event::Line(line),
        };
        let more = matches!(event, Event::Line(_));
        if events.send(event).is_err() || !more {
            return;
        }
    }
}

impl Connection {
    fn close(self) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        drop(self.holders);
        self.projects.close()
    }

    /// Keeps one handoff socket per package this session holds.
    #[cfg(target_os = "macos")]
    fn hold(&mut self) {
        let Some(events) = &self.events else {
            return;
        };
        let local = self.projects.local_paths();
        self.holders
            .retain(|holder| local.contains(&holder.project()));
        for path in local {
            if self.holders.iter().any(|holder| holder.project() == path) {
                continue;
            }
            let Ok(state) = paths::state_dir() else {
                return;
            };
            let events = events.clone();
            let project = path.to_path_buf();
            match super::live::Server::hold(
                path,
                &state,
                Box::new(move |request| {
                    events
                        .send(Event::Handoff(project.clone(), request))
                        .map_err(io::Error::other)
                }),
            ) {
                Ok(holder) => self.holders.push(holder),
                Err(error) => {
                    eprintln!("splice: no handoff socket for {}: {error}", path.display())
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn hand_off(&mut self, path: &std::path::Path, request: super::live::Request) {
        let result = if request.body.field("command").and_then(Json::as_str) == Some("handoff") {
            self.projects.hand_off(path)
        } else {
            Err(super::wire::bad("command", "handoff"))
        };
        request.reply(match result {
            Ok(()) => object(vec![("handedOff", Json::Bool(true))]),
            Err(error) => object(vec![("error", super::wire::error_payload(error))]),
        });
        // After the reply: dropping the holder joins the socket worker that waited for it.
        self.hold();
    }
}

fn handle_line(line: &[u8], connection: &mut Connection) -> Option<Json> {
    let Some(request) = str::from_utf8(line)
        .ok()
        .and_then(|line| splice_json::parse(line).ok())
    else {
        return Some(rpc_error(&Json::Null, -32700, "Parse error"));
    };
    let id = request.field("id");
    if id.is_some_and(|id| !matches!(id, Json::Signed(_) | Json::Unsigned(_) | Json::String(_))) {
        return Some(rpc_error(&Json::Null, -32600, "Invalid request ID"));
    }
    let method = request.field("method").and_then(Json::as_str);
    if request.field("jsonrpc").and_then(Json::as_str) != Some("2.0")
        || method.is_none()
        || request.field("result").is_some()
        || request.field("error").is_some()
    {
        return Some(rpc_error(
            id.unwrap_or(&Json::Null),
            -32600,
            "Invalid request",
        ));
    }
    // Notification messages have no reply and never invoke a request handler; a cancellation
    // stops the job its request started.
    if id.is_none() && method == Some("notifications/cancelled") {
        if let Some(request) = request.field("params").and_then(|p| p.field("requestId")) {
            connection.cancel(request);
        }
        return None;
    }
    let id = id?;
    let method = method.expect("checked method");
    let params = request.field("params").unwrap_or(&EMPTY_OBJECT);
    if params.as_object().is_none() {
        return Some(rpc_error(id, -32602, "params must be an object"));
    }
    Some(dispatch(id, method, params, connection))
}

static EMPTY_OBJECT: Json = Json::Object(Vec::new());

fn dispatch(id: &Json, method: &str, params: &Json, connection: &mut Connection) -> Json {
    let meta = params.field("_meta").unwrap_or(&EMPTY_OBJECT);
    if meta.as_object().is_none() {
        return rpc_error(id, -32602, "params._meta must be an object");
    }
    let modern = method == "server/discover"
        || meta.field(PROTOCOL_VERSION).is_some()
        || meta.field(CLIENT_CAPABILITIES).is_some();
    if modern {
        let Some(version) = meta.field(PROTOCOL_VERSION).and_then(Json::as_str) else {
            return rpc_error(
                id,
                -32602,
                "Pass protocolVersion as a string in params._meta",
            );
        };
        if version != MODERN {
            return message(
                id,
                "error",
                object(vec![
                    ("code", Json::Signed(-32022)),
                    ("message", text("Unsupported protocol version")),
                    (
                        "data",
                        object(vec![
                            ("supported", supported_versions()),
                            ("requested", text(version)),
                        ]),
                    ),
                ]),
            );
        }
        if meta
            .field(CLIENT_CAPABILITIES)
            .and_then(Json::as_object)
            .is_none()
        {
            return rpc_error(
                id,
                -32602,
                "Pass clientCapabilities as an object in params._meta",
            );
        }
    }
    let mut result = match method {
        "initialize" if !modern => {
            if connection.initialized {
                return rpc_error(id, -32600, "Connection already initialized");
            }
            let Some(requested) = params.field("protocolVersion").and_then(Json::as_str) else {
                return rpc_error(id, -32602, "initialize requires a protocolVersion string");
            };
            let client = params.field("clientInfo");
            if params
                .field("capabilities")
                .and_then(Json::as_object)
                .is_none()
                || client
                    .and_then(|info| info.field("name"))
                    .and_then(Json::as_str)
                    .is_none()
                || client
                    .and_then(|info| info.field("version"))
                    .and_then(Json::as_str)
                    .is_none()
            {
                return rpc_error(
                    id,
                    -32602,
                    "initialize requires capabilities and clientInfo",
                );
            }
            connection.initialized = true;
            connection
                .friction
                .initialize(client.expect("checked clientInfo"));
            let version = if requested == MODERN { MODERN } else { LEGACY };
            object(vec![
                ("protocolVersion", text(version)),
                ("capabilities", capabilities()),
                ("serverInfo", server_info()),
            ])
        }
        "server/discover" => object(vec![
            ("supportedVersions", supported_versions()),
            ("capabilities", capabilities()),
        ]),
        "ping" => Json::Object(Vec::new()),
        "tools/list" | "tools/call" if !modern && !connection.initialized => {
            return rpc_error(id, -32600, "Initialize before calling tools");
        }
        "tools/list" => {
            if params.field("cursor").is_some() {
                return rpc_error(id, -32602, "The tool list has no next page");
            }
            object(vec![("tools", tools::list(connection.code))])
        }
        "tools/call" => {
            let name = params.field("name").and_then(Json::as_str);
            let args = params.field("arguments").unwrap_or(&EMPTY_OBJECT);
            let Some(name) = name.filter(|_| args.as_object().is_some()) else {
                return rpc_error(
                    id,
                    -32602,
                    "tools/call requires a name and object arguments",
                );
            };
            if !tools::known(name, connection.code) {
                return rpc_error(id, -32602, &format!("Unknown tool: {name}"));
            }
            match connection.call(name, args, modern.then_some(meta)) {
                Ok(result) => {
                    connection.watch(name, args, &result, id, meta);
                    result
                }
                Err(_) => return rpc_error(id, -32603, "Tool returned invalid JSON"),
            }
        }
        _ => return rpc_error(id, -32601, "Method not found"),
    };
    if modern {
        let Json::Object(fields) = &mut result else {
            unreachable!("MCP results are objects");
        };
        fields.insert(0, ("resultType".into(), text("complete")));
        fields.push(("_meta".into(), object(vec![(SERVER_INFO, server_info())])));
    }
    message(id, "result", result)
}

impl Connection {
    /// Progress notifications for watched jobs that moved (and forgets finished ones).
    #[cfg(target_os = "macos")]
    fn progress(&mut self) -> Vec<Json> {
        let mut notes = Vec::new();
        let projects = &mut self.projects;
        self.watched.retain_mut(|(job, _, token, last)| {
            let Some((active, progress)) = job.parse().ok().and_then(|id| projects.progress(id))
            else {
                return false;
            };
            if let Some(token) = token
                && (progress - *last >= 0.01 || !active)
            {
                *last = progress;
                notes.push(object(vec![
                    ("jsonrpc", text("2.0")),
                    ("method", text("notifications/progress")),
                    (
                        "params",
                        object(vec![
                            ("progressToken", token.clone()),
                            (
                                "progress",
                                Json::Float((progress * 1000.0).round() / 1000.0),
                            ),
                            ("total", Json::Unsigned(1)),
                        ]),
                    ),
                ]));
            }
            active
        });
        notes
    }

    /// Watches a job this request started (H10): never one a status, cancel, stop or resume
    /// call only names, so cancelling that call leaves the job alone. The list is bounded.
    fn watch(&mut self, name: &str, args: &Json, result: &Json, id: &Json, meta: &Json) {
        let starts = match name {
            "export.start" | "media.analyze" | "transcript.get" => true,
            "record.start" => args.field("resume").is_none(),
            _ => false,
        };
        let Some(job) = result
            .field("structuredContent")
            .and_then(|content| content.field("jobId"))
            .and_then(Json::as_str)
            .filter(|_| starts)
        else {
            return;
        };
        self.watched.retain(|(watched, ..)| watched != job);
        if self.watched.len() == MAX_WATCHED {
            self.watched.remove(0);
        }
        self.watched.push((
            job.to_owned(),
            id.clone(),
            meta.field("progressToken").cloned(),
            0.0,
        ));
    }

    /// `notifications/cancelled`: cancels the job the request started, if any.
    fn cancel(&mut self, request: &Json) {
        if let Some((job, ..)) = self.watched.iter().find(|(_, id, ..)| id == request) {
            #[cfg(target_os = "macos")]
            if let Ok(id) = job.parse() {
                self.projects.cancel(id);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = job;
        }
    }

    /// Runs one tool call; a panic becomes an `INTERNAL` error result instead of ending the
    /// server. Calls nested in `code.run` come back here, so a panic never unwinds through
    /// the JavaScript engine.
    fn call(
        &mut self,
        name: &str,
        args: &Json,
        meta: Option<&Json>,
    ) -> Result<Json, splice_json::NonFiniteNumber> {
        let result =
            panic::catch_unwind(panic::AssertUnwindSafe(|| self.call_tool(name, args, meta)));
        match result {
            Ok(result) => result,
            Err(payload) => {
                let result = tools::reply(Err(internal(name, payload.as_ref())), Vec::new())?;
                self.friction
                    .call(&self.projects, name, args, &result, meta);
                Ok(result)
            }
        }
    }

    fn call_tool(
        &mut self,
        name: &str,
        args: &Json,
        meta: Option<&Json>,
    ) -> Result<Json, splice_json::NonFiniteNumber> {
        if name == "code.run" {
            // Each tool call inside the code takes the same path, so viewer forwarding and
            // friction logging work unchanged.
            let mut images = Vec::new();
            let payload = super::code::run_with_images(
                args,
                &mut |tool, tool_args| self.call(tool, tool_args, meta),
                &mut images,
            );
            let result = super::code::reply(payload, images)?;
            self.friction
                .call(&self.projects, name, args, &result, meta);
            return Ok(result);
        }
        #[cfg(target_os = "macos")]
        let result = match self.routes.tool_args(name, args) {
            Ok(routed) => match self.routes.forward(
                &mut self.projects,
                name,
                args,
                &routed,
                self.friction.client(meta),
            ) {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {
                    let mut result = tools::call(&mut self.projects, name, &routed)?;
                    self.routes.remap_job(name, args, &mut result, None);
                    result
                }
                Err(error) => tools::reply(Err(error), Vec::new())?,
            },
            Err(error) => tools::reply(Err(error), Vec::new())?,
        };
        #[cfg(not(target_os = "macos"))]
        let result = tools::call(&mut self.projects, name, args)?;
        self.friction
            .call(&self.projects, name, args, &result, meta);
        Ok(result)
    }
}

/// The error result for a tool call that panicked.
fn internal(tool: &str, payload: &(dyn std::any::Any + Send)) -> StoreError {
    let reason = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic");
    StoreError::new(
        "INTERNAL",
        &format!("{tool} failed inside Splice: {reason}."),
        "This is a Splice bug and was logged; re-read the project state before retrying.",
    )
}

fn capabilities() -> Json {
    object(vec![(
        "tools",
        object(vec![("listChanged", Json::Bool(false))]),
    )])
}

fn server_info() -> Json {
    object(vec![("name", text("splice")), ("version", text(VERSION))])
}

fn supported_versions() -> Json {
    Json::Array(vec![text(MODERN), text(LEGACY)])
}

fn rpc_error(id: &Json, code: i64, reason: &str) -> Json {
    message(
        id,
        "error",
        object(vec![
            ("code", Json::Signed(code)),
            ("message", text(reason)),
        ]),
    )
}

fn message(id: &Json, key: &str, value: Json) -> Json {
    Json::Object(vec![
        ("jsonrpc".into(), text("2.0")),
        ("id".into(), id.clone()),
        (key.into(), value),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::TestDirectory;
    use crate::wire::Args;

    fn connection() -> Connection {
        Connection {
            initialized: true,
            code: false,
            projects: Projects::default(),
            friction: Recorder::new(None),
            #[cfg(target_os = "macos")]
            routes: Default::default(),
            #[cfg(target_os = "macos")]
            holders: Vec::new(),
            #[cfg(target_os = "macos")]
            events: None,
            watched: Vec::new(),
        }
    }

    fn call(connection: &mut Connection, id: u64, tool: &str, args: &str) -> Json {
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#
        );
        let reply = handle_line(line.as_bytes(), connection).expect("a reply");
        reply
            .field("result")
            .and_then(|result| result.field("structuredContent"))
            .cloned()
            .expect("a tool result")
    }

    #[test]
    fn low_a_panicking_tool_call_is_an_internal_error_result() {
        let error = internal("edit.apply", &"index out of bounds");
        assert_eq!(error.code, "INTERNAL");
        assert!(error.message.contains("index out of bounds"));
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("boom"));
        assert!(internal("x", owned.as_ref()).message.contains("boom"));
    }

    #[test]
    fn low_unknown_tool_arguments_are_refused_with_the_closest_name() {
        let mut connection = connection();
        let error = call(
            &mut connection,
            1,
            "commands.describe",
            r#"{"querry":"clip"}"#,
        );
        let error = error.field("error").unwrap();
        assert_eq!(error.string("code").unwrap(), "UNKNOWN_PARAM");
        assert_eq!(error.string("param").unwrap(), "querry");
        assert_eq!(error.string("didYouMean").unwrap(), "query");
        connection.close().unwrap();
    }

    #[test]
    fn t36_9_server_info_names_the_build() {
        let (version, build) = VERSION.split_once('+').unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        let sha = build.strip_suffix("-dirty").unwrap_or(build);
        assert!(
            sha == "unknown" || (sha.len() >= 7 && sha.bytes().all(|b| b.is_ascii_hexdigit())),
            "{build}"
        );
        let mut connection = connection();
        connection.initialized = false;
        let reply = handle_line(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            &mut connection,
        )
        .unwrap();
        let info = reply.field("result").unwrap().field("serverInfo").unwrap();
        assert_eq!(info.string("version").unwrap(), VERSION);
    }

    #[test]
    fn h10_cancelling_a_status_request_keeps_the_job() {
        let temp = TestDirectory::new("rpc-watch");
        let package = temp.0.join("Watch.splice");
        let mut connection = connection();
        let opened = call(
            &mut connection,
            1,
            "project.info",
            &format!(r#"{{"path":"{}","create":true}}"#, package.display()),
        );
        let project = opened.string("projectId").unwrap().to_owned();
        let export = format!(r#"{{"projectId":"{project}","ifRevision":0,"format":"fcpxml"}}"#);
        let started = call(&mut connection, 2, "export.start", &export);
        let job = started.string("jobId").unwrap().to_owned();
        let status = call(
            &mut connection,
            3,
            "jobs.status",
            &format!(r#"{{"jobId":"{job}"}}"#),
        );
        assert_eq!(status.string("jobId").unwrap(), job);
        // Only the request that started the job owns it: cancelling the status call is a no-op.
        assert_eq!(connection.watched.len(), 1);
        assert_eq!(connection.watched[0].0, job);
        assert_eq!(connection.watched[0].1, Json::Unsigned(2));
        handle_line(
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3}}"#,
            &mut connection,
        );
        // The watch list stays bounded however many jobs a connection starts.
        for id in 4..4 + MAX_WATCHED as u64 {
            call(&mut connection, id, "export.start", &export);
        }
        assert_eq!(connection.watched.len(), MAX_WATCHED);
        assert!(
            connection
                .watched
                .iter()
                .all(|(watched, ..)| *watched != job)
        );
        connection.close().unwrap();
    }
}
