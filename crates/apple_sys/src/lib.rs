//! Safe Rust interfaces to Splice's native Apple services.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
#![deny(missing_docs)]

mod error;

pub use error::Error;

/// AVFoundation composition, video instructions and audio mix.
pub mod composition;

/// The bounded native event queue.
pub mod events;

/// Encoding for rendered RGBA frames.
pub mod image;

/// AVFoundation media metadata.
pub mod media;

#[cfg(test)]
mod tests;
