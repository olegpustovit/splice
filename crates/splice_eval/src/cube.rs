//! `.cube` 3D LUTs (T18.1): parsed here, sampled by the renderer as an `n² × n` atlas.

/// Entries in red-fastest order, and the cube size.
pub fn parse_cube(text: &str) -> Option<(u32, Vec<[f32; 3]>)> {
    let mut size = 0u32;
    let mut entries = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
            size = rest
                .trim()
                .parse()
                .ok()
                .filter(|&n| (2..=256).contains(&n))?;
            entries.reserve((size as usize).pow(3));
            continue;
        }
        if line.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue; // TITLE, DOMAIN_MIN/MAX, LUT_1D_SIZE …
        }
        let mut values = line.split_whitespace().map(str::parse::<f32>);
        let (Some(Ok(r)), Some(Ok(g)), Some(Ok(b))) = (values.next(), values.next(), values.next())
        else {
            return None;
        };
        entries.push([r, g, b]);
    }
    (size > 0 && entries.len() == (size as usize).pow(3)).then_some((size, entries))
}

/// The renderer's atlas: `n` slices of `n × n` side by side (blue selects the slice), RGBA8.
pub fn cube_atlas(size: u32, entries: &[[f32; 3]]) -> Vec<u8> {
    let n = size as usize;
    let mut pixels = vec![0u8; n * n * n * 4];
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let [x, y, z] = entries[r + g * n + b * n * n];
                let at = ((g * n * n) + b * n + r) * 4;
                for (slot, value) in pixels[at..at + 3].iter_mut().zip([x, y, z]) {
                    *slot = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                pixels[at + 3] = 255;
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_cube_parses_and_lays_out_slices_by_blue() {
        let mut text = String::from("TITLE \"identity\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        let (size, entries) = parse_cube(&text).unwrap();
        assert_eq!((size, entries.len()), (2, 8));
        let atlas = cube_atlas(size, &entries);
        // Row g=1, slice b=1, column r=0 → texel (b*n + r, g) = (2, 1).
        let at = (4 + 2) * 4;
        assert_eq!(&atlas[at..at + 4], &[0, 255, 255, 255]);
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_none());
    }
}
