//! Safe Rust interfaces to Splice's native Apple services.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
#![deny(missing_docs)]

mod error;

pub use error::Error;

/// AppKit integration for the viewer.
pub mod app;

/// ScreenCaptureKit source discovery and capture targets.
pub mod capture;

/// AVFoundation composition, video instructions and audio mix.
pub mod composition;

/// The bounded native event queue.
pub mod events;

/// Encoding for rendered RGBA frames.
pub mod image;

/// AVFoundation media metadata.
pub mod media;

/// Peer credentials for local macOS sockets.
pub mod ipc;

/// macOS capture authorization and privacy settings.
pub mod permissions;

/// Still capture with ScreenCaptureKit.
pub mod screenshot;

#[cfg(test)]
mod tests;
