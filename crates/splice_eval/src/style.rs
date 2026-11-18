use splice_model::{Effect, EffectParam, EffectValue};

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
}

/// Click zoom settings for an event-aware renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickZoom {
    /// Maximum zoom scale.
    pub scale: f64,
    /// Hold duration on the source clock.
    pub hold: f64,
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
        return Some(Cursor {
            scale: 1.0,
            click_rings: false,
        });
    }
    if style.boolean("hidden", false) {
        return None;
    }
    Some(Cursor {
        scale: style.number("size", 1.5).clamp(0.5, 4.0),
        click_rings: style.boolean("clickRings", true),
    })
}

pub(super) fn click_zoom(scopes: &[&[Effect]]) -> Option<ClickZoom> {
    let style = Resolution::new(scopes, "clickZoom");
    style.enabled.unwrap_or(false).then(|| ClickZoom {
        scale: style.number("scale", 1.6).clamp(1.05, 2.5),
        hold: style.number("hold", 45.0).clamp(15.0, 120.0),
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
    let preset = match style.text("preset") {
        Some("karaoke") => CaptionPreset::Karaoke,
        Some("pop") => CaptionPreset::Pop,
        Some("box") => CaptionPreset::Box,
        Some("clean") => CaptionPreset::Clean,
        Some("typewriter") => CaptionPreset::Typewriter,
        _ => CaptionPreset::Bold,
    };
    let safe = match style.text("safe") {
        Some("reels") => [0.14, 0.2, 0.05, 0.12],
        Some("shorts") => [0.12, 0.18, 0.05, 0.14],
        Some("none") => [0.04; 4],
        _ => [0.1, 0.2, 0.05, 0.15],
    };
    Some(CaptionStyle {
        preset,
        size: style.number("size", 0.075).clamp(0.03, 0.14),
        position: style.number("position", 0.68).clamp(0.1, 0.9),
        color: style.color("color", 0xffffff),
        active: style.color("active", 0xffd23f),
        stroke: style.number("stroke", 0.14).clamp(0.0, 0.3),
        uppercase: preset != CaptionPreset::Clean && style.boolean("uppercase", true),
        safe,
    })
}

struct Resolution {
    enabled: Option<bool>,
    params: Vec<EffectParam>,
}

impl Resolution {
    fn new(scopes: &[&[Effect]], kind: &str) -> Self {
        let mut result = Self {
            enabled: None,
            params: Vec::new(),
        };
        for scope in scopes {
            if let Some(effect) = scope.iter().find(|effect| effect.kind == kind) {
                result.enabled = effect.enabled.or(result.enabled).or(Some(true));
                for param in &effect.params {
                    if let Some(existing) = result.params.iter_mut().find(|p| p.name == param.name)
                    {
                        existing.value.clone_from(&param.value);
                    } else {
                        result.params.push(param.clone());
                    }
                }
            }
        }
        result
    }

    fn value(&self, name: &str) -> Option<&EffectValue> {
        self.params
            .iter()
            .find(|param| param.name == name)
            .map(|param| &param.value)
    }

    fn number(&self, name: &str, default: f64) -> f64 {
        match self.value(name) {
            Some(EffectValue::Number(n)) if n.is_finite() => *n,
            _ => default,
        }
    }

    fn boolean(&self, name: &str, default: bool) -> bool {
        match self.value(name) {
            Some(EffectValue::Bool(b)) => *b,
            _ => default,
        }
    }

    fn text(&self, name: &str) -> Option<&str> {
        match self.value(name) {
            Some(EffectValue::Text(s)) => Some(s),
            _ => None,
        }
    }

    fn color(&self, name: &str, default: u32) -> u32 {
        self.text(name)
            .and_then(|text| text.strip_prefix('#'))
            .filter(|hex| hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .unwrap_or(default)
    }
}
