use crate::style::{CameraStyle, Corner};
use splice_model::{Ease, Fit, Framing, Rect};

pub(super) fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

pub(super) fn picture(
    bounds: Rect,
    size: [u32; 2],
    framing: Framing,
    padding: f64,
) -> (Rect, Rect) {
    let margin = bounds.w.min(bounds.h) * padding;
    let inner = rect(
        bounds.x + margin,
        bounds.y + margin,
        bounds.w - 2.0 * margin,
        bounds.h - 2.0 * margin,
    );
    let source = framing.visible(size, [inner.w, inner.h]);
    let dest = match framing.fit {
        Fit::Fit => {
            let [w, h] = size.map(f64::from);
            let (source_width, source_height) = (w * source.w, h * source.h);
            let scale = (inner.w / source_width).min(inner.h / source_height);
            let width = source_width * scale;
            let height = source_height * scale;
            rect(
                inner.x + (inner.w - width) / 2.0,
                inner.y + (inner.h - height) / 2.0,
                width,
                height,
            )
        }
        Fit::Fill => inner,
    };
    (source, dest)
}

/// The camera bubble at one frame (T46.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Bubble {
    pub(super) bounds: Rect,
    pub(super) radius: f64,
    pub(super) shadow: f64,
    pub(super) opacity: f64,
}

/// How far `shrinkWhenZoomed` takes the bubble at 2× and beyond.
const SHRUNK: f64 = 0.6;

/// The bubble at owner-relative frame `at` under the timeline camera's `amount`: the bubble
/// layout's rectangle, blended toward the full canvas (`full`) or faded out (`hidden`) by the
/// layout weights. `ease` is a layout switch's length in frames.
pub(super) fn camera(
    [w, h]: [f64; 2],
    style: &CameraStyle,
    screen: bool,
    keys: bool,
    at: f64,
    amount: f64,
    ease: f64,
) -> Bubble {
    let short = w.min(h);
    let roundness = style.roundness.at(at).clamp(0.0, 1.0);
    let (bubble, radius) = if screen {
        let mut size = short * style.size.at(at).clamp(0.1, 1.0);
        if style.shrink {
            size *= 1.0 - (1.0 - SHRUNK) * (amount - 1.0).clamp(0.0, 1.0);
        }
        let (bw, bh) = if style.circle || style.aspect == 1.0 {
            (size, size)
        } else if style.aspect > 1.0 {
            (size, size / style.aspect)
        } else {
            (size * style.aspect, size)
        };
        let (x, y) = match &style.position {
            // The center, kept on the canvas.
            Some([x, y]) => (
                (x.at(at) * w - bw / 2.0).clamp(0.0, (w - bw).max(0.0)),
                (y.at(at) * h - bh / 2.0).clamp(0.0, (h - bh).max(0.0)),
            ),
            None => {
                let margin = short * 0.05;
                let right = matches!(style.corner, Corner::TopRight | Corner::BottomRight);
                let bottom = matches!(style.corner, Corner::BottomRight | Corner::BottomLeft);
                (
                    if right { w - margin - bw } else { margin },
                    if bottom {
                        h - margin - bh - if keys { h * 0.06 } else { 0.0 }
                    } else {
                        margin
                    },
                )
            }
        };
        let round = if style.circle { 1.0 } else { roundness };
        (rect(x, y, bw, bh), bw.min(bh) / 2.0 * round)
    } else {
        // A camera-only take: the presenter centered.
        let size = short * 0.72;
        (
            rect((w - size) / 2.0, (h - size) / 2.0, size, size),
            size * 0.05,
        )
    };
    let [b, f, hidden] = layout_weights(&style.layout, at, ease);
    let keep = b + hidden;
    Bubble {
        bounds: rect(
            bubble.x * keep,
            bubble.y * keep,
            bubble.w * keep + w * f,
            bubble.h * keep + h * f,
        ),
        radius: radius * keep.max(0.0),
        shadow: style.shadow.at(at).clamp(0.0, 1.0) * (1.0 - f).clamp(0.0, 1.0),
        opacity: (1.0 - hidden).clamp(0.0, 1.0),
    }
}

/// Weights of the bubble, full and hidden layouts at `at`. Each key switches to its layout at
/// its frame and eases there over `span` frames with its own ease (`hold` cuts; a spring takes
/// its settle time); a switch mid-ease starts from where the last one got to.
pub(super) fn layout_weights(layout: &crate::look::Num, at: f64, span: f64) -> [f64; 3] {
    let one = |value: f64| {
        let mut weights = [0.0; 3];
        weights[value.round().clamp(0.0, 2.0) as usize] = 1.0;
        weights
    };
    let progress = |ease: Ease, elapsed: f64| {
        let span = match ease {
            Ease::Hold => return 1.0,
            Ease::Spring { .. } => ease.settle().unwrap_or(span),
            _ => span,
        };
        if elapsed >= span {
            1.0
        } else {
            ease.apply(elapsed / span, span)
        }
    };
    let mix = |a: [f64; 3], b: [f64; 3], t: f64| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
    let keys = layout.keys();
    let Some(first) = keys.first() else {
        return one(layout.at(at));
    };
    let (mut from, mut to) = (one(first.value), one(first.value));
    let (mut start, mut ease) = (f64::from(first.at), first.ease);
    for key in &keys[1..] {
        let switch = f64::from(key.at);
        if switch > at {
            break;
        }
        from = mix(from, to, progress(ease, switch - start));
        to = one(key.value);
        (start, ease) = (switch, key.ease);
    }
    mix(from, to, progress(ease, (at - start).max(0.0)))
}
