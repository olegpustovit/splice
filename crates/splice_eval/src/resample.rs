//! Exports at another frame rate (`export.start {fps}`, GIFs): frame sampling of the timeline
//! clock. Every time in a project is an integer frame at its own rate; output frame `n` of a
//! range shows the timeline frame on screen at `n / to` seconds, `floor(n × from / to)` frames
//! after the range start. Nothing is blended or interpolated: a lower rate drops frames, a
//! higher one repeats them, and the output lasts as long as the range (rounded up to a whole
//! output frame).
use splice_model::Frame;

/// Output frames at `to` fps covering `frames` timeline frames at `from` fps.
pub fn frames(frames: Frame, from: u32, to: u32) -> Frame {
    let (frames, from, to) = (
        i64::from(frames.max(0)),
        i64::from(from.max(1)),
        i64::from(to),
    );
    Frame::try_from((frames * to + from - 1) / from).unwrap_or(Frame::MAX)
}

/// The timeline frame (counted from the range start) output frame `n` shows.
pub fn source(n: Frame, from: u32, to: u32) -> Frame {
    let (n, from, to) = (i64::from(n.max(0)), i64::from(from), i64::from(to.max(1)));
    Frame::try_from(n * from / to).unwrap_or(Frame::MAX)
}

/// The output frames, half-open, that show timeline frame `k` (counted from the range start):
/// empty when a lower rate drops it, several when a higher one repeats it.
pub fn outputs(k: Frame, from: u32, to: u32) -> [Frame; 2] {
    [frames(k, from, to), frames(k + 1, from, to)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_frames_sample_the_timeline_clock_without_blending() {
        // Same rate: every frame once.
        assert_eq!(frames(90, 30, 30), 90);
        assert_eq!(
            (0..5).map(|n| source(n, 30, 30)).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        // 60 → 24: frames on screen at n/24 s; 2.5 timeline frames per output frame.
        assert_eq!(frames(60, 60, 24), 24);
        assert_eq!(
            (0..5).map(|n| source(n, 60, 24)).collect::<Vec<_>>(),
            [0, 2, 5, 7, 10]
        );
        // 30 → 60: each frame twice; 25 → 30: one in five repeats.
        assert_eq!(frames(30, 30, 60), 60);
        assert_eq!(
            (0..4).map(|n| source(n, 30, 60)).collect::<Vec<_>>(),
            [0, 0, 1, 1]
        );
        assert_eq!(
            (0..6).map(|n| source(n, 25, 30)).collect::<Vec<_>>(),
            [0, 0, 1, 2, 3, 4]
        );
        // A range that is not a whole number of output frames rounds up: 31 frames at 30 → 24.
        assert_eq!(frames(31, 30, 24), 25);
        // `outputs` inverts `source`: every output frame belongs to exactly one timeline frame.
        for (from, to) in [(30, 24), (24, 60), (25, 30), (60, 15), (50, 24)] {
            let total = frames(100, from, to);
            let mut next = 0;
            for k in 0..100 {
                let [first, last] = outputs(k, from, to);
                assert_eq!(first, next, "{from}->{to} at {k}");
                for n in first..last {
                    assert_eq!(source(n, from, to), k, "{from}->{to} output {n}");
                }
                next = last;
            }
            assert_eq!(next, total, "{from}->{to}");
        }
    }
}
