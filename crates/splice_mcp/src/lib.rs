//! Splice tools over newline-delimited MCP stdio.
#![deny(missing_docs)]

use splice_json::Value as Json;

#[cfg(target_os = "macos")]
mod export;
#[cfg(any(target_os = "macos", test))]
mod export_file;
#[cfg(target_os = "macos")]
mod frames;
/// Persistent MCP friction records and their CLI commands.
pub mod friction;
mod history;
mod media;
mod platform;
mod project;
mod rpc;
mod timeline;
mod tools;
mod wire;

#[cfg(target_os = "macos")]
pub use export::export_project;
#[cfg(target_os = "macos")]
pub use frames::{grab_frame, write_png};
pub use history::{apply_batch, edit_result_json};
pub use rpc::serve;
pub use wire::error_payload;

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
