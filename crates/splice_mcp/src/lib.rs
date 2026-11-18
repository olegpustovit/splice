//! Splice tools over newline-delimited MCP stdio.
#![deny(missing_docs)]

use splice_json::Value as Json;

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

pub use history::edit_result_json;
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
