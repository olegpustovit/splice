//! Safe Rust interfaces to Splice's native Apple services.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
#![deny(missing_docs)]

mod error;

pub use error::Error;

/// The bounded native event queue.
pub mod events;

/// AVFoundation media metadata.
pub mod media;

#[cfg(test)]
mod tests;
