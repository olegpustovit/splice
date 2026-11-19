use super::{Json, friction::Recorder, object, paths, project::Projects, text, tools};
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

const LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";
const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

struct Connection {
    initialized: bool,
    projects: Projects,
    friction: Recorder,
    #[cfg(target_os = "macos")]
    routes: super::live::proxy::Routes,
    /// Handoff sockets for the packages this headless session holds (T10.1).
    #[cfg(target_os = "macos")]
    holders: Vec<super::live::Server>,
    #[cfg(target_os = "macos")]
    events: Option<Sender<Event>>,
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
        projects: Projects::default(),
        friction: Recorder::new(paths::state_dir().ok()),
        routes: Default::default(),
        holders: Vec::new(),
        events: None,
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
pub fn serve(input: &mut (dyn BufRead + Send), output: &mut dyn Write) -> io::Result<()> {
    let global = match paths::state_dir() {
        Ok(dir) => Some(dir),
        Err(error) => {
            eprintln!("splice friction: {error}");
            None
        }
    };
    serve_with_log(input, output, global)
}

pub(super) fn serve_with_log(
    input: &mut (dyn BufRead + Send),
    output: &mut dyn Write,
    global: Option<PathBuf>,
) -> io::Result<()> {
    let (events, incoming) = mpsc::channel();
    let mut connection = Connection {
        initialized: false,
        projects: Projects::default(),
        friction: Recorder::new(global),
        #[cfg(target_os = "macos")]
        routes: Default::default(),
        #[cfg(target_os = "macos")]
        holders: Vec::new(),
        #[cfg(target_os = "macos")]
        events: Some(events.clone()),
    };
    // Stdin is read on its own thread so a viewer's handoff request is served while idle.
    thread::scope(|scope| {
        scope.spawn(move || read_lines(input, &events));
        let mut wire = String::new();
        loop {
            let line = match incoming.recv() {
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
    // Notification messages have no reply and never invoke a request handler.
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
            object(vec![("tools", tools::list())])
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
            if !tools::known(name) {
                return rpc_error(id, -32602, &format!("Unknown tool: {name}"));
            }
            match connection.call(name, args, modern.then_some(meta)) {
                Ok(result) => result,
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
    fn call(
        &mut self,
        name: &str,
        args: &Json,
        meta: Option<&Json>,
    ) -> Result<Json, splice_json::NonFiniteNumber> {
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

fn capabilities() -> Json {
    object(vec![(
        "tools",
        object(vec![("listChanged", Json::Bool(false))]),
    )])
}

fn server_info() -> Json {
    object(vec![
        ("name", text("splice")),
        ("version", text(env!("CARGO_PKG_VERSION"))),
    ])
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
