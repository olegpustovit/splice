//! What happens in a screen take, from its recorded input (D15, PLAN.md T41.1): idle stretches,
//! typing runs, click and scroll ranges, and the busy bursts that silent-take editing
//! (`intent.steps`, `basis:"activity"`, `intent.speedTyping`) cuts around. Times are seconds on
//! the take's clock. Only event kinds and times are read: typed characters never enter here (D19).
//!
//! Idle means no input event and no transcript word for at least [`IDLE`] seconds. Each idle
//! stretch keeps [`PAD`] seconds at its head and tail, so the moment before and after stays
//! visible when an editor cuts it.
// ponytail: idle ignores frame differences and audio level (D15 lists both); add a macOS
// frame-diff and loudness pass when a silent take with an on-screen animation or narration
// without a transcript trims away content.

/// Quiet seconds before a stretch counts as idle.
pub const IDLE: f64 = 2.0;
/// Seconds of each idle stretch's head and tail that stay visible.
pub const PAD: f64 = 0.25;
/// Events closer than this (seconds) belong to one busy burst.
pub const BURST_GAP: f64 = 0.5;
/// Key presses closer than this (seconds) belong to one typing run.
pub const TYPING_GAP: f64 = 1.0;
/// How long one instantaneous event (a key, a move) counts as busy, in seconds.
const BLIP: f64 = 0.1;

/// What one recorded input event was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The pointer moved or dragged.
    Pointer,
    /// A button went down.
    Press,
    /// A button went up.
    Release,
    /// A key without command, control or option: text entry.
    Typing,
    /// A key with a command modifier, or a modifier change.
    Shortcut,
    /// A scroll wheel or trackpad scroll.
    Scroll,
}

/// One recorded input event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Seconds on the take's clock.
    pub t: f64,
    /// What happened.
    pub input: Input,
}

/// A take's activity inside one window, as `[from, to)` second ranges in time order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    /// Stretches with no input and no speech for at least [`IDLE`] s, inset by [`PAD`].
    pub idle: Vec<[f64; 2]>,
    /// Typing runs (keys within [`TYPING_GAP`] of each other).
    pub typing: Vec<[f64; 2]>,
    /// Press-to-release ranges, presses within [`BURST_GAP`] merged.
    pub clicks: Vec<[f64; 2]>,
    /// Scroll runs.
    pub scroll: Vec<[f64; 2]>,
    /// Every event and spoken word, merged into bursts within [`BURST_GAP`].
    pub busy: Vec<[f64; 2]>,
}

/// The activity of `events` (sorted or not) and `speech` ranges inside `window` (seconds).
pub fn activity(events: &[Event], speech: &[[f64; 2]], window: [f64; 2]) -> Activity {
    let mut sorted = events.to_vec();
    sorted.sort_by(|a, b| a.t.total_cmp(&b.t));
    let blips = |wanted: &dyn Fn(Input) -> bool| -> Vec<[f64; 2]> {
        sorted
            .iter()
            .filter(|event| wanted(event.input))
            .map(|event| [event.t, event.t + BLIP])
            .collect()
    };
    let typing = runs(blips(&|input| input == Input::Typing), TYPING_GAP, window);
    let scroll = runs(blips(&|input| input == Input::Scroll), BURST_GAP, window);
    // A press lasts until its release.
    let mut presses = Vec::new();
    let mut down: Option<f64> = None;
    for event in &sorted {
        match event.input {
            Input::Press => {
                if let Some(t) = down {
                    presses.push([t, t + BLIP]);
                }
                down = Some(event.t);
            }
            Input::Release => {
                if let Some(t) = down.take() {
                    presses.push([t, event.t.max(t + BLIP)]);
                }
            }
            _ => {}
        }
    }
    if let Some(t) = down {
        presses.push([t, t + BLIP]);
    }
    let clicks = runs(presses, BURST_GAP, window);
    let mut all = blips(&|_| true);
    all.extend_from_slice(speech);
    let busy = runs(all, BURST_GAP, window);
    // Idle: gaps of at least IDLE between busy bursts (and the window's edges), inset by PAD.
    let mut idle = Vec::new();
    let mut at = window[0];
    for range in busy.iter().chain(std::iter::once(&[window[1], window[1]])) {
        if range[0] - at >= IDLE {
            idle.push([at + PAD, range[0] - PAD]);
        }
        at = at.max(range[1]);
    }
    Activity {
        idle,
        typing,
        clicks,
        scroll,
        busy,
    }
}

/// `ranges` sorted, merged when closer than `gap`, and clipped to `window`.
fn runs(mut ranges: Vec<[f64; 2]>, gap: f64, window: [f64; 2]) -> Vec<[f64; 2]> {
    ranges.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(ranges.len());
    for [from, to] in ranges {
        let (from, to) = (from.max(window[0]), to.min(window[1]));
        if to <= from {
            continue;
        }
        match out.last_mut() {
            Some(last) if from - last[1] <= gap => last[1] = last[1].max(to),
            _ => out.push([from, to]),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(t: f64, input: Input) -> Event {
        Event { t, input }
    }

    fn close(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| (x[0] - y[0]).abs() < 1e-9 && (x[1] - y[1]).abs() < 1e-9)
    }

    /// T41.1: a silent take's idle stretches, typing runs, clicks and scrolls, with speech
    /// keeping a stretch busy and each idle stretch keeping its head and tail.
    #[test]
    fn activity_finds_idle_typing_clicks_and_scroll() {
        let mut events = vec![
            at(1.0, Input::Pointer),
            at(1.2, Input::Press),
            at(1.3, Input::Release),
        ];
        // Typing 5.0–6.6 s with one shortcut; a gap of 1.5 s then more typing (a new run).
        for i in 0..9 {
            events.push(at(5.0 + f64::from(i) * 0.2, Input::Typing));
        }
        events.push(at(5.5, Input::Shortcut));
        events.push(at(8.2, Input::Typing));
        events.push(at(12.0, Input::Scroll));
        events.push(at(12.3, Input::Scroll));
        let activity = activity(&events, &[[15.0, 17.0]], [0.0, 20.0]);
        assert!(close(&activity.clicks, &[[1.2, 1.3]]), "{activity:?}");
        assert!(close(&activity.typing, &[[5.0, 6.7], [8.2, 8.3]]));
        assert!(close(&activity.scroll, &[[12.0, 12.4]]));
        assert!(close(
            &activity.busy,
            &[
                [1.0, 1.4],
                [5.0, 6.7],
                [8.2, 8.3],
                [12.0, 12.4],
                [15.0, 17.0]
            ]
        ));
        // Quiet from 1.4 to 5.0, 8.3 to 12.0 and 12.4 to 15.0; speech keeps 15–17 busy and
        // the window's 3 s tail is idle too. Each keeps 0.25 s at both ends.
        assert!(close(
            &activity.idle,
            &[[1.65, 4.75], [8.55, 11.75], [12.65, 14.75], [17.25, 19.75]]
        ));
        // The window clips everything; a lone press without release still counts.
        let clipped = super::activity(&[at(3.0, Input::Press)], &[], [2.0, 3.05]);
        assert!(close(&clipped.clicks, &[[3.0, 3.05]]));
        assert!(clipped.idle.is_empty());
    }
}
