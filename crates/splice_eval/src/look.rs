//! Keyed per-layer treatment (PLAN.md T16, T18): transform, blend, animated crop, color, masks,
//! chroma key and cutout. Effects compile once per revision into a `Rig`; `Rig::look` samples it
//! at a frame without allocating.
use super::style::Resolution;
use splice_model::{Effect, Frame, Key, keys};

/// A number that may be keyed. Key frames are relative to the owner's start.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Num {
    Const(f64),
    Keys(Vec<Key>),
}

impl Num {
    pub(crate) fn at(&self, frame: f64) -> f64 {
        match self {
            Self::Const(value) => *value,
            Self::Keys(list) => keys::sample(list, frame),
        }
    }

    fn keys(&self) -> &[Key] {
        match self {
            Self::Const(_) => &[],
            Self::Keys(list) => list,
        }
    }
}

/// Compositing mode against what is already drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Blend {
    /// Source over.
    #[default]
    Normal,
    /// Multiply.
    Multiply,
    /// Screen.
    Screen,
    /// Overlay.
    Overlay,
    /// Additive.
    Add,
    /// Darken.
    Darken,
    /// Lighten.
    Lighten,
}

impl Blend {
    fn parse(name: &str) -> Self {
        match name {
            "multiply" => Self::Multiply,
            "screen" => Self::Screen,
            "overlay" => Self::Overlay,
            "add" => Self::Add,
            "darken" => Self::Darken,
            "lighten" => Self::Lighten,
            _ => Self::Normal,
        }
    }

    /// Shader index (0 is normal).
    pub fn index(self) -> u32 {
        self as u32
    }
}

/// Color correction at one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grade {
    /// Stops.
    pub exposure: f64,
    /// 1 is neutral.
    pub contrast: f64,
    /// 1 is neutral.
    pub saturation: f64,
    /// −1…1.
    pub vibrance: f64,
    /// −1 (cool) … 1 (warm).
    pub temperature: f64,
    /// −1 (green) … 1 (magenta).
    pub tint: f64,
    /// −1…1.
    pub highlights: f64,
    /// −1…1.
    pub shadows: f64,
    /// Index into `Evaluator::lut`, when a LUT is set.
    pub lut: Option<u16>,
    /// LUT mix 0…1.
    pub lut_amount: f64,
}

/// Mask outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskShape {
    /// Rectangle.
    Rect,
    /// Rounded rectangle.
    RoundRect,
    /// Ellipse.
    Ellipse,
}

/// What a mask does to the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskMode {
    /// Gaussian-like blur of the picture.
    Blur,
    /// Mosaic.
    Pixelate,
    /// Solid color.
    Cover,
    /// Darken everything outside.
    Spotlight,
}

/// One mask at a frame, in coordinates normalized to the layer's destination.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mask {
    /// Outline.
    pub shape: MaskShape,
    /// Treatment.
    pub mode: MaskMode,
    /// x, y, w, h normalized to the destination.
    pub rect: [f64; 4],
    /// Strength 0…1.
    pub amount: f64,
    /// Soft edge as a fraction of the destination's shorter side.
    pub feather: f64,
    /// Treat the outside instead.
    pub invert: bool,
    /// Cover color.
    pub color: u32,
}

/// Masks per layer.
pub const MAX_MASKS: usize = 4;

/// Chroma key at one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChromaKey {
    /// Key color.
    pub color: u32,
    /// Hue/chroma distance treated as background.
    pub tolerance: f64,
    /// Soft edge width.
    pub softness: f64,
    /// Spill suppression.
    pub spill: f64,
}

/// Person cutout from the segmentation matte (analysis store).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cutout {
    /// Blur the background instead of removing it.
    pub blur: bool,
    /// Strength 0…1.
    pub amount: f64,
}

/// Everything keyed about a layer at one frame. `Default` is the identity look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Translation in canvas pixels.
    pub offset: [f64; 2],
    /// Scale about `anchor`.
    pub scale: f64,
    /// Clockwise degrees about `anchor`.
    pub rotation: f64,
    /// Opacity multiplier.
    pub opacity: f64,
    /// Pivot, normalized to the layer's destination.
    pub anchor: [f64; 2],
    /// Compositing mode.
    pub blend: Blend,
    /// Animated crop overriding the static one, normalized to the source.
    pub crop: Option<[f64; 4]>,
    /// Color correction.
    pub color: Option<Grade>,
    /// Masks in order.
    pub masks: [Option<Mask>; MAX_MASKS],
    /// Chroma key.
    pub key: Option<ChromaKey>,
    /// Person cutout.
    pub cutout: Option<Cutout>,
    /// Visible canvas region in pixels (transition wipes), when limited.
    pub clip: Option<[f64; 4]>,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            offset: [0.0; 2],
            scale: 1.0,
            rotation: 0.0,
            opacity: 1.0,
            anchor: [0.5; 2],
            blend: Blend::Normal,
            crop: None,
            color: None,
            masks: [None; MAX_MASKS],
            key: None,
            cutout: None,
            clip: None,
        }
    }
}

impl Look {
    /// True when nothing changes the layer.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

struct RigMask {
    shape: MaskShape,
    mode: MaskMode,
    rect: [Num; 4],
    amount: Num,
    feather: Num,
    invert: bool,
    color: u32,
}

/// Compiled look for one layer.
#[derive(Default)]
pub(crate) struct Rig {
    transform: Option<[Num; 7]>,
    blend: Blend,
    crop: Option<[Num; 4]>,
    grade: Option<([Num; 9], Option<u16>)>,
    masks: Vec<RigMask>,
    key: Option<(u32, [Num; 3])>,
    cutout: Option<(bool, Num)>,
}

impl Rig {
    /// Compiles the visual effects of `local` (owner-relative keys) under inherited `color`
    /// scopes (project, lane). LUT paths are interned into `luts`.
    pub(crate) fn new(
        inherited: &[&[Effect]],
        shifts: &[Frame],
        local: &[Effect],
        luts: &mut Vec<String>,
    ) -> Self {
        let own: [&[Effect]; 1] = [local];
        let mut rig = Self::default();
        if let Some(t) = Resolution::on(&own, &[], "transform") {
            rig.transform = Some([
                t.num("x", 0.0),
                t.num("y", 0.0),
                t.num("scale", 1.0),
                t.num("rotation", 0.0),
                t.num("opacity", 1.0),
                t.num("anchorX", 0.5),
                t.num("anchorY", 0.5),
            ]);
            rig.blend = Blend::parse(t.text("blend").unwrap_or("normal"));
        }
        if let Some(c) = Resolution::on(&own, &[], "crop") {
            rig.crop = Some([
                c.num("x", 0.0),
                c.num("y", 0.0),
                c.num("w", 1.0),
                c.num("h", 1.0),
            ]);
        }
        let mut chain: Vec<&[Effect]> = Vec::with_capacity(inherited.len() + 1);
        chain.extend_from_slice(inherited);
        chain.push(local);
        if let Some(g) = Resolution::on(&chain, shifts, "color") {
            let lut = g.text("lut").filter(|path| !path.is_empty()).map(|path| {
                match luts.iter().position(|known| known == path) {
                    Some(index) => index as u16,
                    None => {
                        luts.push(path.into());
                        (luts.len() - 1) as u16
                    }
                }
            });
            rig.grade = Some((
                [
                    g.num("exposure", 0.0),
                    g.num("contrast", 1.0),
                    g.num("saturation", 1.0),
                    g.num("vibrance", 0.0),
                    g.num("temperature", 0.0),
                    g.num("tint", 0.0),
                    g.num("highlights", 0.0),
                    g.num("shadows", 0.0),
                    g.num("lutAmount", 1.0),
                ],
                lut,
            ));
        }
        for effect in local
            .iter()
            .filter(|effect| effect.kind == "mask" && effect.enabled != Some(false))
            .take(MAX_MASKS)
        {
            let single: [&[Effect]; 1] = [std::slice::from_ref(effect)];
            let m = Resolution::new(&single, "mask");
            rig.masks.push(RigMask {
                shape: match m.text("shape") {
                    Some("roundRect") => MaskShape::RoundRect,
                    Some("ellipse") => MaskShape::Ellipse,
                    _ => MaskShape::Rect,
                },
                mode: match m.text("mode") {
                    Some("pixelate") => MaskMode::Pixelate,
                    Some("cover") => MaskMode::Cover,
                    Some("spotlight") => MaskMode::Spotlight,
                    _ => MaskMode::Blur,
                },
                rect: [
                    m.num("x", 0.25),
                    m.num("y", 0.25),
                    m.num("w", 0.5),
                    m.num("h", 0.5),
                ],
                amount: m.num("amount", 0.6),
                feather: m.num("feather", 0.01),
                invert: m.boolean("invert", false),
                color: m.color("color", 0),
            });
        }
        if let Some(k) = Resolution::on(&own, &[], "key") {
            rig.key = Some((
                k.color("color", 0x00ff00),
                [
                    k.num("tolerance", 0.3),
                    k.num("softness", 0.1),
                    k.num("spill", 0.5),
                ],
            ));
        }
        if let Some(c) = Resolution::on(&own, &[], "cutout") {
            rig.cutout = Some((c.text("background") == Some("blur"), c.num("amount", 1.0)));
        }
        rig
    }

    /// Whether the layer needs the person matte.
    pub(crate) fn cutout(&self) -> bool {
        self.cutout.is_some()
    }

    /// Samples the rig at `frame` frames after the owner's start. `size` is the destination in
    /// canvas pixels; `canvas` the canvas size (transform x/y are fractions of its width/height).
    pub(crate) fn look(&self, frame: f64, canvas: [f64; 2]) -> Look {
        let mut look = Look::default();
        if let Some([x, y, scale, rotation, opacity, ax, ay]) = &self.transform {
            look.offset = [x.at(frame) * canvas[0], y.at(frame) * canvas[1]];
            look.scale = scale.at(frame).max(0.0);
            look.rotation = rotation.at(frame);
            look.opacity = opacity.at(frame).clamp(0.0, 1.0);
            look.anchor = [ax.at(frame), ay.at(frame)];
        }
        look.blend = self.blend;
        if let Some([x, y, w, h]) = &self.crop {
            let w = w.at(frame).clamp(0.01, 1.0);
            let h = h.at(frame).clamp(0.01, 1.0);
            look.crop = Some([
                x.at(frame).clamp(0.0, 1.0 - w),
                y.at(frame).clamp(0.0, 1.0 - h),
                w,
                h,
            ]);
        }
        if let Some((n, lut)) = &self.grade {
            look.color = Some(Grade {
                exposure: n[0].at(frame),
                contrast: n[1].at(frame),
                saturation: n[2].at(frame),
                vibrance: n[3].at(frame),
                temperature: n[4].at(frame),
                tint: n[5].at(frame),
                highlights: n[6].at(frame),
                shadows: n[7].at(frame),
                lut: *lut,
                lut_amount: n[8].at(frame).clamp(0.0, 1.0),
            });
        }
        for (slot, mask) in look.masks.iter_mut().zip(&self.masks) {
            *slot = Some(Mask {
                shape: mask.shape,
                mode: mask.mode,
                rect: [
                    mask.rect[0].at(frame),
                    mask.rect[1].at(frame),
                    mask.rect[2].at(frame).max(0.0),
                    mask.rect[3].at(frame).max(0.0),
                ],
                amount: mask.amount.at(frame).clamp(0.0, 1.0),
                feather: mask.feather.at(frame).max(0.0),
                invert: mask.invert,
                color: mask.color,
            });
        }
        if let Some((color, [tolerance, softness, spill])) = &self.key {
            look.key = Some(ChromaKey {
                color: *color,
                tolerance: tolerance.at(frame),
                softness: softness.at(frame),
                spill: spill.at(frame),
            });
        }
        if let Some((blur, amount)) = &self.cutout {
            look.cutout = Some(Cutout {
                blur: *blur,
                amount: amount.at(frame),
            });
        }
        look
    }
}

/// Audio treatment of an item after inheritance: keyed gain and pan, and the effect chain.
pub(crate) struct Mix {
    gain: Num,
    pan: Num,
    pub mute: bool,
    pub solo: bool,
}

impl Mix {
    pub(crate) fn new(scopes: &[&[Effect]], shifts: &[Frame]) -> Option<Self> {
        let v = Resolution::on(scopes, shifts, "volume")?;
        Some(Self {
            gain: v.num("gain", 1.0),
            pan: v.num("pan", 0.0),
            mute: v.boolean("mute", false),
            solo: v.boolean("solo", false),
        })
    }

    pub(crate) fn gain(&self, frame: f64) -> f64 {
        if self.mute {
            0.0
        } else {
            self.gain.at(frame).max(0.0)
        }
    }

    pub(crate) fn pan(&self, frame: f64) -> f64 {
        self.pan.at(frame).clamp(-1.0, 1.0)
    }

    pub(crate) fn breakpoints(&self, offset: Frame, out: &mut Vec<Frame>) {
        keys::breakpoints(self.gain.keys(), offset, 2, out);
        keys::breakpoints(self.pan.keys(), offset, 2, out);
    }
}

/// Offline audio processing for an item (rendered by the platform).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioFx {
    /// Low cut Hz, low/mid/high gain dB, mid frequency Hz.
    pub eq: Option<[f64; 5]>,
    /// Threshold dB, ratio, makeup gain dB.
    pub compressor: Option<[f64; 3]>,
    /// Gate threshold dB.
    pub gate: Option<f64>,
    /// Voice isolation mix 0…1.
    pub isolation: Option<f64>,
}

impl AudioFx {
    pub(crate) fn new(scopes: &[&[Effect]]) -> Self {
        let get = |kind| Resolution::on(scopes, &[], kind);
        let constant = |m: &Resolution, name, default| m.number(name, default);
        Self {
            eq: get("eq").map(|m| {
                [
                    constant(&m, "lowCut", 20.0),
                    constant(&m, "low", 0.0),
                    constant(&m, "mid", 0.0),
                    constant(&m, "high", 0.0),
                    constant(&m, "midFrequency", 1000.0),
                ]
            }),
            compressor: get("compressor").map(|m| {
                [
                    constant(&m, "threshold", -20.0),
                    constant(&m, "ratio", 3.0),
                    constant(&m, "gain", 0.0),
                ]
            }),
            gate: get("gate").map(|m| constant(&m, "threshold", -50.0)),
            isolation: get("voiceIsolation").map(|m| constant(&m, "amount", 1.0)),
        }
    }
}

/// A clip's transition in from the clip before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// Kind.
    pub kind: TransitionKind,
    /// Length in frames, centered on the cut.
    pub length: Frame,
    /// Direction for wipes, pushes and slides.
    pub direction: [f64; 2],
    /// Dip color.
    pub color: u32,
}

/// Transition kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionKind {
    /// Cross dissolve.
    Dissolve,
    /// Dip through a color.
    Dip,
    /// Hard-edged wipe.
    Wipe,
    /// The incoming picture pushes the outgoing one out.
    Push,
    /// The incoming picture slides over.
    Slide,
    /// Zoom and dissolve.
    Zoom,
}

pub(crate) fn transition(local: &[Effect]) -> Option<Transition> {
    let t = Resolution::on(&[local], &[], "transition")?;
    Some(Transition {
        kind: match t.text("kind") {
            Some("dip") => TransitionKind::Dip,
            Some("wipe") => TransitionKind::Wipe,
            Some("push") => TransitionKind::Push,
            Some("slide") => TransitionKind::Slide,
            Some("zoom") => TransitionKind::Zoom,
            _ => TransitionKind::Dissolve,
        },
        length: (t.number("length", 15.0).round() as Frame).max(2),
        direction: match t.text("direction") {
            Some("right") => [1.0, 0.0],
            Some("up") => [0.0, -1.0],
            Some("down") => [0.0, 1.0],
            _ => [-1.0, 0.0],
        },
        color: t.color("color", 0),
    })
}

/// Speed curve of a clip: rate keys (owner-relative), or `None` for constant speed.
pub(crate) fn speed(local: &[Effect]) -> Option<(Num, bool)> {
    let s = Resolution::on(&[local], &[], "speed")?;
    Some((s.num("rate", 1.0), s.boolean("pitch", true)))
}

/// Fill, second fill, stroke color, stroke width (fraction of the canvas's shorter side) and
/// corner (fraction of the shape's shorter side) from an item's `shape` effect.
pub(crate) fn shape_style(local: &[Effect]) -> (u32, u32, u32, f64, f64) {
    match Resolution::on(&[local], &[], "shape") {
        Some(s) => {
            let fill = s.color("fill", 0xffffff);
            (
                fill,
                s.color("fill2", fill),
                s.color("stroke", 0),
                s.number("strokeWidth", 0.0),
                s.number("corner", 0.0),
            )
        }
        None => (0xffffff, 0xffffff, 0, 0.0, 0.0),
    }
}
