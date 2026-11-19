use crate::{Frame, MediaId, WordId};

/// File descriptor and probe results. The model never reads media bytes.
#[derive(Debug, PartialEq)]
pub struct Media {
    /// Stable media ID.
    pub id: MediaId,
    /// Import or recording type.
    pub kind: MediaKind,
    /// Stored path, relative to the package when possible.
    pub path: String,
    /// Base64-encoded macOS file bookmark when available.
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
#[derive(Debug, PartialEq)]
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
    /// Integrated loudness (ITU-R BS.1770, LUFS) of the audio, when measured.
    pub loudness: Option<f64>,
}

/// A named audio track and its channel count.
#[derive(Debug, PartialEq, Eq)]
pub struct AudioTrack {
    /// Track role or name.
    pub name: String,
    /// Number of channels when known.
    pub channels: Option<u32>,
}

/// Individual files present in a captured take.
#[derive(Debug, Default, PartialEq, Eq)]
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
#[derive(Debug, PartialEq)]
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
#[derive(Debug, PartialEq)]
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

impl Clone for Media {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            kind: self.kind,
            path: self.path.clone(),
            bookmark: self.bookmark.clone(),
            size: self.size,
            mtime: self.mtime,
            probe: self.probe.clone(),
            take: self.take.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.kind = source.kind;
        self.path.clone_from(&source.path);
        self.bookmark.clone_from(&source.bookmark);
        self.size = source.size;
        self.mtime = source.mtime;
        self.probe.clone_from(&source.probe);
        self.take.clone_from(&source.take);
    }
}

impl Clone for Probe {
    fn clone(&self) -> Self {
        Self {
            fps: self.fps,
            frames: self.frames,
            width: self.width,
            height: self.height,
            audio: self.audio.clone(),
            codec: self.codec.clone(),
            loudness: self.loudness,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.fps = source.fps;
        self.frames = source.frames;
        self.width = source.width;
        self.height = source.height;
        self.audio.clone_from(&source.audio);
        self.codec.clone_from(&source.codec);
        self.loudness = source.loudness;
    }
}

impl Clone for AudioTrack {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            channels: self.channels,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.name.clone_from(&source.name);
        self.channels = source.channels;
    }
}

impl Clone for TakeComponents {
    fn clone(&self) -> Self {
        Self {
            screen: self.screen.clone(),
            camera: self.camera.clone(),
            mic: self.mic.clone(),
            system: self.system.clone(),
            events: self.events.clone(),
            cursors: self.cursors.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.screen.clone_from(&source.screen);
        self.camera.clone_from(&source.camera);
        self.mic.clone_from(&source.mic);
        self.system.clone_from(&source.system);
        self.events.clone_from(&source.events);
        self.cursors.clone_from(&source.cursors);
    }
}

impl Clone for Transcript {
    fn clone(&self) -> Self {
        Self {
            media: self.media,
            hash: self.hash,
            language: self.language.clone(),
            words: self.words.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.media = source.media;
        self.hash = source.hash;
        self.language.clone_from(&source.language);
        self.words.clone_from(&source.words);
    }
}

impl Clone for Word {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            text: self.text.clone(),
            from: self.from,
            to: self.to,
            confidence: self.confidence,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.text.clone_from(&source.text);
        self.from = source.from;
        self.to = source.to;
        self.confidence = source.confidence;
    }
}
