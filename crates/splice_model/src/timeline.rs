use crate::{ClipId, EffectId, Frame, ItemId, Key, LaneId, MarkerId, MediaId, TimelineId, WordId};

/// Supported canvas shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aspect {
    /// 9:16.
    Portrait,
    /// 4:5.
    Social,
    /// 1:1.
    Square,
    /// 4:3.
    Standard,
    /// 16:9.
    Landscape,
}

/// Agent permissions stored in the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Agents have no access.
    Off,
    /// Agents can inspect the project.
    Read,
    /// Agents can edit the project.
    Live,
}

/// The primary lane.
#[derive(Debug, PartialEq)]
pub struct Spine {
    /// Stable lane ID.
    pub id: LaneId,
    /// Display name.
    pub name: String,
    /// Effects inherited by primary clips.
    pub effects: Vec<Effect>,
}

/// A secondary lane.
#[derive(Debug, PartialEq)]
pub struct Lane {
    /// Stable lane ID.
    pub id: LaneId,
    /// Display name.
    pub name: String,
    /// Which kind of item the lane holds.
    pub kind: LaneKind,
    /// Effects inherited by items in the lane.
    pub effects: Vec<Effect>,
}

/// Secondary lane categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneKind {
    /// Titles and graphics.
    Graphics,
    /// Media overlays.
    Overlay,
    /// Camera zoom segments (D12).
    Camera,
    /// Caption cues.
    Captions,
    /// Music and sound effects.
    Audio,
}

/// Full spine clip record used when adding a clip to storage.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Stable clip ID.
    pub id: ClipId,
    /// Timeline and source coordinates.
    pub timing: ClipTiming,
    /// Media and appearance.
    pub data: ClipData,
}

/// Spine clip timing. Timeline intervals are half-open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipTiming {
    /// First timeline frame.
    pub start: Frame,
    /// Duration in timeline frames.
    pub length: Frame,
    /// First media frame.
    pub source: Frame,
    /// Playback speed multiplier.
    pub speed: f64,
}

impl ClipTiming {
    /// First frame after the clip, or `None` on overflow.
    pub fn end(self) -> Option<Frame> {
        self.start.checked_add(self.length)
    }

    /// Whether the timeline frame lies inside the clip.
    pub fn contains(self, frame: Frame) -> bool {
        frame >= self.start && self.end().is_some_and(|end| frame < end)
    }
}

/// Spine clip settings.
#[derive(Debug, Default, PartialEq)]
pub struct ClipData {
    /// Source media, or `None` for a gap.
    pub media: Option<MediaId>,
    /// Fades, name, color and effects.
    pub appearance: Appearance,
    /// Video framing.
    pub framing: Framing,
    /// Enabled state for each recorded component. Absent components have no state.
    pub enabled: ComponentStates,
    /// Microphone and system audio gains.
    pub volume: AudioGains,
    /// Keystroke event indices hidden in this clip.
    pub hidden_keys: Vec<u32>,
    /// Offsets from the clip start where clip.join merged clips (clip.ungroup splits there).
    pub joins: Vec<Frame>,
}

/// Independently switchable media components.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentStates {
    /// Screen video.
    pub screen: Option<bool>,
    /// Camera video.
    pub camera: Option<bool>,
    /// Microphone audio.
    pub mic: Option<bool>,
    /// System audio.
    pub system: Option<bool>,
    /// Recorded cursor.
    pub pointer: Option<bool>,
    /// Keystroke chips.
    pub keys: Option<bool>,
}

/// Source audio gain multipliers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioGains {
    /// Microphone gain.
    pub mic: f64,
    /// System audio gain.
    pub system: f64,
}

impl Default for AudioGains {
    fn default() -> Self {
        Self {
            mic: 1.0,
            system: 0.8,
        }
    }
}

/// Shared clip and item settings.
#[derive(Debug, Default, PartialEq)]
pub struct Appearance {
    /// Fade-in duration.
    pub fade_in: Frame,
    /// Fade-out duration.
    pub fade_out: Frame,
    /// Custom name.
    pub name: Option<String>,
    /// Timeline swatch.
    pub color: Option<String>,
    /// Local effects in application order.
    pub effects: Vec<Effect>,
    /// Paint order of a visual item (T42.4): higher draws in front, across every kind; equal
    /// z keeps the default order (pictures, shapes, titles, captions; then lane stack).
    /// Spine clips ignore it.
    pub z: i32,
}

/// Video crop and fit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Framing {
    /// Normalized crop in source coordinates, if cropped.
    pub crop: Option<Rect>,
    /// How to fit the source to its destination.
    pub fit: Fit,
}

/// Normalized rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
}

/// Scaling mode for media.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// Contain the source in the destination.
    #[default]
    Fit,
    /// Cover the destination with the source.
    Fill,
}

/// Full item record used when adding an item to storage.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Stable item ID.
    pub id: ItemId,
    /// Anchor and duration.
    pub timing: ItemTiming,
    /// Lane, kind and appearance.
    pub data: ItemData,
}

/// Overlay timing. Anchored starts are derived from the project when read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemTiming {
    /// Where the item starts.
    pub anchor: Anchor,
    /// Duration in timeline frames.
    pub length: Frame,
}

/// An exact frame or a reference whose timeline position can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Fixed timeline frame.
    Pinned(Frame),
    /// Fixed source frame within a particular clip.
    ClipSource {
        /// Source clip.
        clip: ClipId,
        /// Frame on the clip's media clock.
        source: Frame,
    },
    /// Offset from an item edge.
    Item {
        /// Timeline object.
        item: TimelineId,
        /// Start or end edge.
        edge: Edge,
        /// Signed timeline frame offset.
        offset: Frame,
    },
    /// Offset from a marker's position.
    Marker {
        /// Marker to follow.
        marker: MarkerId,
        /// Signed timeline frame offset.
        offset: Frame,
    },
    /// Offset from a word in a given transcript version.
    Word {
        /// Word in the transcript.
        word: WordId,
        /// Expected transcript hash.
        transcript_hash: u64,
        /// Start or end of the word.
        edge: Edge,
        /// Signed timeline frame offset.
        offset: Frame,
    },
}

/// Boundary of a timeline interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// First frame in the interval.
    Start,
    /// First frame after the interval.
    End,
}

/// Item settings.
#[derive(Debug, PartialEq)]
pub struct ItemData {
    /// Lane containing the item.
    pub lane: LaneId,
    /// Kind-specific content.
    pub kind: ItemKind,
    /// Fades, name, color and effects.
    pub appearance: Appearance,
}

/// Content of an overlay item.
#[derive(Debug, PartialEq)]
pub enum ItemKind {
    /// Title text.
    Title {
        /// Visible string.
        text: String,
    },
    /// Video or still overlay.
    Media(Overlay),
    /// A camera segment: while it lasts the timeline camera springs toward its framing (D12).
    Zoom(Zoom),
    /// Music or a sound effect on an audio lane.
    Sound(Sound),
    /// Generated graphic: solid, gradient, shape, arrow, line or callout (T17.5).
    Shape {
        /// What to draw.
        shape: ShapeKind,
        /// Bounds in normalized canvas coordinates. Arrows and lines run from the top-left
        /// corner to the bottom-right one.
        rect: Rect,
    },
    /// Cue composed from words in a transcript, or free text (T41.5).
    Caption {
        /// Words in cue order; empty for a free-text cue.
        words: Vec<WordId>,
        /// Display edits for individual words.
        edits: Vec<CaptionEdit>,
        /// A free-text cue's text: it needs no transcript and keeps the item's own timing.
        text: Option<String>,
    },
}

impl ItemKind {
    /// Lane kind compatible with the item.
    pub fn lane_kind(&self) -> LaneKind {
        match self {
            Self::Title { .. } => LaneKind::Graphics,
            Self::Media(_) => LaneKind::Overlay,
            Self::Zoom(_) => LaneKind::Camera,
            Self::Caption { .. } => LaneKind::Captions,
            Self::Sound(_) => LaneKind::Audio,
            Self::Shape { .. } => LaneKind::Graphics,
        }
    }
}

/// One camera segment (PLAN.md D12, D13). The camera springs toward `amount` and `center` while
/// the segment lasts; outside every segment it springs back to 1× with the last center held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    /// Magnification, at least 1.
    pub amount: f64,
    /// The content point (normalized 0…1) to frame; the camera centers on it as far as the
    /// viewport can stay inside the content. `None` keeps the framing the camera already has.
    pub focus: Option<[f64; 2]>,
    /// What zooms: the content inside a fixed frame, or the whole framed card.
    pub mode: CameraMode,
    /// How the camera moves into the segment.
    pub spring: Spring,
}

impl Zoom {
    /// Magnification range a segment accepts.
    pub const AMOUNT: [f64; 2] = [1.0, 5.0];
}

/// What a camera segment zooms (D13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraMode {
    /// Content on portrait canvases, frame otherwise.
    #[default]
    Auto,
    /// The frame stays put; the content zooms inside it.
    Content,
    /// The framed card zooms and may overflow the canvas.
    Frame,
}

impl CameraMode {
    /// Whether the zoom happens inside a fixed frame (content) rather than scaling the card:
    /// `auto` is content on a portrait canvas (D13).
    pub fn zooms_content(self, portrait: bool) -> bool {
        match self {
            Self::Content => true,
            Self::Frame => false,
            Self::Auto => portrait,
        }
    }

    /// Wire names, in order.
    pub const NAMES: [(Self, &'static str); 3] = [
        (Self::Auto, "auto"),
        (Self::Content, "content"),
        (Self::Frame, "frame"),
    ];

    /// Wire name.
    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(v, _)| *v == self)
            .map_or("auto", |(_, n)| n)
    }

    /// Parses a wire name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(v, _)| *v)
    }
}

/// Camera spring presets (critically damped or close to it; bounce 0 for cameras).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spring {
    /// Stiffness 205, damping 33.3, mass 1.5: 90% in about 0.32 s (Cap's default).
    #[default]
    Focused,
    /// About 0.9 s to settle, for slow pushes.
    Smooth,
    /// About 0.3 s to settle.
    Snappy,
    /// No motion: the camera jumps to the target.
    Instant,
}

impl Spring {
    /// Wire names, in order.
    pub const NAMES: [(Self, &'static str); 4] = [
        (Self::Focused, "focused"),
        (Self::Smooth, "smooth"),
        (Self::Snappy, "snappy"),
        (Self::Instant, "instant"),
    ];

    /// Wire name.
    pub fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(v, _)| *v == self)
            .map_or("focused", |(_, n)| n)
    }

    /// Parses a wire name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(v, _)| *v)
    }

    /// Stiffness, damping and mass, or `None` for an instant cut.
    pub fn physics(self) -> Option<[f64; 3]> {
        match self {
            Self::Focused => Some([205.0, 33.3, 1.5]),
            Self::Smooth => Some([60.0, 15.5, 1.0]),
            Self::Snappy => Some([500.0, 44.7, 1.0]),
            Self::Instant => None,
        }
    }
}

/// Generator kinds. Colors and stroke come from the item's `shape` effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    /// Flat fill of the bounds.
    Solid,
    /// Two-color gradient fill.
    Gradient,
    /// Rectangle with optional rounded corners.
    Rect,
    /// Ellipse inscribed in the bounds.
    Ellipse,
    /// Arrow with a head at the end point.
    Arrow,
    /// Straight line.
    Line,
    /// Rounded box with a pointer, for annotations.
    Callout,
    /// Annulus inscribed in the bounds, `strokeWidth` thick; progress rings sweep it (T45.4).
    Ring,
    /// Heart inscribed in the bounds (particles, T45.4).
    Heart,
}

impl ShapeKind {
    /// Every kind with its wire name.
    pub const ALL: [(Self, &'static str); 9] = [
        (Self::Solid, "solid"),
        (Self::Gradient, "gradient"),
        (Self::Rect, "rect"),
        (Self::Ellipse, "ellipse"),
        (Self::Arrow, "arrow"),
        (Self::Line, "line"),
        (Self::Callout, "callout"),
        (Self::Ring, "ring"),
        (Self::Heart, "heart"),
    ];

    /// Wire name.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(kind, _)| *kind == self)
            .map_or("rect", |(_, name)| name)
    }

    /// Parses a wire name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(kind, _)| *kind)
    }
}

/// A sound item: an audio file placed on an audio lane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sound {
    /// Audio media descriptor.
    pub media: MediaId,
    /// First source frame.
    pub source: Frame,
    /// Linear gain, 0…2.
    pub volume: f64,
    /// Lowers the sound while someone speaks.
    pub duck: Option<Duck>,
}

/// Ducking under speech.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Duck {
    /// Linear gain while ducked, 0…1.
    pub amount: f64,
    /// Frames to duck before speech starts.
    pub attack: Frame,
    /// Frames to recover after speech ends.
    pub release: Frame,
}

/// Settings specific to a video or still overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    /// Source media.
    pub media: MediaId,
    /// First media frame.
    pub source: Frame,
    /// Destination in the canvas.
    pub placement: Placement,
    /// Whether the presenter is in front.
    pub behind_presenter: bool,
    /// Whether to mute the media.
    pub muted: bool,
    /// Crop and fit.
    pub framing: Framing,
}

/// Overlay placement in the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Fill the canvas.
    Full,
    /// Centered card.
    Card,
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    BottomRight,
}

/// Custom display overrides for a caption word.
#[derive(Debug, PartialEq, Eq)]
pub struct CaptionEdit {
    /// Word to override.
    pub word: WordId,
    /// Replacement text, if present.
    pub text: Option<String>,
    /// Whether to emphasize the word.
    pub emph: bool,
    /// Optional emoji accompaniment.
    pub emoji: Option<String>,
}

/// Named timeline point.
#[derive(Debug, PartialEq, Eq)]
pub struct Marker {
    /// Stable marker ID.
    pub id: MarkerId,
    /// Timeline frame.
    pub at: Frame,
    /// Display name.
    pub name: String,
    /// Optional timeline swatch.
    pub color: Option<String>,
}

/// Effect attached to a project, lane, clip, or item.
#[derive(Debug, PartialEq)]
pub struct Effect {
    /// Stable effect ID.
    pub id: EffectId,
    /// Effect type as named by its schema.
    pub kind: String,
    /// Explicit enable override, when present.
    pub enabled: Option<bool>,
    /// Explicit effect parameters. Omitted values inherit.
    pub params: Vec<EffectParam>,
}

/// One override in an effect's parameter set.
#[derive(Debug, PartialEq)]
pub struct EffectParam {
    /// Parameter name.
    pub name: String,
    /// Parameter value.
    pub value: EffectValue,
}

/// Value types used by effect schemas.
#[derive(Debug, PartialEq)]
pub enum EffectValue {
    /// Numeric parameter.
    Number(f64),
    /// Boolean toggle.
    Bool(bool),
    /// Text, enum name, or color.
    Text(String),
    /// Keyframed number (PLAN.md D5).
    Keys(Vec<Key>),
}

impl Clone for Spine {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            effects: self.effects.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.name.clone_from(&source.name);
        self.effects.clone_from(&source.effects);
    }
}

impl Clone for Lane {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind,
            effects: self.effects.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.name.clone_from(&source.name);
        self.kind = source.kind;
        self.effects.clone_from(&source.effects);
    }
}

impl Clone for ClipData {
    fn clone(&self) -> Self {
        Self {
            media: self.media,
            appearance: self.appearance.clone(),
            framing: self.framing,
            enabled: self.enabled.clone(),
            volume: self.volume,
            hidden_keys: self.hidden_keys.clone(),
            joins: self.joins.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.media = source.media;
        self.appearance.clone_from(&source.appearance);
        self.framing = source.framing;
        self.enabled.clone_from(&source.enabled);
        self.volume = source.volume;
        self.hidden_keys.clone_from(&source.hidden_keys);
        self.joins.clone_from(&source.joins);
    }
}

impl Clone for Appearance {
    fn clone(&self) -> Self {
        Self {
            fade_in: self.fade_in,
            fade_out: self.fade_out,
            name: self.name.clone(),
            color: self.color.clone(),
            effects: self.effects.clone(),
            z: self.z,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.fade_in = source.fade_in;
        self.fade_out = source.fade_out;
        self.name.clone_from(&source.name);
        self.color.clone_from(&source.color);
        self.effects.clone_from(&source.effects);
        self.z = source.z;
    }
}

impl Clone for ItemData {
    fn clone(&self) -> Self {
        Self {
            lane: self.lane,
            kind: self.kind.clone(),
            appearance: self.appearance.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.lane = source.lane;
        self.kind.clone_from(&source.kind);
        self.appearance.clone_from(&source.appearance);
    }
}

impl Clone for CaptionEdit {
    fn clone(&self) -> Self {
        Self {
            word: self.word,
            text: self.text.clone(),
            emph: self.emph,
            emoji: self.emoji.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.word = source.word;
        self.text.clone_from(&source.text);
        self.emph = source.emph;
        self.emoji.clone_from(&source.emoji);
    }
}

impl Clone for Marker {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            at: self.at,
            name: self.name.clone(),
            color: self.color.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.at = source.at;
        self.name.clone_from(&source.name);
        self.color.clone_from(&source.color);
    }
}

impl Clone for Effect {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            kind: self.kind.clone(),
            enabled: self.enabled,
            params: self.params.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.id = source.id;
        self.kind.clone_from(&source.kind);
        self.enabled = source.enabled;
        self.params.clone_from(&source.params);
    }
}

impl Clone for EffectParam {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            value: self.value.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.name.clone_from(&source.name);
        self.value.clone_from(&source.value);
    }
}

impl Clone for ItemKind {
    fn clone(&self) -> Self {
        match self {
            Self::Title { text } => Self::Title { text: text.clone() },
            Self::Media(overlay) => Self::Media(*overlay),
            Self::Sound(sound) => Self::Sound(*sound),
            Self::Shape { shape, rect } => Self::Shape {
                shape: *shape,
                rect: *rect,
            },
            Self::Zoom(zoom) => Self::Zoom(*zoom),
            Self::Caption { words, edits, text } => Self::Caption {
                words: words.clone(),
                edits: edits.clone(),
                text: text.clone(),
            },
        }
    }

    fn clone_from(&mut self, source: &Self) {
        match (self, source) {
            (Self::Title { text }, Self::Title { text: other }) => text.clone_from(other),
            (
                Self::Caption { words, edits, text },
                Self::Caption {
                    words: source_words,
                    edits: source_edits,
                    text: source_text,
                },
            ) => {
                words.clone_from(source_words);
                edits.clone_from(source_edits);
                text.clone_from(source_text);
            }
            (dest, source) => *dest = source.clone(),
        }
    }
}

impl Clone for EffectValue {
    fn clone(&self) -> Self {
        match self {
            Self::Number(n) => Self::Number(*n),
            Self::Bool(b) => Self::Bool(*b),
            Self::Text(s) => Self::Text(s.clone()),
            Self::Keys(keys) => Self::Keys(keys.clone()),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        match (self, source) {
            (Self::Text(text), Self::Text(other)) => text.clone_from(other),
            (Self::Keys(keys), Self::Keys(other)) => keys.clone_from(other),
            (dst, src) => *dst = src.clone(),
        }
    }
}
