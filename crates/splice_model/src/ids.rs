use std::{fmt, str::FromStr};

/// An invalid spelling, prefix, or value for an ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidId;

impl fmt::Display for InvalidId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid Splice ID")
    }
}

impl std::error::Error for InvalidId {}

fn parse_serial(text: &str, prefix: &str, radix: u32) -> Result<u64, InvalidId> {
    let digits = text.strip_prefix(prefix).ok_or(InvalidId)?;
    if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
        return Err(InvalidId);
    }
    if !digits
        .bytes()
        .all(|digit| digit.is_ascii_digit() || (radix == 36 && digit.is_ascii_lowercase()))
    {
        return Err(InvalidId);
    }
    u64::from_str_radix(digits, radix).map_err(|_| InvalidId)
}

fn write_serial(f: &mut fmt::Formatter<'_>, prefix: &str, mut serial: u64) -> fmt::Result {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut bytes = [0; 13];
    let mut start = bytes.len();
    loop {
        start -= 1;
        bytes[start] = DIGITS[(serial % 36) as usize];
        serial /= 36;
        if serial == 0 {
            break;
        }
    }
    f.write_str(prefix)?;
    f.write_str(str::from_utf8(&bytes[start..]).map_err(|_| fmt::Error)?)
}

/// Stable ID for a spine clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(u64);

impl ClipId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for ClipId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "clp_", 36).map(Self)
    }
}

impl fmt::Display for ClipId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "clp_", self.0)
    }
}

/// Stable ID for a overlay item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId(u64);

impl ItemId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for ItemId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "ovl_", 36).map(Self)
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "ovl_", self.0)
    }
}

/// Stable ID for a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LaneId(u64);

impl LaneId {
    /// Reserved ID for the primary video lane, `lan_spine`.
    pub const SPINE: Self = Self(48_219_818);

    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for LaneId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "lan_", 36).map(Self)
    }
}

impl fmt::Display for LaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "lan_", self.0)
    }
}

/// Stable ID for a marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MarkerId(u64);

impl MarkerId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for MarkerId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "mrk_", 36).map(Self)
    }
}

impl fmt::Display for MarkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "mrk_", self.0)
    }
}

/// Stable ID for a media descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MediaId(u64);

impl MediaId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for MediaId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "med_", 36).map(Self)
    }
}

impl fmt::Display for MediaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "med_", self.0)
    }
}

/// Stable ID for a effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectId(u64);

impl EffectId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for EffectId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "fx_", 36).map(Self)
    }
}

impl fmt::Display for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "fx_", self.0)
    }
}

/// Stable ID for a editing session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for SessionId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "ses_", 36).map(Self)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "ses_", self.0)
    }
}

/// Stable ID for a background job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct JobId(u64);

impl JobId {
    /// Constructs an ID using a serial assigned by the owning store.
    pub const fn from_serial(serial: u64) -> Self {
        Self(serial)
    }

    /// Returns the numeric serial.
    pub const fn serial(self) -> u64 {
        self.0
    }
}

impl FromStr for JobId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_serial(text, "job_", 36).map(Self)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_serial(f, "job_", self.0)
    }
}

/// Project-unique word ID: the media whose transcript holds the word and the word's index in it.
/// Spelled `w:<media>:<n>`, `<media>` being the media ID's base36 serial (`med_screen` →
/// `w:screen:12`). A legacy `w<n>` (before 0.4) names no media; loading binds it to its transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WordId {
    media: u64,
    index: u32,
}

impl WordId {
    /// The `index`th word (1-based) of `media`'s transcript.
    pub const fn new(media: MediaId, index: u32) -> Self {
        Self {
            media: media.serial(),
            index,
        }
    }

    /// A legacy `w<n>` ID whose media is not known yet.
    pub const fn legacy(index: u32) -> Self {
        Self { media: 0, index }
    }

    /// The media whose transcript holds the word, or `None` for an unbound legacy ID.
    pub const fn media(self) -> Option<MediaId> {
        if self.media == 0 {
            None
        } else {
            Some(MediaId::from_serial(self.media))
        }
    }

    /// Returns the word's index within its transcript.
    pub const fn index(self) -> u32 {
        self.index
    }
}

impl FromStr for WordId {
    type Err = InvalidId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let Some(rest) = text.strip_prefix("w:") else {
            let index = parse_serial(text, "w", 10)?;
            return u32::try_from(index)
                .map(Self::legacy)
                .map_err(|_| InvalidId);
        };
        let (media, index) = rest.split_once(':').ok_or(InvalidId)?;
        let media = parse_serial(media, "", 36)?;
        let index = u32::try_from(parse_serial(index, "", 10)?).map_err(|_| InvalidId)?;
        if media == 0 {
            return Err(InvalidId);
        }
        Ok(Self { media, index })
    }
}

impl fmt::Display for WordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.media == 0 {
            return write!(f, "w{}", self.index);
        }
        write_serial(f, "w:", self.media)?;
        write!(f, ":{}", self.index)
    }
}

/// IDs that can be addressed by timeline editing commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimelineId {
    /// A spine clip.
    Clip(ClipId),
    /// An overlay item.
    Item(ItemId),
    /// A marker.
    Marker(MarkerId),
}

impl fmt::Display for TimelineId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clip(id) => fmt::Display::fmt(id, f),
            Self::Item(id) => fmt::Display::fmt(id, f),
            Self::Marker(id) => fmt::Display::fmt(id, f),
        }
    }
}
