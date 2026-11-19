//! The timeline camera (PLAN.md D12): zoom segments set step targets, and one analytic
//! spring per channel chases them. Precomputed once per frame at prepare, so seeking and
//! playback read the same values and a frame costs one lookup.
use splice_model::{CameraMode, Frame, Spring};

/// The camera at one frame: magnification and center in travel space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    /// Magnification, at least 1.
    pub amount: f64,
    /// Travel-space center: (0, 0) frames the top-left corner, (1, 1) the bottom-right.
    pub center: [f64; 2],
    /// What zooms.
    pub mode: CameraMode,
}

impl View {
    /// No zoom.
    pub const IDENTITY: Self = Self {
        amount: 1.0,
        center: [0.5, 0.5],
        mode: CameraMode::Auto,
    };

    /// The visible part of the content, normalized: always inside [0, 1]².
    pub fn viewport(&self) -> [f64; 4] {
        let size = 1.0 / self.amount.max(1.0);
        let travel = 1.0 - size;
        [
            self.center[0].clamp(0.0, 1.0) * travel,
            self.center[1].clamp(0.0, 1.0) * travel,
            size,
            size,
        ]
    }

    /// The travel-space center that puts content point `p` (normalized) in the middle of the
    /// viewport at `amount`, clamped so the viewport stays inside the content.
    pub fn center_on(p: [f64; 2], amount: f64) -> [f64; 2] {
        let size = 1.0 / amount.max(1.0);
        let travel = 1.0 - size;
        p.map(|v| {
            if travel <= 1e-9 {
                0.5
            } else {
                ((v - size / 2.0) / travel).clamp(0.0, 1.0)
            }
        })
    }
}

/// One segment as the solver sees it: frames `[start, end)`, its target and spring.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Segment {
    pub start: Frame,
    pub end: Frame,
    pub amount: f64,
    pub center: Option<[f64; 2]>,
    pub mode: CameraMode,
    pub spring: Spring,
}

/// Physics substeps per second: retargets carry velocity, and finer steps keep the analytic
/// solution exact for targets that change mid-frame.
const STEPS_PER_SECOND: f64 = 240.0;
/// Below this magnification the center is invisible, so it snaps to the target (pre-aim).
const PRE_AIM: f64 = 1.0005;

/// Solves the camera for frames `0..end`. Segments are sorted by start; where they overlap the
/// later one wins. Outside segments the target is 1× with the last center held.
pub(crate) fn solve(segments: &[Segment], end: Frame, fps: u32) -> Vec<View> {
    let frames = usize::try_from(end.max(0)).unwrap_or(0);
    let mut out = Vec::with_capacity(frames);
    let dt_frame = 1.0 / f64::from(fps.max(1));
    let substeps = (STEPS_PER_SECOND * dt_frame).ceil().max(1.0) as u32;
    let dt = dt_frame / f64::from(substeps);
    let (mut amount, mut amount_v) = (1.0, 0.0);
    let (mut center, mut center_v) = ([0.5, 0.5], [0.0, 0.0]);
    let mut held = [0.5, 0.5];
    let mut active = 0usize;
    for frame in 0..end {
        while active < segments.len() && segments[active].end <= frame {
            active += 1;
        }
        // The latest-starting segment containing this frame.
        let current = segments[active..]
            .iter()
            .take_while(|s| s.start <= frame)
            .filter(|s| s.end > frame)
            .last();
        let (target, spring, mode) = match current {
            Some(s) => {
                let c = s.center.unwrap_or(held);
                held = c;
                (s.amount.max(1.0), s.spring, s.mode)
            }
            None => (
                1.0,
                segments
                    .get(active.saturating_sub(1))
                    .map_or(Spring::Focused, |s| s.spring),
                CameraMode::Auto,
            ),
        };
        match spring.physics() {
            None => {
                (amount, amount_v) = (target, 0.0);
                (center, center_v) = (held, [0.0, 0.0]);
            }
            Some(physics) => {
                for _ in 0..substeps {
                    // Pre-aim: while unzoomed at the start of the step the center is invisible,
                    // so it lands on the target before the zoom leaves 1×.
                    if amount <= PRE_AIM {
                        (center, center_v) = (held, [0.0, 0.0]);
                    } else {
                        for axis in 0..2 {
                            (center[axis], center_v[axis]) =
                                step(center[axis], center_v[axis], held[axis], dt, physics);
                        }
                    }
                    (amount, amount_v) = step(amount, amount_v, target, dt, physics);
                    // Bounce-out guard: never below 1× (that would show what is behind the content).
                    if amount < 1.0 {
                        (amount, amount_v) = (1.0, 0.0);
                    }
                }
            }
        }
        out.push(View {
            amount,
            center: center.map(|c| c.clamp(0.0, 1.0)),
            mode,
        });
    }
    out
}

/// Exact step of a damped spring `m·x'' + c·x' + k·(x − target) = 0` over `dt`.
pub(crate) fn step(x: f64, v: f64, target: f64, dt: f64, [k, c, m]: [f64; 3]) -> (f64, f64) {
    let omega = (k / m).sqrt();
    let zeta = c / (2.0 * (k * m).sqrt());
    let x0 = x - target;
    if (zeta - 1.0).abs() < 1e-6 {
        // Critically damped.
        let b = v + omega * x0;
        let e = (-omega * dt).exp();
        let xt = (x0 + b * dt) * e;
        let vt = (b - omega * (x0 + b * dt)) * e;
        (target + xt, vt)
    } else if zeta < 1.0 {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        let a = x0;
        let b = (v + zeta * omega * x0) / wd;
        let e = (-zeta * omega * dt).exp();
        let (s, co) = (wd * dt).sin_cos();
        let xt = e * (a * co + b * s);
        let vt = e * ((b * wd - zeta * omega * a) * co - (a * wd + zeta * omega * b) * s);
        (target + xt, vt)
    } else {
        let r = omega * (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-zeta * omega + r, -zeta * omega - r);
        let c2 = (v - r1 * x0) / (r2 - r1);
        let c1 = x0 - c2;
        let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
        (target + c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start: Frame, end: Frame, amount: f64, center: [f64; 2]) -> Segment {
        Segment {
            start,
            end,
            amount,
            center: Some(center),
            mode: CameraMode::Auto,
            spring: Spring::Focused,
        }
    }

    #[test]
    fn the_focused_spring_settles_like_caps_and_never_snaps_back() {
        let views = solve(&[segment(30, 90, 2.0, [0.5, 0.5])], 180, 60);
        // 90% of the way in about 0.32 s (19 frames at 60 fps) after the segment starts.
        let reached = views.iter().position(|v| v.amount >= 1.9).unwrap();
        assert!((44..=54).contains(&reached), "90% at frame {reached}");
        // Holds while the segment lasts, then springs back to 1× (no jump at the end).
        assert!((views[89].amount - 2.0).abs() < 0.01);
        for pair in views.windows(2) {
            assert!(
                (pair[1].amount - pair[0].amount).abs() < 0.08,
                "jump {pair:?}"
            );
            assert!(pair[1].amount >= 1.0);
        }
        assert!((views[179].amount - 1.0).abs() < 0.005);
    }

    #[test]
    fn the_viewport_stays_inside_the_content_for_any_framing() {
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            f64::from(seed) / f64::from(u32::MAX)
        };
        let mut segments = Vec::new();
        let mut at = 0;
        for _ in 0..400 {
            let length = 6 + (next() * 90.0) as Frame;
            segments.push(segment(
                at,
                at + length,
                1.0 + next() * 4.0,
                [next() * 1.4 - 0.2, next() * 1.4 - 0.2],
            ));
            at += length + (next() * 20.0) as Frame;
        }
        for view in solve(&segments, at, 30) {
            let [x, y, w, h] = view.viewport();
            assert!(
                x >= -1e-9 && y >= -1e-9 && x + w <= 1.0 + 1e-9 && y + h <= 1.0 + 1e-9,
                "{view:?}"
            );
        }
    }

    #[test]
    fn center_on_puts_a_point_in_the_middle_unless_that_leaves_the_content() {
        assert_eq!(View::center_on([0.5, 0.5], 2.0), [0.5, 0.5]);
        let view = View {
            amount: 2.0,
            center: View::center_on([0.4, 0.6], 2.0),
            mode: CameraMode::Auto,
        };
        let [x, y, w, h] = view.viewport();
        assert!(((x + w / 2.0) - 0.4).abs() < 1e-9 && ((y + h / 2.0) - 0.6).abs() < 1e-9);
        // A corner point clamps to the corner framing.
        assert_eq!(View::center_on([0.0, 1.0], 3.0), [0.0, 1.0]);
    }

    #[test]
    fn pre_aim_snaps_the_center_while_unzoomed() {
        let views = solve(&[segment(10, 40, 2.0, [1.0, 0.0])], 60, 30);
        // The first zoomed frame is already aimed at the target corner.
        let first = views.iter().find(|v| v.amount > 1.01).unwrap();
        assert!(
            first.center[0] > 0.95 && first.center[1] < 0.05,
            "{first:?}"
        );
    }
}
