//! Keyframes for any numeric effect parameter (PLAN.md D5, T16.1). Frames are relative to the
//! owner's start (clip or item), or absolute for lane and project effects. Sampling is a binary
//! search plus one curve solve: no allocation.
use crate::Frame;

/// Interpolation from a key to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ease {
    /// Straight line.
    #[default]
    Linear,
    /// Hold the value until the next key.
    Hold,
    /// Slow start.
    In,
    /// Slow end.
    Out,
    /// Slow start and end.
    InOut,
    /// CSS-style cubic Bezier with control points (x1, y1) and (x2, y2).
    Bezier([f64; 4]),
    /// Overshoots the next value by about 10% and settles back (GSAP `back.out(1.7)`).
    Back,
    /// Springs past the next value and rings down onto it (GSAP `elastic.out(1, 0.3)`).
    Elastic,
    /// Lands on the next value like a dropped ball (GSAP `bounce.out`).
    Bounce,
    /// `n` equal jumps, each at the end of its step (CSS `steps(n)`).
    Steps(u32),
    /// A perceptual spring from this key's value toward the next one, in closed form so any
    /// frame can be sampled directly (PLAN.md D20). `duration` is the perceptual duration in
    /// frames (the period of the undamped spring); `bounce` 0 is critically damped, above 0
    /// overshoots, below 0 is overdamped. The spring runs in frames from the key, independent
    /// of the gap to the next key, which takes over at its frame: leave at least `settle()`.
    Spring {
        /// Perceptual duration in frames.
        duration: f64,
        /// −1 < bounce < 1.
        bounce: f64,
    },
}

/// Accepted `Ease::Steps` counts.
pub const STEPS: [u32; 2] = [1, 100];
/// Accepted spring durations in frames.
pub const SPRING_DURATION: [f64; 2] = [2.0, 600.0];
/// Accepted spring bounce.
pub const SPRING_BOUNCE: [f64; 2] = [-0.9, 0.9];
/// A spring has settled once it stays within this fraction of the distance to its target.
const SETTLED: f64 = 0.001;

/// One keyframe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    /// Frame relative to the owner.
    pub at: Frame,
    /// Value at the frame.
    pub value: f64,
    /// Curve toward the next key.
    pub ease: Ease,
}

impl Ease {
    /// Wire name, or `None` for a Bezier.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            Self::Linear => "linear",
            Self::Hold => "hold",
            Self::In => "in",
            Self::Out => "out",
            Self::InOut => "inOut",
            Self::Back => "back",
            Self::Elastic => "elastic",
            Self::Bounce => "bounce",
            Self::Bezier(_) | Self::Steps(_) | Self::Spring { .. } => return None,
        })
    }

    /// Parses a wire name.
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "linear" => Self::Linear,
            "hold" => Self::Hold,
            "in" => Self::In,
            "out" => Self::Out,
            "inOut" => Self::InOut,
            "back" => Self::Back,
            "elastic" => Self::Elastic,
            "bounce" => Self::Bounce,
            _ => return None,
        })
    }

    /// Maps progress 0…1 of a segment `span` frames long through the curve. Overshooting
    /// eases (back, elastic, spring) leave 0…1 on the way.
    pub fn apply(self, t: f64, span: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::In => bezier([0.42, 0.0, 1.0, 1.0], t),
            Self::Out => bezier([0.0, 0.0, 0.58, 1.0], t),
            Self::InOut => bezier([0.42, 0.0, 0.58, 1.0], t),
            Self::Bezier(points) => bezier(points, t),
            Self::Back => {
                let (c1, u) = (1.70158, t - 1.0);
                1.0 + (c1 + 1.0) * u * u * u + c1 * u * u
            }
            Self::Elastic => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    let c4 = std::f64::consts::TAU / 3.0;
                    (-10.0 * t).exp2() * ((t * 10.0 - 0.75) * c4).sin() + 1.0
                }
            }
            Self::Bounce => bounce_out(t),
            Self::Steps(n) => {
                let n = f64::from(n.max(1));
                (t * n).floor() / n
            }
            Self::Spring { duration, bounce } => {
                let [omega, zeta] = spring(duration, bounce);
                1.0 - spring_offset(omega, zeta, t * span.max(0.0))
            }
        }
    }

    /// Frames until a spring stays within 0.1% of its target, or `None` for other eases (they
    /// end exactly at the next key).
    pub fn settle(self) -> Option<f64> {
        let Self::Spring { duration, bounce } = self else {
            return None;
        };
        let [omega, zeta] = spring(duration, bounce);
        let mut hi = duration.max(1.0);
        while spring_envelope(omega, zeta, hi) > SETTLED && hi < 1e7 {
            hi *= 2.0;
        }
        let mut lo = 0.0;
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if spring_envelope(omega, zeta, mid) > SETTLED {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(hi)
    }
}

/// Angular frequency (radians per frame) and damping ratio of a perceptual spring, as SwiftUI
/// and Motion define them: ω₀ = 2π / duration, ζ = 1 − bounce (bounce ≥ 0) or 1 / (1 + bounce).
fn spring(duration: f64, bounce: f64) -> [f64; 2] {
    let duration = duration.max(1e-3);
    let bounce = bounce.clamp(SPRING_BOUNCE[0], SPRING_BOUNCE[1]);
    let zeta = if bounce >= 0.0 {
        1.0 - bounce
    } else {
        1.0 / (1.0 + bounce)
    };
    [std::f64::consts::TAU / duration, zeta]
}

/// Remaining distance to the target (1 at rest at τ = 0) of a unit step response starting at
/// rest: the closed form of x″ + 2ζω₀x′ + ω₀²x = 0 with x(0) = 1, x′(0) = 0.
fn spring_offset(omega: f64, zeta: f64, tau: f64) -> f64 {
    if (zeta - 1.0).abs() < 1e-9 {
        (1.0 + omega * tau) * (-omega * tau).exp()
    } else if zeta < 1.0 {
        let damped = omega * (1.0 - zeta * zeta).sqrt();
        (-zeta * omega * tau).exp()
            * ((damped * tau).cos() + zeta * omega / damped * (damped * tau).sin())
    } else {
        let root = (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-omega * (zeta - root), -omega * (zeta + root));
        (r2 * (r1 * tau).exp() - r1 * (r2 * tau).exp()) / (r2 - r1)
    }
}

/// A monotone bound on `|spring_offset|` from τ on.
fn spring_envelope(omega: f64, zeta: f64, tau: f64) -> f64 {
    if (zeta - 1.0).abs() < 1e-9 {
        (1.0 + omega * tau) * (-omega * tau).exp()
    } else if zeta < 1.0 {
        (-zeta * omega * tau).exp() / (1.0 - zeta * zeta).sqrt()
    } else {
        let root = (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-omega * (zeta - root), -omega * (zeta + root));
        (r2.abs() * (r1 * tau).exp() + r1.abs() * (r2 * tau).exp()) / (r1 - r2).abs()
    }
}

/// easings.net `easeOutBounce`.
fn bounce_out(t: f64) -> f64 {
    let (n, d) = (7.5625, 2.75);
    if t < 1.0 / d {
        n * t * t
    } else if t < 2.0 / d {
        let t = t - 1.5 / d;
        n * t * t + 0.75
    } else if t < 2.5 / d {
        let t = t - 2.25 / d;
        n * t * t + 0.9375
    } else {
        let t = t - 2.625 / d;
        n * t * t + 0.984375
    }
}

/// Cubic Bezier y at x = t: Newton steps on x(s), bisection when the slope flattens.
fn bezier([x1, y1, x2, y2]: [f64; 4], t: f64) -> f64 {
    let curve = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    let slope = |a: f64, b: f64, s: f64| {
        let u = 1.0 - s;
        3.0 * u * u * a + 6.0 * u * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    let mut s = t;
    for _ in 0..8 {
        let error = curve(x1, x2, s) - t;
        if error.abs() < 1e-7 {
            return curve(y1, y2, s);
        }
        let d = slope(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s = (s - error / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    s = t;
    for _ in 0..40 {
        let x = curve(x1, x2, s);
        if (x - t).abs() < 1e-7 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    curve(y1, y2, s)
}

/// Value of sorted keys at a (fractional) frame. Holds the first and last values outside them.
pub fn sample(keys: &[Key], at: f64) -> f64 {
    let Some(first) = keys.first() else {
        return 0.0;
    };
    let index = keys.partition_point(|key| f64::from(key.at) <= at);
    if index == 0 {
        return first.value;
    }
    let a = keys[index - 1];
    let Some(b) = keys.get(index) else {
        return a.value;
    };
    let span = f64::from(b.at - a.at);
    let t = (at - f64::from(a.at)) / span;
    a.value + (b.value - a.value) * a.ease.apply(t, span)
}

/// Integral of the keyed value from frame 0 to `to` (the source frames a speed curve consumes),
/// in 1/4-frame trapezoids per linear frame. Allocation-free.
pub fn integrate(keys: &[Key], to: f64) -> f64 {
    if to <= 0.0 {
        return 0.0;
    }
    let steps = (to * 4.0).ceil().max(1.0);
    let dt = to / steps;
    let mut sum = 0.0;
    let mut previous = sample(keys, 0.0);
    let mut i = 1.0;
    while i <= steps {
        let next = sample(keys, i * dt);
        sum += (previous + next) * 0.5 * dt;
        previous = next;
        i += 1.0;
    }
    sum
}

/// Frames inside the keyed range where a linear-ramp renderer must cut to follow the curve:
/// every key, plus every `step` frames inside non-linear segments.
pub fn breakpoints(keys: &[Key], offset: Frame, step: Frame, out: &mut Vec<Frame>) {
    for pair in keys.windows(2) {
        out.push(offset + pair[0].at);
        match pair[0].ease {
            Ease::Linear => {}
            Ease::Hold => out.push(offset + pair[1].at),
            // Each jump lands on the frame where floor(t·n) changes.
            Ease::Steps(n) => {
                let span = i64::from(pair[1].at - pair[0].at);
                let n = i64::from(n.max(1));
                out.extend((1..n).map(|k| offset + pair[0].at + ((k * span + n - 1) / n) as Frame));
            }
            _ => out.extend(
                (pair[0].at..pair[1].at)
                    .step_by(step.max(1) as usize)
                    .map(|at| offset + at),
            ),
        }
    }
    if let Some(last) = keys.last() {
        out.push(offset + last.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(at: Frame, value: f64, ease: Ease) -> Key {
        Key { at, value, ease }
    }

    #[test]
    fn keys_sample_each_ease_and_hit_keys_exactly() {
        let golden = [
            (
                Ease::Linear,
                [0.0, 12.5, 25.0, 37.5, 50.0, 62.5, 75.0, 87.5, 100.0],
            ),
            (Ease::Hold, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 100.0]),
            (
                Ease::In,
                [0.0, 2.6, 9.3, 19.2, 31.5, 45.9, 62.2, 80.1, 100.0],
            ),
            (
                Ease::Out,
                [0.0, 19.9, 37.8, 54.1, 68.5, 80.8, 90.7, 97.4, 100.0],
            ),
            (
                Ease::InOut,
                [0.0, 3.1, 12.9, 29.3, 50.0, 70.7, 87.1, 96.9, 100.0],
            ),
            (
                Ease::Bezier([0.25, 0.1, 0.25, 1.0]),
                [0.0, 13.7, 40.9, 64.5, 80.2, 90.1, 96.0, 99.1, 100.0],
            ),
        ];
        for (ease, values) in golden {
            let keys = [key(0, 0.0, ease), key(8, 100.0, Ease::Linear)];
            for (frame, expected) in values.into_iter().enumerate() {
                let got = sample(&keys, frame as f64);
                assert!((got - expected).abs() < 0.1, "{ease:?} at {frame}: {got}");
            }
        }
        let keys = [
            key(10, 1.0, Ease::InOut),
            key(20, 3.0, Ease::Hold),
            key(30, -2.0, Ease::Linear),
        ];
        assert_eq!(
            [0.0, 10.0, 20.0, 29.0, 30.0, 99.0].map(|f| sample(&keys, f)),
            [1.0, 1.0, 3.0, 3.0, -2.0, -2.0]
        );
    }

    #[test]
    fn integration_and_breakpoints_follow_the_curve() {
        let constant = [key(0, 2.0, Ease::Linear)];
        assert!((integrate(&constant, 30.0) - 60.0).abs() < 1e-9);
        let ramp = [key(0, 1.0, Ease::Linear), key(10, 3.0, Ease::Linear)];
        assert!((integrate(&ramp, 10.0) - 20.0).abs() < 1e-9);
        assert!((integrate(&ramp, 20.0) - 50.0).abs() < 1e-9);
        let mut cuts = Vec::new();
        breakpoints(
            &[
                key(0, 0.0, Ease::InOut),
                key(10, 1.0, Ease::Hold),
                key(12, 0.0, Ease::Linear),
            ],
            100,
            4,
            &mut cuts,
        );
        assert_eq!(cuts, [100, 100, 104, 108, 110, 112, 112]);
    }

    /// T42.1: the closed-form spring matches a fine numeric integration of the same ODE for
    /// underdamped, critically damped and overdamped bounce, so any frame seeks exactly.
    #[test]
    fn spring_is_closed_form_and_reports_its_settle_time() {
        for bounce in [0.5, 0.25, 0.0, -0.5] {
            let ease = Ease::Spring {
                duration: 30.0,
                bounce,
            };
            let [omega, zeta] = spring(30.0, bounce);
            // RK4 on x'' = -2ζωx' - ω²x from x = 1, v = 0, compared every frame.
            let (mut x, mut v, dt) = (1.0_f64, 0.0_f64, 1.0 / 64.0);
            let f = |x: f64, v: f64| (v, -2.0 * zeta * omega * v - omega * omega * x);
            for frame in 0..120 {
                let closed = 1.0 - ease.apply(f64::from(frame) / 120.0, 120.0);
                assert!((closed - x).abs() < 1e-6, "bounce {bounce} frame {frame}");
                for _ in 0..64 {
                    let (k1x, k1v) = f(x, v);
                    let (k2x, k2v) = f(x + k1x * dt / 2.0, v + k1v * dt / 2.0);
                    let (k3x, k3v) = f(x + k2x * dt / 2.0, v + k2v * dt / 2.0);
                    let (k4x, k4v) = f(x + k3x * dt, v + k3v * dt);
                    x += dt / 6.0 * (k1x + 2.0 * k2x + 2.0 * k3x + k4x);
                    v += dt / 6.0 * (k1v + 2.0 * k2v + 2.0 * k3v + k4v);
                }
            }
            let settle = ease.settle().unwrap();
            // Past the settle time the spring stays within 0.1%, and it did not just before.
            for step in 0..400 {
                let tau = settle + f64::from(step) * 0.5;
                assert!(spring_offset(omega, zeta, tau).abs() <= SETTLED * 1.0001);
            }
            assert!(spring_envelope(omega, zeta, settle * 0.95) > SETTLED);
        }
        let settle = |bounce| {
            Ease::Spring {
                duration: 30.0,
                bounce,
            }
            .settle()
            .unwrap()
        };
        // More bounce rings longer; overdamped creeps longer than critical.
        assert!(settle(0.5) > settle(0.25));
        assert!(settle(-0.5) > settle(0.0));
        assert!((settle(0.0) - 44.0).abs() < 1.0, "{}", settle(0.0));
        assert_eq!(Ease::InOut.settle(), None);
        // Bounce > 0 overshoots, bounce ≤ 0 never does.
        let peak = |bounce| {
            (0..120)
                .map(|f| {
                    Ease::Spring {
                        duration: 30.0,
                        bounce,
                    }
                    .apply(f64::from(f) / 120.0, 120.0)
                })
                .fold(f64::MIN, f64::max)
        };
        assert!(peak(0.3) > 1.04);
        assert!(peak(0.0) <= 1.0 && peak(-0.5) <= 1.0);
    }

    #[test]
    fn named_eases_hit_both_ends_and_overshoot_where_they_should() {
        for ease in [Ease::Back, Ease::Elastic, Ease::Bounce, Ease::Steps(4)] {
            assert!(ease.apply(0.0, 10.0).abs() < 1e-12, "{ease:?}");
            assert!((ease.apply(1.0, 10.0) - 1.0).abs() < 1e-12, "{ease:?}");
            assert_eq!(
                Ease::parse(ease.name().unwrap_or("x")).unwrap_or(ease),
                ease
            );
        }
        let max = |ease: Ease| {
            (0..=100)
                .map(|i| ease.apply(f64::from(i) / 100.0, 1.0))
                .fold(0.0, f64::max)
        };
        assert!(max(Ease::Back) > 1.09 && max(Ease::Back) < 1.11);
        assert!(max(Ease::Elastic) > 1.2);
        assert!(max(Ease::Bounce) <= 1.0);
        assert_eq!(
            [0.0, 0.24, 0.25, 0.5, 0.99].map(|t| Ease::Steps(4).apply(t, 1.0)),
            [0.0, 0.0, 0.25, 0.5, 0.75]
        );
        let mut cuts = Vec::new();
        breakpoints(
            &[key(0, 0.0, Ease::Steps(3)), key(10, 1.0, Ease::Linear)],
            0,
            2,
            &mut cuts,
        );
        assert_eq!(cuts, [0, 4, 7, 10]);
        let keys = [key(0, 0.0, Ease::Steps(3)), key(10, 3.0, Ease::Linear)];
        assert_eq!(
            [3.0, 4.0, 6.0, 7.0].map(|f| sample(&keys, f)),
            [0.0, 1.0, 1.0, 2.0]
        );
    }
}
