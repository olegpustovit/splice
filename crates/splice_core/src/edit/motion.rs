//! Text and motion (PLAN.md T42, D16, D20): spring settle reports, inline title looks that
//! compile to `text`/`layout`/`animate`/`transform` effects, motion presets as ordinary keys,
//! and paint order.
use super::*;

/// The slowest spring among `keys` and the first spring the next key cuts short.
#[derive(Clone, Copy, Default)]
pub(super) struct Springs {
    settle: Option<f64>,
    /// (key frame, settle frames, frames to the next key).
    cut: Option<(Frame, f64, Frame)>,
}

/// Measures the spring segments of sorted keys (the last key's ease never plays).
pub(super) fn springs(keys: &[Key]) -> Springs {
    let mut out = Springs::default();
    for pair in keys.windows(2) {
        let Some(settle) = pair[0].ease.settle() else {
            continue;
        };
        out.settle = Some(out.settle.map_or(settle, |s: f64| s.max(settle)));
        let gap = pair[1].at - pair[0].at;
        if out.cut.is_none() && f64::from(gap) < settle {
            out.cut = Some((pair[0].at, settle, gap));
        }
    }
    out
}

impl Editor {
    /// Reports spring settle times on the current change; a spring the next key interrupts is
    /// named in the summary so the agent can move the key.
    pub(super) fn report_springs(&mut self, springs: Springs) {
        let Some(settle) = springs.settle else {
            return;
        };
        let detail = self.detail.get_or_insert_with(ChangeDetail::default);
        detail.settle = Some(detail.settle.map_or(settle, |s| s.max(settle)));
        if let Some((at, settle, gap)) = springs.cut {
            write!(
                self.note,
                "; the spring at frame {at} needs {} frames to settle but the next key is {gap} \
                 frames later (it jumps there)",
                settle.ceil()
            )
            .expect("writing to a String");
        }
    }
}
