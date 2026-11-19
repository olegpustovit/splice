//! The drawn cursor of a screen take (PLAN.md T39, research §cursor): one smoothed 60 Hz table
//! per take and treatment, built once at prepare from the recorded samples, so playback, seeking
//! and export read the same point with one lookup.
//!
//! Pipeline (Cap's, constants from its renderer): recorded samples are resampled onto the grid
//! (gaps over 67 ms hold instead of teleporting), hand shake is removed with a small rope
//! filter, and a "mellow" spring chases the path a little ahead of time (lead c/k) so it does not
//! lag. Around a click a snappier spring takes over and the cursor lands exactly on the click.
//! Jumps no hand makes (an automation warp) become a synthesized human path: a seeded cubic
//! Bezier with minimum-jerk progress and a Fitts-law duration, arriving before the press.
//! Idle cursors fade out after 2 s and fade back in just before they move; a click pulses
//! the cursor to 0.8×.
use crate::camera::step;

/// One recorded pointer sample on the take's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Source seconds.
    pub t: f64,
    /// Position normalized to the captured source (0–1).
    pub x: f64,
    /// Position normalized to the captured source (0–1).
    pub y: f64,
    /// Cursor image index, or -1 while the system cursor is hidden.
    pub image: i32,
    /// A button press.
    pub click: bool,
}

/// How a take's cursor moves and shows (the `cursor` effect).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// 0 draws the recorded path, 1 the full spring.
    pub smoothing: f64,
    /// Fade out after 2 s without movement.
    pub hide_idle: bool,
    /// Shrink to 0.8× at a press.
    pub click_pulse: bool,
    /// Replace warps (jumps no hand makes) with a synthesized path.
    pub synthesize: bool,
    /// Bend of a synthesized path, relative to its length.
    pub arc: f64,
    /// Seconds a synthesized path arrives before the press.
    pub arrive_before: f64,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            smoothing: 1.0,
            hide_idle: true,
            click_pulse: true,
            synthesize: true,
            arc: 0.15,
            arrive_before: 0.15,
        }
    }
}

/// The cursor at one instant.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Position normalized to the captured source.
    pub x: f64,
    /// Position normalized to the captured source.
    pub y: f64,
    /// Cursor image index, or -1 when hidden.
    pub image: i32,
    /// 0 hidden … 1 shown.
    pub opacity: f64,
    /// Click pulse scale (1 at rest).
    pub scale: f64,
}

/// A take's cursor table at `RATE`, plus its clicks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    start: f64,
    points: Vec<Point>,
    clicks: Vec<[f64; 3]>,
}

/// Table rate in points per second.
pub const RATE: f64 = 60.0;
/// A sample gap longer than this holds the last position (no interpolation across it).
const HOLD_GAP: f64 = 0.067;
/// Hand shake below this many UV is removed.
const SHAKE: f64 = 0.0035;
/// Mellow spring (stiffness, damping, mass): lead = damping / stiffness seconds.
const MELLOW: [f64; 3] = [470.0, 70.0, 3.0];
/// Click spring, used within `CLICK_WINDOW` of a press.
const SNAPPY: [f64; 3] = [530.0, 40.0, 1.0];
const CLICK_WINDOW: f64 = 0.175;
/// The cursor blends onto the press position over this many seconds each side.
const PRESS_AIM: f64 = 0.15;
const IDLE_AFTER: f64 = 2.0;
const FADE_OUT: f64 = 0.4;
const FADE_IN: f64 = 0.25;
const PULSE: f64 = 0.13;
/// A move faster than this (screen points per second) is a warp, not a hand.
const WARP_SPEED: f64 = 12_000.0;
/// A pointer silent this long that then appears far away was warped.
const WARP_SILENCE: f64 = 0.1;
/// Fitts target width in points when the real target is unknown.
const FITTS_WIDTH: f64 = 100.0;

impl Track {
    /// Builds the table for `samples` (sorted by time) captured from a `size` (points) source.
    pub fn build(samples: &[Sample], size: [f64; 2], motion: Motion) -> Self {
        let Some(first) = samples.first() else {
            return Self::default();
        };
        let path = if motion.synthesize {
            synthesize(samples, size, motion)
        } else {
            samples.iter().map(|&s| (s, false)).collect()
        };
        let last = path.last().map_or(first.t, |(s, _)| s.t);
        let start = first.t;
        let count = ((last - start) * RATE).ceil() as usize + 1;
        let clicks: Vec<[f64; 3]> = samples
            .iter()
            .filter(|s| s.click)
            .map(|s| [s.t, s.x, s.y])
            .collect();
        // Raw path on the grid, then the rope filter.
        let mut raw = Vec::with_capacity(count);
        let mut cursor = 0;
        for i in 0..count {
            let t = start + i as f64 / RATE;
            while cursor + 1 < path.len() && path[cursor + 1].0.t <= t {
                cursor += 1;
            }
            let (a, synthetic) = path[cursor];
            let mut p = [a.x, a.y];
            if let Some(&(b, _)) = path.get(cursor + 1)
                && b.t > a.t
                && (synthetic || b.t - a.t <= HOLD_GAP)
            {
                let k = ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0);
                p = [a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k];
            }
            raw.push((p, a.image, synthetic));
        }
        let mut rope = raw[0].0;
        let filtered: Vec<[f64; 2]> = raw
            .iter()
            .map(|&(p, _, synthetic)| {
                let (dx, dy) = (p[0] - rope[0], p[1] - rope[1]);
                let d = dx.hypot(dy);
                if synthetic {
                    rope = p;
                } else if d > SHAKE {
                    let k = (d - SHAKE) / d;
                    rope = [rope[0] + dx * k, rope[1] + dy * k];
                }
                rope
            })
            .collect();
        // The spring, sampled ahead by its own lag; synthesized stretches are already smooth.
        let lead = MELLOW[1] / MELLOW[0];
        let near_click = |t: f64| clicks.iter().any(|c| (t - c[0]).abs() <= CLICK_WINDOW);
        let dt = 1.0 / RATE / 4.0;
        let mut position = filtered[0];
        let mut velocity = [0.0, 0.0];
        let mut points = Vec::with_capacity(count);
        // The lead never looks into a synthesized stretch: the spring would leave the resting
        // point early and snap back when the stretch takes over.
        let mut next_synthetic = vec![count; count];
        for i in (0..count).rev() {
            next_synthetic[i] = if raw[i].2 {
                i
            } else {
                next_synthetic.get(i + 1).copied().unwrap_or(count)
            };
        }
        for i in 0..count {
            let t = start + i as f64 / RATE;
            let ahead = ((i as f64 + lead * RATE).round() as usize)
                .min(count - 1)
                .min(next_synthetic[i].saturating_sub(1).max(i));
            let target = filtered[ahead];
            if raw[i].2 {
                position = filtered[i];
                velocity = [0.0, 0.0];
            } else {
                let physics = if near_click(t) { SNAPPY } else { MELLOW };
                let goal = if near_click(t) { filtered[i] } else { target };
                for _ in 0..4 {
                    for axis in 0..2 {
                        (position[axis], velocity[axis]) =
                            step(position[axis], velocity[axis], goal[axis], dt, physics);
                    }
                }
            }
            let s = motion.smoothing.clamp(0.0, 1.0);
            let mut p = [
                filtered[i][0] + (position[0] - filtered[i][0]) * s,
                filtered[i][1] + (position[1] - filtered[i][1]) * s,
            ];
            // Land exactly on each press.
            for c in &clicks {
                // Flat for one table step each side, so lookups between points stay exact.
                let w = smoothstep(1.0 - ((t - c[0]).abs() - 1.0 / RATE).max(0.0) / PRESS_AIM);
                p = [p[0] + (c[1] - p[0]) * w, p[1] + (c[2] - p[1]) * w];
            }
            let image = raw[i].1;
            let mut scale = 1.0;
            if motion.click_pulse {
                for c in &clicks {
                    let age = t - c[0];
                    if (0.0..=PULSE).contains(&age) {
                        scale = 1.0 - 0.2 * (std::f64::consts::PI * age / PULSE).sin();
                    }
                }
            }
            points.push(Point {
                x: p[0].clamp(0.0, 1.0),
                y: p[1].clamp(0.0, 1.0),
                image,
                opacity: if image < 0 { 0.0 } else { 1.0 },
                scale,
            });
        }
        if motion.hide_idle {
            fade_idle(&mut points, &filtered, start, &clicks);
        }
        Self {
            start,
            points,
            clicks,
        }
    }

    /// The cursor at source second `t` (interpolated between table points), or `None` before
    /// the first sample. Allocation-free.
    pub fn at(&self, t: f64) -> Option<Point> {
        if self.points.is_empty() || t < self.start {
            return None;
        }
        let f = (t - self.start) * RATE;
        let i = (f.floor() as usize).min(self.points.len() - 1);
        let a = self.points[i];
        let Some(b) = self.points.get(i + 1) else {
            return Some(a);
        };
        let k = f - i as f64;
        let lerp = |a: f64, b: f64| a + (b - a) * k;
        Some(Point {
            x: lerp(a.x, b.x),
            y: lerp(a.y, b.y),
            image: a.image,
            opacity: lerp(a.opacity, b.opacity),
            scale: lerp(a.scale, b.scale),
        })
    }

    /// Presses as (source seconds, x, y).
    pub fn clicks(&self) -> &[[f64; 3]] {
        &self.clicks
    }

    /// Table points (tests and diagnostics).
    pub fn points(&self) -> &[Point] {
        &self.points
    }
}

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Hides the cursor after `IDLE_AFTER` seconds without movement or clicks (fading over
/// `FADE_OUT`) and shows it again `FADE_IN` before it next moves.
fn fade_idle(points: &mut [Point], path: &[[f64; 2]], start: f64, clicks: &[[f64; 3]]) {
    let n = points.len();
    let moving: Vec<bool> = (0..n)
        .map(|i| {
            let t = start + i as f64 / RATE;
            let moved =
                i > 0 && (path[i][0] - path[i - 1][0]).hypot(path[i][1] - path[i - 1][1]) > 1e-6;
            moved || clicks.iter().any(|c| (c[0] - t).abs() < 0.5 / RATE)
        })
        .collect();
    let mut last = 0.0;
    let mut next = vec![f64::INFINITY; n];
    let mut upcoming = f64::INFINITY;
    for i in (0..n).rev() {
        if moving[i] {
            upcoming = i as f64 / RATE;
        }
        next[i] = upcoming;
    }
    for i in 0..n {
        let t = i as f64 / RATE;
        if moving[i] || i == 0 {
            last = t;
        }
        let shown_after_last = 1.0 - smoothstep((t - last - IDLE_AFTER) / FADE_OUT);
        let shown_before_next = smoothstep(1.0 - (next[i] - t) / FADE_IN);
        points[i].opacity *= shown_after_last.max(shown_before_next);
    }
}

/// Splits warps into synthesized human paths; marks the synthesized samples.
fn synthesize(samples: &[Sample], size: [f64; 2], motion: Motion) -> Vec<(Sample, bool)> {
    let mut out: Vec<(Sample, bool)> = Vec::with_capacity(samples.len());
    for (i, &b) in samples.iter().enumerate() {
        if let Some(&a) = i.checked_sub(1).and_then(|j| samples.get(j)) {
            let distance = ((b.x - a.x) * size[0]).hypot((b.y - a.y) * size[1]);
            let gap = b.t - a.t;
            // A hand reports motion every few ms; silence followed by a far position, or a
            // speed no hand reaches, is a warp.
            let warp = distance > 30.0
                && (gap <= 0.0 || gap > WARP_SILENCE || distance / gap > WARP_SPEED);
            if warp {
                path(&mut out, a, b, distance, size, motion);
            }
        }
        out.push((b, false));
    }
    out
}

/// A human-looking move from `a` to `b`: Fitts duration, arriving `arrive_before` the press,
/// a seeded perpendicular bend and minimum-jerk progress.
fn path(
    out: &mut Vec<(Sample, bool)>,
    a: Sample,
    b: Sample,
    distance: f64,
    size: [f64; 2],
    motion: Motion,
) {
    let duration = (0.2 + 0.1 * (distance / FITTS_WIDTH + 1.0).log2()).clamp(0.25, 1.0);
    // Arrive early when there is time; a tight gap keeps most of itself for the move.
    let end = b.t - motion.arrive_before.max(0.0).min((b.t - a.t) * 0.25);
    // Leave from `a` no earlier than it was recorded; if there is no room the path still runs
    // (the press waits for nothing: it is on a recorded clock).
    let begin = (end - duration).max(a.t);
    let (begin, end) = if end - begin < 0.05 {
        (a.t.max(b.t - duration), b.t)
    } else {
        (begin, end)
    };
    // Seeded side and spread: the same take renders the same path.
    let mut seed =
        (a.t.to_bits() ^ b.x.to_bits().rotate_left(17) ^ b.y.to_bits().rotate_left(31)) | 1;
    let mut random = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let side = if random() < 0.5 { -1.0 } else { 1.0 };
    let spread = motion.arc.max(0.0) * distance.clamp(2.0, 200.0);
    let (dx, dy) = ((b.x - a.x) * size[0], (b.y - a.y) * size[1]);
    let normal = [-dy / distance, dx / distance];
    let bend = |fraction: f64, amount: f64| {
        [
            a.x + (b.x - a.x) * fraction + normal[0] * amount * side / size[0],
            a.y + (b.y - a.y) * fraction + normal[1] * amount * side / size[1],
        ]
    };
    let c1 = bend(0.3, spread * (0.8 + 0.4 * random()));
    let c2 = bend(0.7, spread * (0.4 + 0.4 * random()));
    // Hold at `a` until the move starts.
    out.push((Sample { t: begin, ..a }, true));
    let steps = ((end - begin) * RATE).ceil().max(1.0) as usize;
    for k in 1..=steps {
        let u = k as f64 / steps as f64;
        let s = u * u * u * (10.0 - 15.0 * u + 6.0 * u * u);
        let r = 1.0 - s;
        let bezier = |p0: f64, p1: f64, p2: f64, p3: f64| {
            r * r * r * p0 + 3.0 * r * r * s * p1 + 3.0 * r * s * s * p2 + s * s * s * p3
        };
        out.push((
            Sample {
                t: begin + (end - begin) * u,
                x: bezier(a.x, c1[0], c2[0], b.x),
                y: bezier(a.y, c1[1], c2[1], b.y),
                image: a.image,
                click: false,
            },
            true,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [f64; 2] = [1440.0, 900.0];

    fn sample(t: f64, x: f64, y: f64, click: bool) -> Sample {
        Sample {
            t,
            x,
            y,
            image: 0,
            click,
        }
    }

    /// Largest step between consecutive table points, in screen points.
    fn largest_step(track: &Track) -> f64 {
        track
            .points()
            .windows(2)
            .map(|w| ((w[1].x - w[0].x) * SIZE[0]).hypot((w[1].y - w[0].y) * SIZE[1]))
            .fold(0.0, f64::max)
    }

    #[test]
    fn h12_a_gap_or_warp_never_teleports_and_lands_before_the_press() {
        // Rests at the left, then an automation warp straight onto a click at the right.
        let samples = [
            sample(0.0, 0.1, 0.5, false),
            sample(1.0, 0.1, 0.5, false),
            sample(1.6, 0.9, 0.5, true),
            sample(2.5, 0.9, 0.5, false),
        ];
        let track = Track::build(&samples, SIZE, Motion::default());
        // 1152 points in about 0.4 s: a fast human move, never a jump.
        assert!(largest_step(&track) < 120.0, "{}", largest_step(&track));
        let at = |t: f64| track.at(t).unwrap();
        let landed = at(1.6 - 0.1);
        assert!(
            (landed.x - 0.9).abs() < 0.01,
            "arrives before the press: {landed:?}"
        );
        assert!((at(1.6).x - 0.9).abs() < 1e-9 && (at(1.6).y - 0.5).abs() < 1e-9);
        // Without synthesis a > 67 ms gap holds, and the spring travels instead of jumping.
        let plain = Track::build(
            &samples,
            SIZE,
            Motion {
                synthesize: false,
                ..Motion::default()
            },
        );
        let before = plain.at(1.4).unwrap();
        assert!((before.x - 0.1).abs() < 0.02, "held: {before:?}");
    }

    #[test]
    fn shake_is_removed_and_slow_moves_survive() {
        let mut samples = Vec::new();
        for i in 0..240 {
            let t = i as f64 / 120.0;
            let wobble = if i % 2 == 0 { 0.002 } else { -0.002 };
            samples.push(sample(t, 0.5 + wobble, 0.5 - wobble, false));
        }
        let still = Track::build(&samples, SIZE, Motion::default());
        let spread = still
            .points()
            .iter()
            .map(|p| (p.x - 0.5).abs().max((p.y - 0.5).abs()))
            .fold(0.0, f64::max);
        assert!(spread < 0.0025, "shake {spread}");
        let slow: Vec<Sample> = (0..240)
            .map(|i| sample(i as f64 / 120.0, 0.2 + 0.1 * i as f64 / 240.0, 0.5, false))
            .collect();
        let moved = Track::build(&slow, SIZE, Motion::default());
        assert!(moved.at(2.0).unwrap().x > 0.29, "slow drag kept");
    }

    #[test]
    fn idle_cursors_fade_out_and_return_before_they_move() {
        let samples = [
            sample(0.0, 0.3, 0.3, false),
            sample(0.1, 0.32, 0.3, false),
            sample(6.0, 0.34, 0.3, false),
            sample(6.05, 0.36, 0.3, false),
        ];
        let track = Track::build(&samples, SIZE, Motion::default());
        let opacity = |t: f64| track.at(t).unwrap().opacity;
        assert!(opacity(1.0) > 0.99);
        assert!(opacity(3.0) < 0.01, "hidden after 2 s idle");
        assert!(
            opacity(6.0) > 0.99 && opacity(5.9) > 0.2,
            "back before it moves"
        );
        let kept = Track::build(
            &samples,
            SIZE,
            Motion {
                hide_idle: false,
                ..Motion::default()
            },
        );
        assert!(kept.at(3.0).unwrap().opacity > 0.99);
    }

    #[test]
    fn a_click_pulses_to_four_fifths() {
        let samples = [
            sample(0.0, 0.5, 0.5, false),
            sample(0.5, 0.5, 0.5, true),
            sample(1.0, 0.5, 0.5, false),
        ];
        let track = Track::build(&samples, SIZE, Motion::default());
        let smallest = (0..20)
            .map(|i| track.at(0.5 + i as f64 * 0.01).unwrap().scale)
            .fold(1.0, f64::min);
        assert!((smallest - 0.8).abs() < 0.02, "{smallest}");
        assert_eq!(track.at(0.8).unwrap().scale, 1.0);
    }

    /// T39.3: a thousand random anchor sets give deterministic, continuous, speed-capped paths
    /// that land on every press.
    #[test]
    fn synthesized_paths_pass_human_heuristics() {
        let mut seed = 0x2545_f491_u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            f64::from(seed) / f64::from(u32::MAX)
        };
        for _ in 0..1000 {
            let mut samples = vec![sample(0.0, next(), next(), false)];
            let mut t = 0.0;
            for _ in 0..4 {
                t += 0.6 + next() * 1.5;
                samples.push(sample(t, next(), next(), true));
            }
            let track = Track::build(&samples, SIZE, Motion::default());
            assert_eq!(track, Track::build(&samples, SIZE, Motion::default()));
            // Under 7,200 points per second (a fast flick), and no frame-to-frame jump.
            assert!(largest_step(&track) < 120.0, "{samples:?}");
            for click in &samples[1..] {
                let at = track.at(click.t).unwrap();
                assert!((at.x - click.x).abs() < 1e-9 && (at.y - click.y).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn lookups_allocate_nothing() {
        let samples: Vec<Sample> = (0..600)
            .map(|i| {
                sample(
                    i as f64 / 60.0,
                    (i as f64 / 600.0).sin().abs(),
                    0.5,
                    i % 90 == 0,
                )
            })
            .collect();
        let track = Track::build(&samples, SIZE, Motion::default());
        let mut sum = 0.0;
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                for i in 0..1000 {
                    sum += track.at(i as f64 * 0.01).map_or(0.0, |p| p.x);
                }
            },
        );
        assert!(sum > 0.0);
    }
}
