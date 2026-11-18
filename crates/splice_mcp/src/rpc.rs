use super::{Json, object, project::Projects, text, tools};
use std::io::{self, BufRead, Read, Write};

const LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";
const PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";
const MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

/// Serves one MCP connection, flushing each reply and returning on end of input.
pub fn serve(input: &mut dyn BufRead, output: &mut dyn Write) -> io::Result<()> {
    let mut initialized = false;
    let mut projects = Projects::default();
    let mut line = Vec::new();
    let mut wire = String::new();
    loop {
        line.clear();
        (&mut *input)
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if line.is_empty() {
            return projects.close();
        }
        let response = if line.len() as u64 > MAX_LINE_BYTES {
            if !line.ends_with(b"\n") {
                input.skip_until(b'\n')?;
            }
            Some(rpc_error(&Json::Null, -32700, "Message exceeds 16 MiB"))
        } else {
            handle_line(&line, &mut initialized, &mut projects)
        };
        if let Some(response) = response {
            splice_json::write(&response, &mut wire).map_err(io::Error::other)?;
            wire.push('\n');
            output.write_all(wire.as_bytes())?;
            output.flush()?;
        }
    }
}

fn handle_line(line: &[u8], initialized: &mut bool, projects: &mut Projects) -> Option<Json> {
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
    Some(dispatch(id, method, params, initialized, projects))
}

static EMPTY_OBJECT: Json = Json::Object(Vec::new());

fn dispatch(
    id: &Json,
    method: &str,
    params: &Json,
    initialized: &mut bool,
    projects: &mut Projects,
) -> Json {
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
            if *initialized {
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
            *initialized = true;
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
        "tools/list" | "tools/call" if !modern && !*initialized => {
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
            match tools::call(projects, name, args) {
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
