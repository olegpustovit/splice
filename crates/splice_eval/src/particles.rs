//! Particle emitters (T45.4): every particle is drawn from a seeded generator when the timeline
//! is prepared, so a frame is a closed-form function of time (seekable, deterministic and
//! allocation-free). An emitter always yields `count` sprites; dead ones have opacity 0, so the
//! render stack never changes inside an item.
use splice_model::{Rect, ShapeKind};
use std::f64::consts::TAU;

/// What the particles look like and how they move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preset {
    /// Paper rectangles tumbling down once, in a burst.
    Confetti,
    /// Twinkling dots in place, forever.
    Sparkle,
    /// Flakes drifting down, forever.
    Snow,
    /// Hearts floating up, forever.
    Hearts,
}

impl Preset {
    pub(crate) fn parse(name: &str) -> Self {
        match name {
            "sparkle" => Self::Sparkle,
            "snow" => Self::Snow,
            "hearts" => Self::Hearts,
            _ => Self::Confetti,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    /// One-shot: local frame it is born. Looping: phase offset in frames.
    offset: f64,
    /// Frames it lives (one period when looping).
    life: f64,
    origin: [f64; 2],
    /// Pixels per frame.
    velocity: [f64; 2],
    /// Horizontal sway: amplitude (px), angular speed (radians per frame), phase.
    sway: [f64; 3],
    size: [f64; 2],
    /// Degrees, and degrees per frame.
    spin: [f64; 2],
    color: u32,
}

/// A particle's state at one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sprite {
    /// Center in canvas pixels.
    pub(crate) center: [f64; 2],
    /// Width and height in canvas pixels.
    pub(crate) size: [f64; 2],
    /// Clockwise degrees.
    pub(crate) rotation: f64,
    /// 0 when the particle is not alive.
    pub(crate) opacity: f64,
    pub(crate) color: u32,
}

/// A prepared emitter.
pub(crate) struct Emitter {
    preset: Preset,
    particles: Vec<Particle>,
}

/// SplitMix64: tiny, seedable, the same everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.next()
    }

    fn pick(&mut self, palette: &[u32]) -> u32 {
        palette[((self.next() * palette.len() as f64) as usize).min(palette.len() - 1)]
    }
}

const CONFETTI: [u32; 8] = [
    0xff3b30, 0xffcc00, 0x34c759, 0x007aff, 0xaf52de, 0xff9500, 0xff2d55, 0x5ac8fa,
];
const SPARKLE: [u32; 3] = [0xffffff, 0xfff4c2, 0xffd54a];
const SNOW: [u32; 2] = [0xffffff, 0xeef6ff];
const HEARTS: [u32; 4] = [0xff2d55, 0xff375f, 0xff6b81, 0xe0245e];

impl Emitter {
    /// `count` particles of `preset` in `bounds` (canvas pixels) over `length` frames; `size`
    /// is the particle size in pixels.
    pub(crate) fn new(
        preset: Preset,
        count: usize,
        seed: u64,
        size: f64,
        bounds: Rect,
        length: f64,
        fps: f64,
    ) -> Self {
        let mut rng = Rng(seed ^ 0x5EED_0000_0000_0000);
        let Rect { x, y, w, h } = bounds;
        let s = size;
        let particles = (0..count)
            .map(|_| match preset {
                Preset::Confetti => {
                    let birth = rng.range(0.0, 0.35 * length);
                    let life = (length - birth).max(1.0);
                    Particle {
                        offset: birth,
                        life,
                        origin: [x + rng.next() * w, y - rng.range(0.0, 0.15) * h - s],
                        velocity: [rng.range(-0.25, 0.25) * s, {
                            (h + 2.0 * s) / (life * rng.range(0.75, 1.0))
                        }],
                        sway: [
                            s * rng.range(1.0, 3.0),
                            TAU / (fps * rng.range(0.8, 1.6)),
                            { rng.range(0.0, TAU) },
                        ],
                        size: [s * rng.range(0.8, 1.2), s * rng.range(0.45, 0.6)],
                        spin: [rng.range(0.0, 360.0), rng.range(-12.0, 12.0)],
                        color: rng.pick(&CONFETTI),
                    }
                }
                Preset::Sparkle => {
                    let life = fps * rng.range(0.4, 0.9);
                    let side = s * rng.range(0.5, 1.2);
                    Particle {
                        offset: rng.range(0.0, life),
                        life,
                        origin: [x + rng.next() * w, y + rng.next() * h],
                        velocity: [0.0, -s * 0.02],
                        sway: [0.0; 3],
                        size: [side, side],
                        spin: [0.0, 0.0],
                        color: rng.pick(&SPARKLE),
                    }
                }
                Preset::Snow => {
                    let life = fps * rng.range(3.0, 6.0);
                    let side = s * rng.range(0.5, 1.1);
                    Particle {
                        offset: rng.range(0.0, life),
                        life,
                        origin: [x + rng.next() * w, y - side],
                        velocity: [0.0, (h + 2.0 * side) / life],
                        sway: [
                            s * rng.range(1.0, 3.0),
                            TAU / (fps * rng.range(2.0, 4.0)),
                            { rng.range(0.0, TAU) },
                        ],
                        size: [side, side],
                        spin: [0.0, 0.0],
                        color: rng.pick(&SNOW),
                    }
                }
                Preset::Hearts => {
                    let life = fps * rng.range(2.0, 3.5);
                    let side = s * rng.range(1.2, 1.8);
                    Particle {
                        offset: rng.range(0.0, life),
                        life,
                        origin: [x + rng.next() * w, y + h + side],
                        velocity: [0.0, -(h * rng.range(0.6, 1.0)) / life],
                        sway: [
                            s * rng.range(1.0, 2.5),
                            TAU / (fps * rng.range(1.5, 3.0)),
                            { rng.range(0.0, TAU) },
                        ],
                        size: [side, side],
                        spin: [rng.range(-15.0, 15.0), 0.0],
                        color: rng.pick(&HEARTS),
                    }
                }
            })
            .collect();
        Self { preset, particles }
    }

    pub(crate) fn len(&self) -> usize {
        self.particles.len()
    }

    /// What every particle draws as.
    pub(crate) fn kind(&self) -> ShapeKind {
        match self.preset {
            Preset::Confetti => ShapeKind::Rect,
            Preset::Sparkle | Preset::Snow => ShapeKind::Ellipse,
            Preset::Hearts => ShapeKind::Heart,
        }
    }

    /// Particle `index` at local frame `t`.
    pub(crate) fn sprite(&self, index: usize, t: f64) -> Sprite {
        let p = &self.particles[index];
        let age = match self.preset {
            Preset::Confetti => t - p.offset,
            _ => (t + p.offset).rem_euclid(p.life),
        };
        let progress = age / p.life;
        let mut opacity = match self.preset {
            Preset::Confetti => ((1.0 - progress) / 0.1).min(1.0),
            Preset::Sparkle => (std::f64::consts::PI * progress).sin(),
            Preset::Snow => (progress / 0.1).min((1.0 - progress) / 0.1).min(1.0),
            Preset::Hearts => (progress / 0.15).min((1.0 - progress) / 0.3).min(1.0),
        };
        if !(0.0..1.0).contains(&progress) {
            opacity = 0.0;
        }
        let opacity = opacity.clamp(0.0, 1.0);
        let scale = match self.preset {
            Preset::Sparkle => 0.3 + 0.7 * opacity,
            _ => 1.0,
        };
        let [amplitude, speed, phase] = p.sway;
        Sprite {
            center: [
                p.origin[0] + p.velocity[0] * age + amplitude * (speed * age + phase).sin(),
                p.origin[1] + p.velocity[1] * age,
            ],
            size: [p.size[0] * scale, p.size[1] * scale],
            rotation: p.spin[0] + p.spin[1] * age,
            opacity,
            color: p.color,
        }
    }
}
