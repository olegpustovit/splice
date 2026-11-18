use crate::{Frame, MediaId, WordId};

/// File descriptor and probe results. The model never reads media bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Media {
    /// Stable media ID.
    pub id: MediaId,
    /// Import or recording type.
    pub kind: MediaKind,
    /// Stored path, relative to the package when possible.
    pub path: String,
    /// Base64-encoded security-scoped bookmark when available.
    pub bookmark: Option<String>,
    /// File size in bytes.
    pub size: u64,
    /// Modification time in Unix seconds.
    pub mtime: i64,
    /// Probe data obtained by the media backend.
    pub probe: Probe,
    /// Component paths relative to a recorded take.
    pub take: Option<TakeComponents>,
}

/// Media descriptor categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    /// Imported video.
    Video,
    /// Imported audio.
    Audio,
    /// Imported or generated still image.
    Image,
    /// Multi-component recording.
    Take,
}

/// Properties needed for editing without opening a media file.
#[derive(Clone, Debug, PartialEq)]
pub struct Probe {
    /// Source frame rate.
    pub fps: f64,
    /// Duration in source frames.
    pub frames: Frame,
    /// Video width in pixels.
    pub width: u32,
    /// Video height in pixels.
    pub height: u32,
    /// Named audio tracks in probe order.
    pub audio: Vec<AudioTrack>,
    /// Codec label, if known.
    pub codec: Option<String>,
}

/// A named audio track and its channel count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Track role or name.
    pub name: String,
    /// Number of channels when known.
    pub channels: Option<u32>,
}

/// Individual files present in a captured take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TakeComponents {
    /// Screen video file.
    pub screen: Option<String>,
    /// Camera video file.
    pub camera: Option<String>,
    /// Microphone audio file.
    pub mic: Option<String>,
    /// System audio file.
    pub system: Option<String>,
    /// Cursor and key event file.
    pub events: Option<String>,
    /// Cursor image directory.
    pub cursors: Option<String>,
}

/// Versioned words on the media source clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    /// Media item described by the transcript.
    pub media: MediaId,
    /// Signature of this transcript version.
    pub hash: u64,
    /// Language tag.
    pub language: String,
    /// Words in source order.
    pub words: Vec<Word>,
}

/// A timed word in a transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    /// Stable ID within the transcript.
    pub id: WordId,
    /// Visible text.
    pub text: String,
    /// Inclusive start in source frames.
    pub from: Frame,
    /// Exclusive end in source frames.
    pub to: Frame,
    /// Recognition confidence when available.
    pub confidence: Option<f64>,
}
