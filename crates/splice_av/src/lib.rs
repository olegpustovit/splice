//! Native lowering of Splice's prepared playback plan.
#![cfg(target_os = "macos")]
#![deny(missing_docs)]

mod lower;
mod takes;

use apple_sys::composition;
use splice_eval::{EvalError, Evaluator};
use splice_model::{Frame, Project};
use std::{
    fmt,
    path::{Path, PathBuf},
};

pub use apple_sys::image::png as encode_png;
pub use composition::{Export, ExportState, Player};

/// A prepared timeline and its native composition for playback and delivery.
pub struct Composition {
    native: composition::Composition,
    evaluator: Evaluator,
    size: [u32; 2],
    end: Frame,
    text: Vec<TimedText>,
}

/// A resolved title or caption shared by the export and viewer.
pub struct TimedText {
    range: [Frame; 2],
    /// Resolved text, placement, color and opacity ramp.
    pub layer: composition::TextLayer,
}

impl TimedText {
    /// Samples the opacity ramp at a frame in the cue's interval.
    pub fn opacity(&self, at: Frame) -> f32 {
        let fraction = (at - self.range[0]) as f32 / (self.range[1] - self.range[0]) as f32;
        let [a, b] = self.layer.opacity;
        a + (b - a) * fraction
    }
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
    /// A source file the timeline uses is missing.
    Offline(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(error) => error.fmt(f),
            Self::Input(message) => f.write_str(message),
            Self::Native(error) => error.fmt(f),
            Self::Offline(path) => write!(f, "Media is offline: {}", path.display()),
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
        let (native, text) = lower::build(project, package, &evaluator, &cuts, end)?;
        Ok(Self {
            native,
            evaluator,
            size: project.size,
            end,
            text,
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

    /// Title and caption cards visible at this timeline frame, in paint order.
    pub fn text_at(&self, at: Frame) -> &[TimedText] {
        let first = self.text.partition_point(|cue| cue.range[1] <= at);
        let rest = &self.text[first..];
        &rest[..rest.partition_point(|cue| cue.range[0] <= at)]
    }

    /// Creates a paused playback output for this revision.
    pub fn player(&self) -> Result<Player, Error> {
        self.native.player().map_err(Into::into)
    }

    /// Reads one offscreen program frame into a caller-owned RGBA8 buffer.
    pub fn read_frame(&self, at: Frame, rgba: &mut [u8]) -> Result<(), Error> {
        self.native.read_frame(at, rgba).map_err(Into::into)
    }

    /// Reads the program picture so the viewer can draw text in its own layout.
    pub fn read_picture(&self, at: Frame, rgba: &mut [u8]) -> Result<(), Error> {
        self.native.read_picture(at, rgba).map_err(Into::into)
    }

    /// Starts encoding the prepared composition and audio mix into a new MP4.
    pub fn start_export(&self, file: &Path, hevc: bool) -> Result<Export, Error> {
        self.native.start_export(file, hevc).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests;
