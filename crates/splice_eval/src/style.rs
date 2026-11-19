use super::look::Num;
use splice_model::{Effect, EffectParam, EffectValue, Frame};

/// Window background inside a layer's placement rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// Keep the existing backdrop.
    None,
    /// Fill with an RGB color.
    Solid(u32),
    /// Diagonal gradient between two RGB colors.
    Gradient(u32, u32),
}

/// Per-picture Window effect after inheritance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Padding as a fraction of the shorter placement edge.
    pub padding: f64,
    /// Corner radius as a fraction of the shorter picture edge.
    pub corner: f64,
    /// Shadow intensity.
    pub shadow: f64,
    /// Inset stroke intensity.
    pub inset: f64,
    /// Background treatment.
    pub background: Background,
}

/// Cursor treatment before the picture's crop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cursor {
    /// Scale relative to the recorded cursor.
    pub scale: f64,
    /// Draw rings at recorded clicks.
    pub click_rings: bool,
    /// Smoothing, idle hiding, click pulse and synthesized paths (T39).
    pub motion: crate::cursor::Motion,
}

/// Camera bubble corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CameraStyle {
    /// Background behind the presenter: 0 keep, 1 blur, 2 remove (person matte).
    pub(super) background: u8,
    pub(super) corner: Corner,
    pub(super) size: f64,
    pub(super) roundness: f64,
    pub(super) shadow: f64,
}

/// Keystroke-chip placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPosition {
    /// Bottom left.
    BottomLeft,
    /// Bottom center.
    BottomCenter,
    /// Top left.
    TopLeft,
}

/// Style for an active recorded keystroke chip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keys {
    /// Scale relative to the default chip.
    pub scale: f64,
    /// Overlay placement.
    pub position: KeyPosition,
}

/// Preset used to lay out and highlight caption words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionPreset {
    /// Highlight and scale the current word.
    Bold,
    /// Highlight spoken words.
    Karaoke,
    /// Reveal each word with a scale animation.
    Pop,
    /// Highlight the current word with a box.
    Box,
    /// Plain text on a dark backing.
    Clean,
    /// Reveal letters.
    Typewriter,
}

/// Caption style resolved from the project, lane and cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionStyle {
    /// Word highlight and reveal behavior.
    pub preset: CaptionPreset,
    /// Font size as a fraction of canvas width.
    pub size: f64,
    /// Vertical center as a fraction of canvas height.
    pub position: f64,
    /// Text RGB.
    pub color: u32,
    /// Active word RGB.
    pub active: u32,
    /// Stroke relative to font size.
    pub stroke: f64,
    /// Uppercase the display text.
    pub uppercase: bool,
    /// Safe-zone margins in top, bottom, left, right order.
    pub safe: [f64; 4],
}

pub(super) fn window(scopes: &[&[Effect]]) -> Option<Window> {
    let style = Resolution::new(scopes, "window");
    if !style.enabled.unwrap_or(false) {
        return None;
    }
    let a = style.color("color", 0x5b6f63);
    let b = style.color("color2", 0x1d2420);
    let background = match style.text("background") {
        Some("none") => Background::None,
        Some("color") => Background::Solid(a),
        _ => Background::Gradient(a, b),
    };
    Some(Window {
        padding: style.number("padding", 0.07).clamp(0.0, 0.3),
        corner: style.number("corner", 0.03).clamp(0.0, 0.12),
        shadow: style.number("shadow", 0.6).clamp(0.0, 1.0),
        inset: style.number("inset", 0.0).clamp(0.0, 0.02),
        background,
    })
}

pub(super) fn cursor(scopes: &[&[Effect]]) -> Option<Cursor> {
    let style = Resolution::new(scopes, "cursor");
    if !style.enabled.unwrap_or(false) {
        // No cursor effect: the recorded path as captured.
        return Some(Cursor {
            scale: 1.0,
            click_rings: false,
            motion: crate::cursor::Motion {
                smoothing: 0.0,
                hide_idle: false,
                click_pulse: false,
                synthesize: false,
                ..crate::cursor::Motion::default()
            },
        });
    }
    if style.boolean("hidden", false) {
        return None;
    }
    let base = crate::cursor::Motion::default();
    Some(Cursor {
        scale: style.number("size", 1.5),
        click_rings: style.boolean("clickRings", true),
        motion: crate::cursor::Motion {
            smoothing: style.number("smoothing", base.smoothing),
            hide_idle: style.boolean("hideWhenIdle", base.hide_idle),
            click_pulse: style.boolean("clickPulse", base.click_pulse),
            synthesize: style.text("path") != Some("recorded"),
            arc: style.number("arc", base.arc),
            arrive_before: style.number("arriveBefore", base.arrive_before),
        },
    })
}

pub(super) fn camera(scopes: &[&[Effect]]) -> CameraStyle {
    let mut style = Resolution::new(scopes, "camera");
    if style.enabled == Some(false) {
        style.params.clear();
    }
    let corner = match style.text("corner") {
        Some("tl") => Corner::TopLeft,
        Some("tr") => Corner::TopRight,
        Some("bl") => Corner::BottomLeft,
        _ => Corner::BottomRight,
    };
    CameraStyle {
        background: match style.text("background") {
            Some("blur") => 1,
            Some("remove") => 2,
            _ => 0,
        },
        corner,
        size: style.number("size", 0.24).clamp(0.12, 0.45),
        roundness: style.number("roundness", 0.2).clamp(0.0, 1.0),
        shadow: style.number("shadow", 0.5).clamp(0.0, 1.0),
    }
}

pub(super) fn keys(scopes: &[&[Effect]]) -> Option<Keys> {
    let style = Resolution::new(scopes, "keystrokes");
    if style.enabled == Some(false) {
        return None;
    }
    let position = match style.text("position") {
        Some("bottom-center") => KeyPosition::BottomCenter,
        Some("top-left") => KeyPosition::TopLeft,
        _ => KeyPosition::BottomLeft,
    };
    Some(Keys {
        scale: style.number("size", 1.0).clamp(0.5, 3.0),
        position,
    })
}

pub(super) fn caption(scopes: &[&[Effect]]) -> Option<CaptionStyle> {
    let style = Resolution::new(scopes, "captions");
    if style.enabled == Some(false) {
        return None;
    }
    let named = style.text("preset").unwrap_or("bold");
    let base = CAPTION_PRESETS
        .iter()
        .find(|p| p.0 == named)
        .unwrap_or(&CAPTION_PRESETS[0]);
    let preset = base.1;
    let safe = match style.text("safe") {
        Some("reels") => [0.14, 0.2, 0.05, 0.12],
        Some("shorts") => [0.12, 0.18, 0.05, 0.14],
        Some("none") => [0.04; 4],
        _ => [0.1, 0.2, 0.05, 0.15],
    };
    Some(CaptionStyle {
        preset,
        size: style.number("size", base.6).clamp(0.03, 0.14),
        position: style.number("position", 0.68).clamp(0.1, 0.9),
        color: style.color("color", base.2),
        active: style.color("active", base.3),
        stroke: style.number("stroke", base.4).clamp(0.0, 0.3),
        uppercase: style.boolean("uppercase", base.5),
        safe,
    })
}

/// Named caption looks: name, render behavior, text, highlight, stroke, uppercase, size.
pub const CAPTION_PRESETS: &[(&str, CaptionPreset, u32, u32, f64, bool, f64)] = &[
    (
        "bold",
        CaptionPreset::Bold,
        0xffffff,
        0xffd23f,
        0.14,
        true,
        0.075,
    ),
    (
        "karaoke",
        CaptionPreset::Karaoke,
        0xffffff,
        0xffd23f,
        0.14,
        true,
        0.075,
    ),
    (
        "pop",
        CaptionPreset::Pop,
        0xffffff,
        0xffd23f,
        0.14,
        true,
        0.075,
    ),
    (
        "box",
        CaptionPreset::Box,
        0xffffff,
        0xffd23f,
        0.14,
        true,
        0.075,
    ),
    (
        "clean",
        CaptionPreset::Clean,
        0xffffff,
        0xffd23f,
        0.14,
        false,
        0.075,
    ),
    (
        "typewriter",
        CaptionPreset::Typewriter,
        0xffffff,
        0xffd23f,
        0.14,
        true,
        0.075,
    ),
    (
        "neon",
        CaptionPreset::Bold,
        0xffffff,
        0x39ff14,
        0.2,
        true,
        0.075,
    ),
    (
        "comic",
        CaptionPreset::Pop,
        0xffffff,
        0xff3b30,
        0.25,
        true,
        0.085,
    ),
    (
        "minimal",
        CaptionPreset::Clean,
        0xffffff,
        0xffffff,
        0.0,
        false,
        0.055,
    ),
    (
        "subtitle",
        CaptionPreset::Clean,
        0xffffff,
        0xffffff,
        0.08,
        false,
        0.05,
    ),
    (
        "highlight",
        CaptionPreset::Box,
        0xffffff,
        0xff2d55,
        0.1,
        false,
        0.07,
    ),
    (
        "outline",
        CaptionPreset::Bold,
        0xffffff,
        0xffffff,
        0.3,
        true,
        0.08,
    ),
    (
        "shadow",
        CaptionPreset::Clean,
        0xffffff,
        0xffd23f,
        0.0,
        false,
        0.07,
    ),
    (
        "bubble",
        CaptionPreset::Box,
        0x111111,
        0xffd23f,
        0.0,
        false,
        0.07,
    ),
    (
        "gaming",
        CaptionPreset::Pop,
        0x00e5ff,
        0xff00e5,
        0.22,
        true,
        0.08,
    ),
    (
        "news",
        CaptionPreset::Box,
        0xffffff,
        0xd70015,
        0.0,
        true,
        0.055,
    ),
    (
        "podcast",
        CaptionPreset::Karaoke,
        0xf5f5f5,
        0x7c5cff,
        0.1,
        false,
        0.065,
    ),
    (
        "cinematic",
        CaptionPreset::Clean,
        0xf0e6d2,
        0xf0e6d2,
        0.0,
        false,
        0.05,
    ),
    (
        "glow",
        CaptionPreset::Karaoke,
        0xffffff,
        0x00d1ff,
        0.18,
        false,
        0.075,
    ),
    (
        "retro",
        CaptionPreset::Pop,
        0xffe66d,
        0xff6b6b,
        0.2,
        true,
        0.08,
    ),
    (
        "elegant",
        CaptionPreset::Typewriter,
        0xffffff,
        0xe0c097,
        0.0,
        false,
        0.06,
    ),
    (
        "hype",
        CaptionPreset::Bold,
        0xffffff,
        0xff9500,
        0.2,
        true,
        0.09,
    ),
];

/// Returns an effect's enabled state and merged overrides in project-to-item scope order.
pub fn resolve_effect(scopes: &[&[Effect]], kind: &str) -> Option<(bool, Vec<EffectParam>)> {
    let result = Resolution::new(scopes, kind);
    result.enabled.map(|enabled| (enabled, result.params))
}

/// One effect type's overrides merged project → lane → owner (the only merge in eval).
pub(crate) struct Resolution {
    pub(crate) enabled: Option<bool>,
    pub(crate) params: Vec<EffectParam>,
}

impl Resolution {
    pub(crate) fn new(scopes: &[&[Effect]], kind: &str) -> Self {
        Self::shifted(scopes, &[], kind)
    }

    /// Like `new`, adding `shifts[i]` to the key frames of scope `i` (absent shifts are 0), so
    /// timeline-absolute lane and project keys become owner-relative.
    pub(crate) fn shifted(scopes: &[&[Effect]], shifts: &[Frame], kind: &str) -> Self {
        let mut result = Self {
            enabled: None,
            params: Vec::new(),
        };
        for (index, scope) in scopes.iter().enumerate() {
            let shift = shifts.get(index).copied().unwrap_or(0);
            for effect in scope.iter().filter(|effect| effect.kind == kind) {
                result.enabled = effect.enabled.or(result.enabled).or(Some(true));
                for param in &effect.params {
                    let mut value = param.value.clone();
                    if let EffectValue::Keys(keys) = &mut value {
                        keys.iter_mut().for_each(|key| key.at += shift);
                    }
                    match result.params.iter_mut().find(|p| p.name == param.name) {
                        Some(existing) => existing.value = value,
                        None => result.params.push(EffectParam {
                            name: param.name.clone(),
                            value,
                        }),
                    }
                }
            }
        }
        result
    }

    /// The merged effect when it is on.
    pub(crate) fn on(scopes: &[&[Effect]], shifts: &[Frame], kind: &str) -> Option<Self> {
        let result = Self::shifted(scopes, shifts, kind);
        (result.enabled == Some(true)).then_some(result)
    }

    /// A number or its keys.
    pub(crate) fn num(&self, name: &str, default: f64) -> Num {
        match self.value(name) {
            Some(EffectValue::Number(n)) if n.is_finite() => Num::Const(*n),
            Some(EffectValue::Keys(keys)) if !keys.is_empty() => Num::Keys(keys.clone()),
            _ => Num::Const(default),
        }
    }

    fn value(&self, name: &str) -> Option<&EffectValue> {
        self.params
            .iter()
            .find(|param| param.name == name)
            .map(|param| &param.value)
    }

    pub(crate) fn number(&self, name: &str, default: f64) -> f64 {
        match self.value(name) {
            Some(EffectValue::Number(n)) if n.is_finite() => *n,
            _ => default,
        }
    }

    pub(crate) fn boolean(&self, name: &str, default: bool) -> bool {
        match self.value(name) {
            Some(EffectValue::Bool(b)) => *b,
            _ => default,
        }
    }

    pub(crate) fn text(&self, name: &str) -> Option<&str> {
        match self.value(name) {
            Some(EffectValue::Text(s)) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn color(&self, name: &str, default: u32) -> u32 {
        self.text(name)
            .and_then(|text| text.strip_prefix('#'))
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .unwrap_or(default)
    }
}

/// How a text's words or characters come in (T18.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animate {
    /// Static.
    None,
    /// Opacity.
    Fade,
    /// Slide up while fading in.
    Rise,
    /// Scale from small.
    Pop,
    /// One unit at a time, no fade.
    Typewriter,
    /// Units light up in the accent color.
    Highlight,
    /// Overshooting pop.
    Bounce,
    /// Slide in from the left.
    Slide,
}

/// Resolved text styling for a title or caption (project › lane › item).
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Font family.
    pub font: String,
    /// Weight: 0 regular … 5 black.
    pub weight: u8,
    /// Size as a fraction of the canvas's shorter side.
    pub size: f64,
    /// Text RGB.
    pub color: u32,
    /// Stroke RGB.
    pub stroke: u32,
    /// Stroke width relative to the font size.
    pub stroke_width: f64,
    /// Drop shadow 0…1.
    pub shadow: f64,
    /// Backing box: 0 none, 1 solid, 2 rounded.
    pub boxed: u8,
    /// Backing RGB.
    pub box_color: u32,
    /// 0 left, 1 center, 2 right.
    pub align: u8,
    /// Line height relative to the font size.
    pub line_height: f64,
    /// Vertical placement: 0 top, 1 middle, 2 bottom, 3 lower third.
    pub position: u8,
    /// Animator.
    pub animate: Animate,
    /// Animate words (0), characters (1) or lines (2).
    pub unit: u8,
    /// Frames each unit takes to come in.
    pub duration: f64,
    /// Frames between units.
    pub stagger: f64,
}

fn index(options: &[&str], value: Option<&str>, default: u8) -> u8 {
    value
        .and_then(|value| options.iter().position(|o| *o == value))
        .map_or(default, |i| i as u8)
}

/// The text style of an item, when the project, its lane or the item set one.
pub(super) fn text(scopes: &[&[Effect]]) -> Option<TextStyle> {
    let style = Resolution::on(scopes, &[], "text");
    let animate = Resolution::on(scopes, &[], "animate");
    if style.is_none() && animate.is_none() {
        return None;
    }
    let style = style.unwrap_or(Resolution {
        enabled: Some(true),
        params: Vec::new(),
    });
    let (preset, unit, duration, stagger) = match &animate {
        Some(a) => (
            a.text("preset"),
            index(&["word", "character", "line"], a.text("unit"), 0),
            a.number("duration", 8.0).clamp(1.0, 60.0),
            a.number("stagger", 3.0).clamp(0.0, 30.0),
        ),
        None => (None, 0, 8.0, 3.0),
    };
    Some(TextStyle {
        font: style.text("font").unwrap_or("SF Pro Display").to_owned(),
        weight: index(
            &["regular", "medium", "semibold", "bold", "heavy", "black"],
            style.text("weight"),
            3,
        ),
        size: style.number("size", 0.06).clamp(0.02, 0.3),
        color: style.color("color", 0xffffff),
        stroke: style.color("stroke", 0),
        stroke_width: style.number("strokeWidth", 0.0).clamp(0.0, 0.3),
        shadow: style.number("shadow", 0.3).clamp(0.0, 1.0),
        boxed: index(&["none", "solid", "rounded"], style.text("box"), 0),
        box_color: style.color("boxColor", 0),
        align: index(&["left", "center", "right"], style.text("align"), 1),
        line_height: style.number("lineHeight", 1.15).clamp(0.8, 2.0),
        position: index(
            &["top", "middle", "bottom", "lowerThird"],
            style.text("position"),
            0,
        ),
        animate: match preset {
            Some("fade") => Animate::Fade,
            Some("rise") => Animate::Rise,
            Some("pop") => Animate::Pop,
            Some("typewriter") => Animate::Typewriter,
            Some("highlight") => Animate::Highlight,
            Some("bounce") => Animate::Bounce,
            Some("slide") => Animate::Slide,
            _ => Animate::None,
        },
        unit,
        duration,
        stagger,
    })
}
