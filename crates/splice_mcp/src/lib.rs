//! Splice tools over newline-delimited MCP stdio.
#![deny(missing_docs)]

use splice_json::Value as Json;

#[cfg(target_os = "macos")]
/// The installed application's private headless capture transport.
pub mod capture_agent;
#[cfg(target_os = "macos")]
mod export;
#[cfg(any(target_os = "macos", test))]
mod export_file;
#[cfg(target_os = "macos")]
mod frames;
/// Persistent MCP friction records and their CLI commands.
pub mod friction;
mod history;
#[cfg(target_os = "macos")]
mod ipc;
#[cfg(target_os = "macos")]
/// The private command channel between a project viewer and same-user clients.
pub mod live;
mod media;
/// Locations shared by the CLI, the viewer and the MCP server.
pub mod paths;
mod platform;
mod project;
#[cfg(target_os = "macos")]
mod record;
#[cfg(any(target_os = "macos", test))]
mod record_options;
#[cfg(any(target_os = "macos", test))]
mod record_publish;
mod rpc;
mod timeline;
mod tools;
mod wire;

#[cfg(target_os = "macos")]
pub use export::export_project;
#[cfg(target_os = "macos")]
pub use frames::{grab_frame, write_png};
pub use history::{apply_batch, checkpoint_edit, edit_result_json};
pub use rpc::serve;
pub use wire::error_payload;

/// Runs the same capture preflight as `record.start {dryRun:true}`.
pub fn record_preflight() -> Result<Json, splice_core::history::StoreError> {
    platform::record_start(
        &mut project::Projects::default(),
        &object(vec![("dryRun", Json::Bool(true))]),
    )
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
