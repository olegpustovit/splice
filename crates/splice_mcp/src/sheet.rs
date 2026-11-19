//! Contact sheets (T23.3): `view.frames {sheet:true}` tiles every requested frame, box-filtered
//! down to `edge` pixels on its long side, into one image so an agent sees a whole edit at once.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// Tiles RGBA `frames` of `size` into a grid; returns the sheet's pixels and size.
pub(super) fn tile(frames: &[Vec<u8>], [w, h]: [u32; 2], edge: u32) -> (Vec<u8>, [u32; 2]) {
    let scale = (w.max(h) as f64 / f64::from(edge.max(1))).max(1.0);
    let (tw, th) = (
        ((f64::from(w) / scale) as u32).max(1),
        ((f64::from(h) / scale) as u32).max(1),
    );
    let columns = (frames.len() as f64).sqrt().ceil().max(1.0) as u32;
    let rows = (frames.len() as u32).div_ceil(columns).max(1);
    let gap = 4;
    let (sw, sh) = (
        columns * tw + (columns + 1) * gap,
        rows * th + (rows + 1) * gap,
    );
    let mut sheet = vec![32u8; (sw * sh * 4) as usize];
    sheet
        .iter_mut()
        .skip(3)
        .step_by(4)
        .for_each(|alpha| *alpha = 255);
    for (index, frame) in frames.iter().enumerate() {
        let (col, row) = (index as u32 % columns, index as u32 / columns);
        let (ox, oy) = (gap + col * (tw + gap), gap + row * (th + gap));
        for y in 0..th {
            for x in 0..tw {
                // Box filter over the source pixels this thumbnail pixel covers.
                let (x0, x1) = ((x * w) / tw, (((x + 1) * w) / tw).max((x * w) / tw + 1));
                let (y0, y1) = ((y * h) / th, (((y + 1) * h) / th).max((y * h) / th + 1));
                let mut sum = [0u32; 4];
                for sy in y0..y1.min(h) {
                    for sx in x0..x1.min(w) {
                        let at = ((sy * w + sx) * 4) as usize;
                        for c in 0..4 {
                            sum[c] += u32::from(frame[at + c]);
                        }
                    }
                }
                let n = (y1.min(h) - y0) * (x1.min(w) - x0);
                let at = (((oy + y) * sw + ox + x) * 4) as usize;
                for c in 0..4 {
                    sheet[at + c] = (sum[c] / n.max(1)) as u8;
                }
            }
        }
    }
    (sheet, [sw, sh])
}

/// What changed from `a` to `b` (T43.3): `b` dimmed to a quarter, with the per-pixel change
/// drawn over it in red (small) through yellow (large), so motion reads at a glance.
pub(super) fn heatmap(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .flat_map(|(p, q)| {
            let change = (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0);
            let heat = u16::from(change).saturating_mul(4).min(255) as u8;
            let dim = |v: u8| v / 4;
            [
                dim(q[0]).max(heat),
                dim(q[1]).max(heat.saturating_sub(128).saturating_mul(2)),
                dim(q[2]),
                255,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn heatmap_marks_only_what_changed() {
        let a = [10, 10, 10, 255, 200, 200, 200, 255, 200, 200, 200, 255];
        let b = [10, 10, 10, 255, 180, 200, 200, 255, 0, 200, 200, 255];
        let map = super::heatmap(&a, &b);
        // Unchanged: the dimmed frame. A small change is red; a large one saturates to yellow.
        assert_eq!(&map[..4], &[2, 2, 2, 255]);
        assert_eq!(&map[4..8], &[80, 50, 50, 255]);
        assert_eq!(&map[8..], &[255, 254, 50, 255]);
    }

    #[test]
    fn frames_tile_into_a_grid_of_thumbnails() {
        let red: Vec<u8> = (0..64 * 32).flat_map(|_| [200, 10, 10, 255]).collect();
        let blue: Vec<u8> = (0..64 * 32).flat_map(|_| [10, 10, 200, 255]).collect();
        let frames = vec![red.clone(), blue.clone(), red, blue, vec![255; 64 * 32 * 4]];
        let (sheet, [w, h]) = super::tile(&frames, [64, 32], 16);
        // Five frames make a 3×2 grid of 16×8 thumbnails with 4 px gaps.
        assert_eq!([w, h], [3 * 16 + 4 * 4, 2 * 8 + 3 * 4]);
        let pixel =
            |x: u32, y: u32| &sheet[((y * w + x) * 4) as usize..((y * w + x) * 4 + 3) as usize];
        assert_eq!(pixel(4 + 8, 4 + 4), [200, 10, 10]);
        assert_eq!(pixel(4 + 16 + 4 + 8, 4 + 4), [10, 10, 200]);
        assert_eq!(pixel(0, 0), [32, 32, 32]);
    }
}
