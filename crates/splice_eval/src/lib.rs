//! Prepare a timeline once per revision, then evaluate frames without allocating.
#![deny(missing_docs)]

use splice_model::{ClipId, Frame, ItemId, MediaId, Rect, ShapeKind, TimelineId, WordId};

pub mod activity;
mod camera;
mod compound;
mod cube;
pub mod cursor;
mod eval;
mod layout;
mod look;
mod style;
mod text;

pub use camera::View;
pub use cube::{cube_atlas, parse_cube};
pub use eval::{EvalError, Evaluator, SourceSpan, caption_tokens};

/// The word ID of a free-text caption cue's single token: an unbound legacy `w0`, never a loaded word.
pub const FREE_TEXT: WordId = WordId::legacy(0);
pub use look::{
    AudioFx, Blend, ChromaKey, Cutout, Grade, Look, MAX_MASKS, Mask, MaskMode, MaskShape,
    Transition, TransitionKind,
};
pub use style::{
    Animate, Background, CaptionPreset, CaptionStyle, Cursor, KeyPosition, Keys, TextStyle, Window,
    resolve_effect,
};
pub use text::{grapheme_len, text_units};

#[cfg(test)]
mod look_tests;
#[cfg(test)]
mod motion_tests;
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
    /// Layers composited as one flattened image: the incoming side of a transition or a fading
    /// clip (D14). Opacity applied layer by layer would compound (a window over its backdrop
    /// fades as t² and bleeds the backdrop through), so renderers flatten the group first.
    pub group: Option<Group>,
    /// A rigid move of a transition's outgoing side. It is opaque, so moving each layer is the
    /// same as moving the flattened image.
    pub shift: Option<Shift>,
}

/// A run of `layers` drawn into a transparent image, then composited once with `opacity`,
/// `scale` about the canvas center, `offset` and an optional `clip` rectangle (canvas pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    /// Indices into `FramePlan::layers`, half-open.
    pub layers: [usize; 2],
    /// Opacity of the flattened image.
    pub opacity: f64,
    /// Scale about the canvas center.
    pub scale: f64,
    /// Offset in canvas pixels.
    pub offset: [f64; 2],
    /// Visible region in canvas pixels (wipes).
    pub clip: Option<[f64; 4]>,
}

/// Scale about the canvas center and offset for a run of `layers`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shift {
    /// Indices into `FramePlan::layers`, half-open.
    pub layers: [usize; 2],
    /// Scale about the canvas center.
    pub scale: f64,
    /// Offset in canvas pixels.
    pub offset: [f64; 2],
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
        /// Background behind the presenter: 0 keep, 1 blur, 2 remove.
        background: u8,
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
    /// The primary clip's fade through black, or a transition's dip through a color.
    Fade {
        /// Color opacity.
        opacity: f64,
        /// RGB color (black for fades).
        color: u32,
    },
    /// Generated shape or fill (T17.5).
    Shape {
        /// Owning item.
        item: ItemId,
        /// What to draw.
        shape: ShapeKind,
        /// Bounds in canvas pixels (arrows and lines run corner to corner).
        bounds: Rect,
        /// Fill color.
        fill: u32,
        /// Second fill color for gradients.
        fill2: u32,
        /// Stroke color.
        stroke: u32,
        /// Stroke width in canvas pixels.
        stroke_width: f64,
        /// Corner radius in canvas pixels.
        corner: f64,
        /// Fade opacity.
        opacity: f64,
        /// Keyed transform.
        look: Look,
    },
    /// Title overlay, with text available from `Evaluator::title`.
    Title {
        /// Owning item.
        item: ItemId,
        /// Title card bounds in canvas pixels: the text wraps and shrinks to fit inside.
        bounds: Rect,
        /// Starting font size in canvas pixels (the rasterizer shrinks it until the text fits).
        font: f64,
        /// Whether lines wrap at the box width (otherwise the font shrinks to fit each line).
        wrap: bool,
        /// Fade opacity.
        opacity: f64,
        /// Keyed transform.
        look: Look,
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
        /// Keyed transform.
        look: Look,
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
    /// The timeline camera on spine pictures (identity on overlays).
    pub view: View,
    /// Fade opacity.
    pub opacity: f64,
    /// Resolved Window treatment. Its background is a separate layer.
    pub window: Option<Window>,
    /// Cursor style applied to the source before crop.
    pub cursor: Option<Cursor>,
    /// Keyed transform, color, masks, key and transition treatment.
    pub look: Look,
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
    /// Stereo pan −1 (left) … 1 (right).
    pub pan: f64,
}

/// Resolved text and timeline range for a caption word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptionWord {
    /// Stable transcript word ID, or [`FREE_TEXT`] for a free-text cue's one token.
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
