use crate::style::{CameraStyle, Corner};
use splice_model::{Fit, Framing, Placement, Rect};

pub(super) fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect { x, y, w, h }
}

pub(super) fn overlay(placement: Placement, [w, h]: [f64; 2]) -> Rect {
    let cw = w * 0.34;
    let ch = cw * 0.62;
    let margin = w * 0.05;
    match placement {
        Placement::Full => rect(0.0, 0.0, w, h),
        Placement::Card => rect(w * 0.1, h * 0.5 - w * 0.8 * 0.31, w * 0.8, w * 0.8 * 0.62),
        Placement::TopLeft => rect(margin, margin, cw, ch),
        Placement::TopRight => rect(w - margin - cw * 0.6, margin, cw * 0.6, cw * 0.6 * 0.32),
        Placement::BottomLeft => rect(margin, h - margin - ch, cw, ch),
        Placement::BottomRight => rect(w - margin - cw, h - margin - ch, cw, ch),
    }
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
    let mut source = framing.crop.unwrap_or(rect(0.0, 0.0, 1.0, 1.0));
    let [w, h] = size.map(f64::from);
    let source_width = w * source.w;
    let source_height = h * source.h;
    let fit = inner.w / source_width;
    let fill = inner.h / source_height;
    let dest = match framing.fit {
        Fit::Fit => {
            let scale = fit.min(fill);
            let width = source_width * scale;
            let height = source_height * scale;
            rect(
                inner.x + (inner.w - width) / 2.0,
                inner.y + (inner.h - height) / 2.0,
                width,
                height,
            )
        }
        Fit::Fill => {
            let scale = fit.max(fill);
            let width = inner.w / scale / w;
            let height = inner.h / scale / h;
            source.x += (source.w - width) / 2.0;
            source.y += (source.h - height) / 2.0;
            source.w = width;
            source.h = height;
            inner
        }
    };
    (source, dest)
}

pub(super) fn camera(
    [w, h]: [f64; 2],
    style: CameraStyle,
    screen: bool,
    keys: bool,
) -> (Rect, f64) {
    let short = w.min(h);
    if !screen {
        let size = short * 0.72;
        return (
            rect((w - size) / 2.0, (h - size) / 2.0, size, size),
            size * 0.05,
        );
    }
    let size = short * style.size;
    let margin = short * 0.05;
    let x = if matches!(style.corner, Corner::TopRight | Corner::BottomRight) {
        w - margin - size
    } else {
        margin
    };
    let y = if matches!(style.corner, Corner::BottomRight | Corner::BottomLeft) {
        h - margin - size - if keys { h * 0.06 } else { 0.0 }
    } else {
        margin
    };
    (rect(x, y, size, size), size / 2.0 * style.roundness)
}
