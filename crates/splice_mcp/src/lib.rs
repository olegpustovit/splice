//! Splice tools over newline-delimited MCP stdio.
#![deny(missing_docs)]

use splice_json::Value as Json;

mod rpc;
mod tools;

pub use rpc::serve;

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
