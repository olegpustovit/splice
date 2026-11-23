//! Safe Rust interfaces to Splice's native Apple services.
#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
// `*mut c_void` parameters here are opaque `id<MTL…>`/CoreFoundation handles passed through to
// native code, which validates them; Rust never dereferences them.
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![deny(missing_docs)]

mod error;

pub use error::Error;

/// AppKit integration for the viewer.
pub mod app;

/// Offline audio rendering, envelopes and sound classification.
pub mod audio;

/// ScreenCaptureKit source discovery and capture targets.
pub mod capture;

/// AVFoundation composition, video instructions and audio mix.
pub mod composition;

/// The bounded native event queue.
pub mod events;

/// HTML pages rendered frame by frame into alpha movies (T45.1).
pub mod html;

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

/// On-device transcription (SpeechAnalyzer, through a Swift shim).
pub mod speech;

#[cfg(test)]
mod tests;

/// Vision analysis, tracking, person mattes and on-device translation.
pub mod vision;
