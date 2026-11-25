//! Splice tools over newline-delimited MCP stdio.
#![deny(missing_docs)]

use splice_json::Value as Json;

mod analyze;
#[cfg(target_os = "macos")]
mod background;
#[cfg(target_os = "macos")]
/// The installed application's private headless capture transport.
pub mod capture_agent;
mod check;
mod code;
mod enrich;
#[cfg(target_os = "macos")]
mod export;
#[cfg(any(target_os = "macos", test))]
mod export_file;
mod fcpxml;
#[cfg(target_os = "macos")]
mod frames;
/// Persistent MCP friction records and their CLI commands.
pub mod friction;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod gif;
mod history;
#[cfg(target_os = "macos")]
mod ipc;
mod lints;
#[cfg(target_os = "macos")]
/// The private command channel between a project viewer and same-user clients.
pub mod live;
mod media;
mod music;
#[cfg(target_os = "macos")]
mod native;
/// Locations shared by the CLI, the viewer and the MCP server.
pub mod paths;
mod platform;
mod presets;
mod project;
#[cfg(target_os = "macos")]
mod record;
#[cfg(any(target_os = "macos", test))]
mod record_cli;
#[cfg(any(target_os = "macos", test))]
mod record_options;
#[cfg(any(target_os = "macos", test))]
mod record_publish;
#[cfg(any(target_os = "macos", test))]
mod record_windows;
mod rpc;
mod sheet;
mod timeline;
mod tools;
mod typescript;
mod vector;
mod wire;

#[cfg(target_os = "macos")]
pub use export::export_project;
#[cfg(target_os = "macos")]
pub use frames::{grab_frame, offline_fields, write_png};
pub use history::{apply_batch, checkpoint_edit, edit_result_json};
pub use rpc::{VERSION, serve};
pub use tools::Tool;

/// Tools exposed by the default MCP profile, in discovery order.
pub fn tool_catalog() -> &'static [Tool] {
    tools::TOOLS
}

pub use wire::error_payload;

/// Runs the same capture preflight as `record.start {dryRun:true}`.
pub fn record_preflight() -> Result<Json, splice_core::history::StoreError> {
    platform::record_start(
        &mut project::Projects::default(),
        &object(vec![("dryRun", Json::Bool(true))]),
    )
}

/// Runs the public recording jobs for the CLI and returns their JSON receipt or error payload.
/// For a manual recording, `stop` waits for the user and returns false to discard the capture.
pub fn record_project(
    args: &Json,
    stop: &mut dyn FnMut() -> std::io::Result<bool>,
) -> Result<Json, Json> {
    #[cfg(target_os = "macos")]
    {
        rpc::record_project(args, stop)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = stop;
        platform::record_start(&mut project::Projects::default(), args).map_err(error_payload)
    }
}

/// Reports capture authorization or explicitly requests one missing macOS grant.
pub fn capture_permissions(
    request: bool,
    permission: Option<&str>,
) -> Result<Json, splice_core::history::StoreError> {
    #[cfg(target_os = "macos")]
    {
        capture_agent::permission_report(request, permission)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (request, permission);
        Err(platform::unavailable("Capture permissions"))
    }
}

#[cfg(test)]
mod tests;

fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

fn text(value: &str) -> Json {
    Json::String(value.into())
}
