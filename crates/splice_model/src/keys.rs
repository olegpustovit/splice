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
}

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
            Self::Bezier(_) => return None,
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
            _ => return None,
        })
    }

    /// Maps progress 0…1 through the curve.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::In => bezier([0.42, 0.0, 1.0, 1.0], t),
            Self::Out => bezier([0.0, 0.0, 0.58, 1.0], t),
            Self::InOut => bezier([0.42, 0.0, 0.58, 1.0], t),
            Self::Bezier(points) => bezier(points, t),
        }
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
    let t = (at - f64::from(a.at)) / f64::from(b.at - a.at);
    a.value + (b.value - a.value) * a.ease.apply(t)
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
}
