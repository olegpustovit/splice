//! Native capture package output.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

mod events;
pub use events::EventLog;
