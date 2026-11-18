//! Native lowering of Splice's prepared playback plan.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

mod lower;

use apple_sys::composition;
use splice_eval::{EvalError, Evaluator};
use splice_model::{Frame, Project};
use std::{fmt, path::Path};

pub use composition::{Export, ExportState};

/// A prepared timeline and its native composition for playback and delivery.
pub struct Composition {
    native: composition::Composition,
    evaluator: Evaluator,
    size: [u32; 2],
    end: Frame,
}

/// Why the program could not be prepared or rendered.
#[derive(Debug)]
pub enum Error {
    /// The evaluator could not prepare the project.
    Plan(EvalError),
    /// A rendered layer refers to a missing input.
    Input(String),
    /// A native service rejected the composition or requested frame.
    Native(apple_sys::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(f),
            Self::Input(message) => f.write_str(message),
            Self::Native(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<apple_sys::Error> for Error {
    fn from(error: apple_sys::Error) -> Self {
        Self::Native(error)
    }
}

impl Composition {
    /// Builds the program from a read-only project revision and its package directory.
    pub fn prepare(project: &Project, package: &Path) -> Result<Self, Error> {
        let evaluator = Evaluator::prepare(project).map_err(Error::Plan)?;
        let cuts = evaluator.breakpoints();
        let end = *cuts.last().unwrap_or(&0);
        let native = lower::build(project, package, &evaluator, &cuts, end)?;
        Ok(Self {
            native,
            evaluator,
            size: project.size,
            end,
        })
    }

    /// Output dimensions in canvas pixels.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// Exclusive end of the composition in timeline frames.
    pub fn end(&self) -> Frame {
        self.end
    }

    /// Playback evaluator for overlays drawn by the viewer.
    pub fn evaluator(&self) -> &Evaluator {
        &self.evaluator
    }

    /// Reads one offscreen program frame into a caller-owned RGBA8 buffer.
    pub fn read_frame(&self, at: Frame, rgba: &mut [u8]) -> Result<(), Error> {
        self.native.read_frame(at, rgba).map_err(Into::into)
    }

    /// Starts encoding the prepared composition and audio mix into a new MP4.
    pub fn start_export(&self, file: &Path, hevc: bool) -> Result<Export, Error> {
        self.native.start_export(file, hevc).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
