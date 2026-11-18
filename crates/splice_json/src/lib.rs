//! JSON tokenizer and canonical writer for project files, journal records and MCP messages.
#![deny(missing_docs)]

mod read;
mod tokens;
mod write;

use std::fmt;

pub use read::{Value, parse};
pub use tokens::{Kind, Token, next};
pub use write::{NonFiniteNumber, write};

const MAX_DEPTH: usize = 128;

/// An invalid JSON document or token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// Byte position in the original input, with EOF at the input length.
    pub offset: usize,
    /// Path to the enclosing value, starting at `$`.
    pub path: String,
    /// Explanation of what failed.
    pub message: &'static str,
}

impl Error {
    fn new(offset: usize, path: &str, message: &'static str) -> Self {
        Self {
            offset,
            path: path.into(),
            message,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {} ({})",
            self.message, self.offset, self.path
        )
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests;
