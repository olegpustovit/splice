//! Prepare a timeline once per revision, then evaluate frames without allocating.
#![deny(missing_docs)]

use splice_model::{ClipId, Frame, ItemId, MediaId, Rect, TimelineId, WordId};

mod eval;
mod layout;
mod style;

pub use eval::{EvalError, Evaluator};
pub use style::{
    Background, CaptionPreset, CaptionStyle, ClickZoom, Cursor, KeyPosition, Keys, Window,
};

#[cfg(test)]
mod tests;

/// Reusable output for one frame of video and audio.
#[derive(Debug, Default)]
pub struct FramePlan {
    /// Timeline frame being evaluated.
    pub frame: Frame,
    /// Canvas size in pixels.
    pub size: [u32; 2],
    /// Clip under the playhead, including gap clips.
    pub clip: Option<ClipId>,
    /// Layers from back to front, over an implicit black canvas.
    pub layers: Vec<Layer>,
    /// Audible tracks with their source positions and gain.
    pub audio: Vec<Audio>,
}

/// A visible operation in the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
    /// A backing behind a Window.
    Background {
        /// Pixel bounds of the placement.
        bounds: Rect,
        /// Fill treatment.
        fill: Background,
        /// Opacity from the owning item.
        opacity: f64,
    },
    /// Source picture after its crop, fit and Window layout.
    Picture(Picture),
    /// Camera component from a take.
    Camera {
        /// Owning clip.
        clip: ClipId,
        /// Take media descriptor.
        media: MediaId,
        /// Position on the source clock.
        source_frame: f64,
        /// Bubble bounds in canvas pixels.
        bounds: Rect,
        /// Radius in canvas pixels.
        radius: f64,
        /// Shadow intensity.
        shadow: f64,
    },
    /// Recorded keystrokes on the current source clock.
    Keystrokes {
        /// Owning clip for the hidden event indices in `Evaluator::hidden_keys`.
        clip: ClipId,
        /// Take media descriptor.
        media: MediaId,
        /// Position on the source clock.
        source_frame: f64,
        /// Chip style.
        style: Keys,
    },
    /// The primary clip's fade through black.
    Fade {
        /// Black opacity.
        opacity: f64,
    },
    /// Title overlay, with text available from `Evaluator::title`.
    Title {
        /// Owning item.
        item: ItemId,
        /// Title card bounds in canvas pixels.
        bounds: Rect,
        /// Fade opacity.
        opacity: f64,
    },
    /// Caption cue with timed text available from `Evaluator::caption_words`.
    Caption {
        /// Owning item.
        item: ItemId,
        /// Index of the latest word that has begun, including during pauses.
        word: Option<usize>,
        /// Fade opacity.
        opacity: f64,
        /// Resolved style and safe-zone placement.
        style: CaptionStyle,
    },
}

/// Picture geometry and source position for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picture {
    /// Owning clip or overlay.
    pub item: TimelineId,
    /// Image or video media descriptor.
    pub media: MediaId,
    /// Position on the source clock.
    pub source_frame: f64,
    /// Source rectangle normalized to the decoded frame.
    pub source: Rect,
    /// Destination rectangle in pixels before zoom.
    pub destination: Rect,
    /// Scale around the canvas center from the active Motion item.
    pub scale: f64,
    /// Fade opacity.
    pub opacity: f64,
    /// Resolved Window treatment. Its background is a separate layer.
    pub window: Option<Window>,
    /// Cursor style applied to the source before crop.
    pub cursor: Option<Cursor>,
    /// Settings for zoom driven by recorded clicks.
    pub click_zoom: Option<ClickZoom>,
}

/// Audio track identity within a descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Track {
    /// Indexed audio track within an imported file.
    Imported(u32),
    /// Recorded microphone track.
    Mic,
    /// Recorded system audio track.
    System,
}

/// Audio gain and source position for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Audio {
    /// Owning clip or overlay.
    pub item: TimelineId,
    /// Media descriptor.
    pub media: MediaId,
    /// Source audio track.
    pub track: Track,
    /// Position in frames on the source clock.
    pub source_frame: f64,
    /// Linear gain after fading.
    pub gain: f64,
}

/// Resolved text and timeline range for a caption word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionWord {
    /// Stable transcript word ID.
    pub id: WordId,
    /// Styled display text without the optional emoji.
    pub text: String,
    /// Optional display emoji.
    pub emoji: Option<String>,
    /// Whether the cue emphasizes the word.
    pub emph: bool,
    /// Inclusive timeline start.
    pub start: Frame,
    /// Exclusive timeline end.
    pub end: Frame,
}
