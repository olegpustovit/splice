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

/// Recorded samples with declared pointer marks (T39.4) merged in time order. A mark keeps the
/// cursor image in use at its time; the synthesized path then carries the cursor to it.
pub fn with_marks(samples: &[Sample], marks: &[splice_model::PointerMark]) -> Vec<Sample> {
    let mut merged = Vec::with_capacity(samples.len() + marks.len());
    merged.extend_from_slice(samples);
    for mark in marks {
        let at = merged.partition_point(|s| s.t <= mark.t);
        let image = at
            .checked_sub(1)
            .or((!merged.is_empty()).then_some(0))
            .map_or(0, |i| merged[i].image);
        merged.insert(
            at,
            Sample {
                t: mark.t,
                x: mark.x,
                y: mark.y,
                image,
                click: mark.click,
            },
        );
    }
    merged
}

/// A take's recorded pointer for the evaluator (T46.2): the camera follows the drawn cursor,
/// so `Evaluator::prepare_with` builds the same table the renderer draws from these samples.
#[derive(Clone, Debug, PartialEq)]
pub struct Recorded {
    /// The take.
    pub media: splice_model::MediaId,
    /// Captured source width and height in screen points.
    pub size: [f64; 2],
    /// Samples on the take's clock, in time order (pointer marks are merged in at prepare).
    pub samples: Vec<Sample>,
}

/// How a take's cursor moves and shows (the `cursor` effect).
///
/// `smoothing` blends the shake-filtered recorded path (0) with the mellow spring (1, which leads
/// by its own lag so it never trails the hand). Against
/// Recorder's presets on the same path, lag aside (`smoothing_matches_recorder_presets`): `none` is 0,
/// `rapid` ≈ 0.1, `medium` ≈ 0.3 and `smooth` ≈ 0.75; 1 is softer than `smooth`.
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
    /// Draw the system arrow whatever shape was recorded (the path is unchanged).
    pub always_arrow: bool,
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
            always_arrow: false,
        }
    }
}

/// The cursor of a looping clip (the `loop` param, T46.2): over the clip's last `frames` it
/// eases (minimum jerk) from where it is to `first`, where it is at the clip's first frame, so
/// the last frame matches the first. `left` is how many frames remain after this one.
pub fn looped(point: Point, first: Point, left: f64, frames: f64) -> Point {
    if frames < 1.0 || left >= frames {
        return point;
    }
    // 0 at `frames` left, 1 on the last frame.
    let k = (1.0 - left.max(0.0) / (frames - 1.0).max(1.0)).clamp(0.0, 1.0);
    let k = k * k * k * (10.0 + k * (6.0 * k - 15.0));
    Point {
        x: point.x * (1.0 - k) + first.x * k,
        y: point.y * (1.0 - k) + first.y * k,
        opacity: point.opacity * (1.0 - k) + first.opacity * k,
        ..point
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
/// How far from a press any click effect reaches (spring window, press aim, pulse).
const REACH: f64 = CLICK_WINDOW + PRESS_AIM + 1.0 / RATE;
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
        // Clicks are in time order and every click effect is local: a moving window of the
        // clicks within REACH keeps the build linear in points plus clicks.
        let (mut lo, mut hi) = (0, 0);
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
            while lo < clicks.len() && clicks[lo][0] < t - REACH {
                lo += 1;
            }
            hi = hi.max(lo);
            while hi < clicks.len() && clicks[hi][0] <= t + REACH {
                hi += 1;
            }
            let near = &clicks[lo..hi];
            let near_click = near.iter().any(|c| (t - c[0]).abs() <= CLICK_WINDOW);
            let ahead = ((i as f64 + lead * RATE).round() as usize)
                .min(count - 1)
                .min(next_synthetic[i].saturating_sub(1).max(i));
            let target = filtered[ahead];
            if raw[i].2 {
                position = filtered[i];
                velocity = [0.0, 0.0];
            } else {
                let (physics, goal) = if near_click {
                    (SNAPPY, filtered[i])
                } else {
                    (MELLOW, target)
                };
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
            for c in near {
                // Flat for one table step each side, so lookups between points stay exact.
                let w = smoothstep(1.0 - ((t - c[0]).abs() - 1.0 / RATE).max(0.0) / PRESS_AIM);
                p = [p[0] + (c[1] - p[0]) * w, p[1] + (c[2] - p[1]) * w];
            }
            let image = raw[i].1;
            let mut scale = 1.0;
            if motion.click_pulse {
                for c in near {
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
                // Outside the captured area (a window capture while the pointer is elsewhere)
                // nothing is drawn: pinned to the edge it would hang over the backdrop.
                opacity: if image < 0 || !inside(raw[i].0) {
                    0.0
                } else {
                    1.0
                },
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

/// The arrow drawn for the cursor effect's `alwaysArrow` (T46.2), in screen points: the
/// macOS arrow's outline, tip at the origin.
const ARROW: [[f64; 2]; 7] = [
    [0.0, 0.0],
    [0.0, 16.0],
    [3.8, 12.3],
    [6.4, 18.2],
    [8.9, 17.1],
    [6.4, 11.4],
    [11.4, 11.4],
];
/// White outline around the black arrow, in points.
const ARROW_OUTLINE: f64 = 1.25;

/// The always-arrow cursor image: straight-alpha RGBA at `scale` pixels per point (a black
/// arrow with a white outline, 4×4 supersampled), its size, and its hotspot (the tip) in
/// pixels. Generated, so headless renders never depend on the system cursor.
pub fn arrow(scale: u32) -> (Vec<u8>, [u32; 2], [f64; 2]) {
    let k = f64::from(scale.max(1));
    let margin = ARROW_OUTLINE + 0.5;
    let size = [
        ((11.4 + 2.0 * margin) * k).ceil() as u32,
        ((18.2 + 2.0 * margin) * k).ceil() as u32,
    ];
    let inside = |[x, y]: [f64; 2]| {
        let mut odd = false;
        for i in 0..ARROW.len() {
            let ([ax, ay], [bx, by]) = (ARROW[i], ARROW[(i + 1) % ARROW.len()]);
            if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                odd = !odd;
            }
        }
        odd
    };
    let near = |[x, y]: [f64; 2]| {
        (0..ARROW.len()).any(|i| {
            let ([ax, ay], [bx, by]) = (ARROW[i], ARROW[(i + 1) % ARROW.len()]);
            let (dx, dy) = (bx - ax, by - ay);
            let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (x - ax - t * dx).hypot(y - ay - t * dy) <= ARROW_OUTLINE
        })
    };
    let mut rgba = Vec::with_capacity(size[0] as usize * size[1] as usize * 4);
    for py in 0..size[1] {
        for px in 0..size[0] {
            let (mut black, mut white) = (0.0, 0.0);
            for sy in 0..4 {
                for sx in 0..4 {
                    let p = [
                        (f64::from(px) + (f64::from(sx) + 0.5) / 4.0) / k - margin,
                        (f64::from(py) + (f64::from(sy) + 0.5) / 4.0) / k - margin,
                    ];
                    if inside(p) {
                        black += 1.0 / 16.0;
                    } else if near(p) {
                        white += 1.0 / 16.0;
                    }
                }
            }
            let alpha = black + white;
            let value = if alpha > 0.0 { white / alpha } else { 0.0 };
            let byte = |v: f64| (v * 255.0).round() as u8;
            rgba.extend_from_slice(&[byte(value), byte(value), byte(value), byte(alpha)]);
        }
    }
    (rgba, size, [margin * k, margin * k])
}

/// Whether a recorded position is on the captured source.
fn inside([x, y]: [f64; 2]) -> bool {
    (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)
}

fn smoothstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Hides the cursor after `IDLE_AFTER` seconds without movement or clicks (fading over
/// `FADE_OUT`) and shows it again `FADE_IN` before it next moves.
fn fade_idle(points: &mut [Point], path: &[[f64; 2]], start: f64, clicks: &[[f64; 3]]) {
    let n = points.len();
    let mut moving: Vec<bool> = (0..n)
        .map(|i| i > 0 && (path[i][0] - path[i - 1][0]).hypot(path[i][1] - path[i - 1][1]) > 1e-6)
        .collect();
    // A press counts as activity at its nearest table point.
    for c in clicks {
        let i = ((c[0] - start) * RATE).round();
        if (0.0..n as f64).contains(&i) {
            moving[i as usize] = true;
        }
    }
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

    /// Found on a real take: a window capture records the pointer outside the window, and the
    /// cursor must not hang at the card's edge over the backdrop.
    #[test]
    fn a_pointer_outside_the_captured_area_is_hidden() {
        let samples = [
            sample(0.0, 0.5, 0.5, false),
            sample(1.0, 0.5, 1.4, false),
            sample(3.0, 0.5, 0.5, false),
        ];
        let track = Track::build(&samples, SIZE, Motion::default());
        assert!(track.at(0.5).unwrap().opacity > 0.99);
        assert_eq!(track.at(2.0).unwrap().opacity, 0.0, "outside the window");
        assert!(track.at(3.2).unwrap().opacity > 0.99);
    }

    /// T39.4: a declared press the recorder never saw becomes a human path that lands on it.
    #[test]
    fn declared_marks_get_a_path_and_land_exactly() {
        let samples = [sample(0.0, 0.2, 0.2, false), sample(4.0, 0.2, 0.2, false)];
        let marks = [splice_model::PointerMark {
            t: 2.0,
            x: 0.8,
            y: 0.7,
            click: true,
        }];
        let merged = with_marks(&samples, &marks);
        assert_eq!(merged.len(), 3);
        assert!(merged[1].click && merged[1].image == 0);
        let track = Track::build(&merged, SIZE, Motion::default());
        let at = track.at(2.0).unwrap();
        assert!((at.x - 0.8).abs() < 1e-9 && (at.y - 0.7).abs() < 1e-9);
        assert!(largest_step(&track) < 120.0);
        assert!(
            track.at(1.0).unwrap().x < 0.25,
            "rests until the move starts"
        );
    }

    /// Review #5: the build is linear in points plus clicks (a 30-minute take with a click
    /// every 1.8 s used to scan every click several times per point).
    #[test]
    fn a_long_take_with_many_clicks_builds_quickly() {
        let samples: Vec<Sample> = (0..108_000)
            .map(|i| {
                let t = i as f64 / 60.0;
                sample(
                    t,
                    0.5 + 0.3 * (t * 0.7).sin(),
                    0.5 + 0.3 * (t * 0.3).cos(),
                    i % 108 == 0,
                )
            })
            .collect();
        let started = std::time::Instant::now();
        let track = Track::build(&samples, SIZE, Motion::default());
        let elapsed = started.elapsed();
        eprintln!("30 min, 1,000 clicks: built in {elapsed:?}");
        assert_eq!(track.clicks().len(), 1000);
        assert!(elapsed.as_millis() < 1500, "{elapsed:?}");
    }

    /// Documents `smoothing` against Recorder's cursor presets (rapid, medium, smooth: critically
    /// damped springs with 0.12, 0.22 and 0.38 s responses, no lead): on the same hand-like
    /// path, the smoothing whose drawn cursor is nearest (RMS) to each preset.
    #[test]
    fn smoothing_matches_recorder_presets() {
        // Moves between targets with a little tremor, 120 Hz like the recorder.
        let targets = [(0.2, 0.3), (0.7, 0.4), (0.6, 0.8), (0.25, 0.6), (0.5, 0.5)];
        let mut samples = Vec::new();
        for i in 0..(6 * 120) {
            let t = f64::from(i) / 120.0;
            let leg = ((t / 1.2) as usize).min(targets.len() - 2);
            let k = ((t - leg as f64 * 1.2) / 0.4).clamp(0.0, 1.0);
            let k = k * k * (3.0 - 2.0 * k);
            let (a, b) = (targets[leg], targets[leg + 1]);
            let tremor = 0.002 * (t * 37.0).sin();
            samples.push(sample(
                t,
                a.0 + (b.0 - a.0) * k + tremor,
                a.1 + (b.1 - a.1) * k,
                false,
            ));
        }
        let raw = |t: f64| {
            let i = samples.partition_point(|s| s.t <= t).max(1) - 1;
            [samples[i].x, samples[i].y]
        };
        // Recorder's CursorPath stage 2: semi-implicit Euler at 240 Hz toward the raw path.
        let recorder = |response: f64| {
            let omega = std::f64::consts::TAU / response;
            let (mut p, mut v) = (raw(0.0), [0.0, 0.0]);
            let mut out = Vec::new();
            for step in 0..(6 * 240) {
                let t = f64::from(step) / 240.0;
                let target = raw(t);
                for axis in 0..2 {
                    let a = omega * omega * (target[axis] - p[axis]) - 2.0 * omega * v[axis];
                    v[axis] += a / 240.0;
                    p[axis] += v[axis] / 240.0;
                }
                if step % 4 == 0 {
                    out.push((t, p));
                }
            }
            out
        };
        let best = |response: f64| {
            let reference = recorder(response);
            (0..=20)
                .map(|n| {
                    let smoothing = f64::from(n) / 20.0;
                    let track = Track::build(
                        &samples,
                        SIZE,
                        Motion {
                            smoothing,
                            hide_idle: false,
                            click_pulse: false,
                            synthesize: false,
                            ..Motion::default()
                        },
                    );
                    // Splice's spring leads by its own lag and Recorder's trails: compare
                    // shapes, aligned at the shift that fits best (Recorder's mean delay is
                    // 2/ω, under 0.13 s for every preset).
                    let error = (0..=12)
                        .map(|shift| {
                            let shift = f64::from(shift) / 60.0;
                            reference
                                .iter()
                                .filter(|(t, _)| *t >= shift)
                                .map(|&(t, [x, y])| {
                                    let p = track.at(t - shift).unwrap();
                                    (p.x - x).powi(2) + (p.y - y).powi(2)
                                })
                                .sum::<f64>()
                        })
                        .fold(f64::INFINITY, f64::min);
                    (error, smoothing)
                })
                .fold((f64::INFINITY, 0.0), |a, b| if b.0 < a.0 { b } else { a })
                .1
        };
        let mapped = [best(0.12), best(0.22), best(0.38)];
        assert_eq!(mapped, [0.1, 0.3, 0.75], "rapid, medium, smooth");
    }

    /// The always-arrow image: black inside, a white rim, transparent around, tip at the hotspot.
    #[test]
    fn the_generated_arrow_is_outlined_and_tipped_at_its_hotspot() {
        let (rgba, [w, h], [hx, hy]) = arrow(2);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let at = |x: f64, y: f64| {
            let i = ((y.floor() as u32 * w + x.floor() as u32) * 4) as usize;
            [rgba[i], rgba[i + 3]]
        };
        // Inside the body (3 pt right, 8 pt down from the tip): opaque black.
        assert_eq!(at(hx + 6.0, hy + 16.0), [0, 255]);
        // Just left of the left edge: the white outline.
        let rim = at(hx - 1.5, hy + 16.0);
        assert!(rim[0] > 200 && rim[1] > 200, "{rim:?}");
        // Far right of the stem: transparent.
        assert_eq!(at(f64::from(w) - 1.0, hy + 2.0)[1], 0);
        // Deterministic.
        assert_eq!(arrow(2).0, rgba);
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
