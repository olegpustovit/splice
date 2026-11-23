//! A streaming GIF89a writer for `export.start {preset:"gif"}`: each frame is quantized and
//! LZW-coded as it arrives and written out, so a GIF of any length holds one frame in memory.
//! (ImageIO's `CGImageDestination` keeps every frame until it finalizes: 2 MB a frame at
//! 960 × 540, gigabytes for a minute at 24 fps.) Portable; macOS feeds it rendered frames.
//!
//! Per frame: only the rectangle that changed since the previous frame is coded, and pixels in
//! it that did not change are transparent, so a screen recording's still stretches cost a few
//! bytes. A frame with no change merges into the previous one's delay. Colors: a local
//! 255-color palette by median cut over a 15-bit histogram of the changed pixels (exact when
//! they hold 255 colors or fewer). Delays are hundredths of a second, accumulated so a 24 fps GIF
//! keeps time (4, 4, 4, 5, …) instead of drifting.
// ponytail: no dithering; gradients band. Add ordered dithering if exported GIFs of video (not
// screen recordings) look posterized.
use std::io::{self, Write};

/// Histogram bins: 5 bits per channel.
const BINS: usize = 1 << 15;
/// Colors per frame; the index after the last color is transparent.
const COLORS: usize = 255;
/// LZW hash slots (twice the 4096 codes).
const SLOTS: usize = 8192;

/// One median-cut box: a range of `bins`, its pixel count, its widest channel and the split
/// priority (pixels × that channel's range; 0 when it cannot split).
#[derive(Clone, Copy, Debug, Default)]
struct Cut {
    start: usize,
    end: usize,
    pixels: u64,
    axis: usize,
    score: u64,
}

/// A 15-bit bin's channel `c` (0 red, 1 green, 2 blue), 0–31.
fn channel(bin: u16, c: usize) -> u16 {
    (bin >> (10 - 5 * c)) & 31
}

impl Cut {
    fn measure(bins: &[u16], counts: &[u32], start: usize, end: usize) -> Self {
        let (mut lo, mut hi) = ([31u16; 3], [0u16; 3]);
        let mut pixels = 0;
        for &b in &bins[start..end] {
            pixels += u64::from(counts[usize::from(b)]);
            for c in 0..3 {
                lo[c] = lo[c].min(channel(b, c));
                hi[c] = hi[c].max(channel(b, c));
            }
        }
        let axis = (0..3)
            .max_by_key(|&c| hi[c].saturating_sub(lo[c]))
            .unwrap_or(0);
        let range = u64::from(hi[axis].saturating_sub(lo[axis]));
        Self {
            start,
            end,
            pixels,
            axis,
            score: if end - start < 2 { 0 } else { pixels * range },
        }
    }
}

/// Writes frames of one size as a looping GIF.
pub(super) struct GifWriter {
    size: [usize; 2],
    fps: u32,
    /// Frames received (the delay clock).
    frames: u64,
    /// The previous frame as received (RGBA); empty before the first.
    previous: Vec<u8>,
    counts: Vec<u32>,
    sums: Vec<[u32; 3]>,
    /// Bins present in this frame.
    bins: Vec<u16>,
    /// Bin → palette index.
    map: Vec<u8>,
    cuts: Vec<Cut>,
    palette: [[u8; 3]; 256],
    /// The frame's transparent index (one past its colors) and its table size `2 << bits`.
    transparent: u8,
    bits: u32,
    indices: Vec<u8>,
    coded: Vec<u8>,
    keys: Vec<u32>,
    codes: Vec<u16>,
    generation: u32,
    /// The last frame, coded but not written: a repeat only lengthens its delay.
    pending: Vec<u8>,
    /// Offset of the pending frame's delay and its value (hundredths of a second).
    delay: Option<(usize, u32)>,
    /// Bytes written so far.
    written: u64,
}

impl GifWriter {
    /// A writer for `size` frames (each at most 65535 px) at `fps`.
    pub(super) fn new(size: [u32; 2], fps: u32) -> Self {
        let size = size.map(|n| n.clamp(1, 65535) as usize);
        let pixels = size[0] * size[1];
        // Worst case LZW: 12 bits a pixel, plus sub-block lengths and headers.
        let coded = pixels * 3 / 2 + 4096;
        Self {
            size,
            fps: fps.max(1),
            frames: 0,
            previous: Vec::with_capacity(pixels * 4),
            counts: vec![0; BINS],
            sums: vec![[0; 3]; BINS],
            bins: Vec::with_capacity(BINS),
            map: vec![0; BINS],
            cuts: Vec::with_capacity(COLORS),
            palette: [[0; 3]; 256],
            transparent: 0,
            bits: 0,
            indices: Vec::with_capacity(pixels),
            coded: Vec::with_capacity(coded),
            keys: vec![0; SLOTS],
            codes: vec![0; SLOTS],
            generation: 0,
            pending: Vec::with_capacity(coded + coded / 255 + 1024),
            delay: None,
            written: 0,
        }
    }

    /// Bytes written to the file so far.
    pub(super) fn written(&self) -> u64 {
        self.written
    }

    /// Adds one RGBA frame (alpha ignored), writing the frame before it to `out`.
    pub(super) fn frame(&mut self, rgba: &[u8], out: &mut dyn Write) -> io::Result<()> {
        let [w, h] = self.size;
        if rgba.len() != w * h * 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "A GIF frame must be one whole RGBA image of the GIF's size.",
            ));
        }
        let n = self.frames;
        self.frames += 1;
        let fps = u64::from(self.fps);
        let hundredths = |n: u64| (n * 200 + fps) / (2 * fps);
        let step = (hundredths(n + 1) - hundredths(n)) as u32;
        let first = self.previous.is_empty();
        let changed = if first {
            Some([0, 0, w, h])
        } else {
            self.changed(rgba)
        };
        // An unchanged frame lengthens the one on screen (up to the format's 655 s).
        if let (None, Some((at, delay))) = (changed, self.delay)
            && delay + step <= 0xffff
        {
            self.delay = Some((at, delay + step));
            self.pending[at..at + 2].copy_from_slice(&((delay + step) as u16).to_le_bytes());
            return Ok(());
        }
        self.flush(out)?;
        if first {
            self.header();
        }
        // A frame with no change past the delay limit: one transparent pixel.
        let [x, y, cw, ch] = changed.unwrap_or([0, 0, 1, 1]);
        self.quantize(rgba, [x, y, cw, ch], first, changed.is_none());
        self.code(cw * ch);
        let pending = &mut self.pending;
        // Graphic control: leave the frame in place, transparency when anything stays.
        pending.extend_from_slice(&[0x21, 0xf9, 4, (1 << 2) | u8::from(!first)]);
        self.delay = Some((pending.len(), step));
        pending.extend_from_slice(&(step as u16).to_le_bytes());
        pending.extend_from_slice(&[self.transparent, 0]);
        pending.push(0x2c);
        for v in [x, y, cw, ch] {
            pending.extend_from_slice(&(v as u16).to_le_bytes());
        }
        // A local table just large enough for the colors and the transparent index.
        pending.push(0x80 | self.bits as u8);
        for color in &self.palette[..2 << self.bits] {
            pending.extend_from_slice(color);
        }
        pending.push(self.bits.max(1) as u8 + 1);
        for block in self.coded.chunks(255) {
            pending.push(block.len() as u8);
            pending.extend_from_slice(block);
        }
        pending.push(0);
        self.previous.clear();
        self.previous.extend_from_slice(rgba);
        Ok(())
    }

    /// Writes the last frame and the trailer.
    pub(super) fn finish(&mut self, out: &mut dyn Write) -> io::Result<()> {
        if self.frames == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "A GIF needs at least one frame.",
            ));
        }
        self.pending.push(0x3b);
        self.flush(out)?;
        out.flush()
    }

    fn flush(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.pending)?;
        self.written += self.pending.len() as u64;
        self.pending.clear();
        self.delay = None;
        Ok(())
    }

    /// The logical screen (no global table) and the loop-forever extension.
    fn header(&mut self) {
        let [w, h] = self.size;
        self.pending.extend_from_slice(b"GIF89a");
        self.pending.extend_from_slice(&(w as u16).to_le_bytes());
        self.pending.extend_from_slice(&(h as u16).to_le_bytes());
        self.pending.extend_from_slice(&[0, 0, 0]);
        self.pending.extend_from_slice(&[0x21, 0xff, 11]);
        self.pending.extend_from_slice(b"NETSCAPE2.0");
        self.pending.extend_from_slice(&[3, 1, 0, 0, 0]);
    }

    /// The bounding box `[x, y, w, h]` of pixels that differ from the previous frame.
    fn changed(&self, rgba: &[u8]) -> Option<[usize; 4]> {
        let [w, h] = self.size;
        let row = w * 4;
        let differs =
            |y: usize| rgba[y * row..(y + 1) * row] != self.previous[y * row..(y + 1) * row];
        let top = (0..h).find(|&y| differs(y))?;
        let bottom = (top..h).rev().find(|&y| differs(y)).unwrap_or(top);
        let (mut left, mut right) = (w, 0);
        for y in top..=bottom {
            let (a, b) = (
                &rgba[y * row..(y + 1) * row],
                &self.previous[y * row..(y + 1) * row],
            );
            if let Some(x) = (0..left).find(|&x| a[x * 4..x * 4 + 3] != b[x * 4..x * 4 + 3]) {
                left = x;
            }
            if let Some(x) = (right..w)
                .rev()
                .find(|&x| a[x * 4..x * 4 + 3] != b[x * 4..x * 4 + 3])
            {
                right = right.max(x);
            }
        }
        // Rows that differ only in alpha change nothing visible.
        (left <= right).then_some([left, top, right - left + 1, bottom - top + 1])
    }

    /// Builds the palette for the changed pixels in `rect` and fills `indices`.
    fn quantize(&mut self, rgba: &[u8], [x0, y0, cw, ch]: [usize; 4], first: bool, empty: bool) {
        let w = self.size[0];
        let bin = |p: &[u8]| {
            (usize::from(p[0] >> 3) << 10) | (usize::from(p[1] >> 3) << 5) | usize::from(p[2] >> 3)
        };
        // Moved out while the palette is built (no copy, no allocation), put back below.
        let previous = std::mem::take(&mut self.previous);
        let same = |at: usize| !first && rgba[at..at + 3] == previous[at..at + 3];
        self.bins.clear();
        if !empty {
            for y in y0..y0 + ch {
                for x in x0..x0 + cw {
                    let at = (y * w + x) * 4;
                    if same(at) {
                        continue;
                    }
                    let b = bin(&rgba[at..at + 3]);
                    if self.counts[b] == 0 {
                        self.bins.push(b as u16);
                    }
                    self.counts[b] += 1;
                    for c in 0..3 {
                        self.sums[b][c] += u32::from(rgba[at + c]);
                    }
                }
            }
        }
        self.cut();
        self.transparent = self.cuts.len() as u8;
        self.bits = (0..7).find(|bits| 2 << bits > self.cuts.len()).unwrap_or(7);
        let transparent = self.transparent;
        self.indices.clear();
        for y in y0..y0 + ch {
            for x in x0..x0 + cw {
                let at = (y * w + x) * 4;
                self.indices.push(if empty || same(at) {
                    transparent
                } else {
                    self.map[bin(&rgba[at..at + 3])]
                });
            }
        }
        for &b in &self.bins {
            self.counts[usize::from(b)] = 0;
            self.sums[usize::from(b)] = [0; 3];
        }
        self.previous = previous;
    }

    /// Median cut of the present bins into at most [`COLORS`] boxes; each box's color is the mean
    /// of its pixels and every bin in it maps there.
    fn cut(&mut self) {
        self.cuts.clear();
        if !self.bins.is_empty() {
            let cut = Cut::measure(&self.bins, &self.counts, 0, self.bins.len());
            self.cuts.push(cut);
        }
        while self.cuts.len() < COLORS {
            // Split the box with the most pixels times its widest channel range.
            let Some((index, cut)) = self
                .cuts
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, cut)| cut.score > 0)
                .max_by_key(|(_, cut)| cut.score)
            else {
                break;
            };
            let axis = cut.axis;
            let bins = &mut self.bins[cut.start..cut.end];
            bins.sort_unstable_by_key(|&b| channel(b, axis));
            // The median pixel, leaving both halves non-empty.
            let mut seen = 0;
            let mut split = 1;
            for (i, &b) in bins.iter().enumerate() {
                seen += u64::from(self.counts[usize::from(b)]);
                if seen * 2 >= cut.pixels {
                    split = (i + 1).clamp(1, bins.len() - 1);
                    break;
                }
            }
            let middle = cut.start + split;
            self.cuts[index] = Cut::measure(&self.bins, &self.counts, cut.start, middle);
            let right = Cut::measure(&self.bins, &self.counts, middle, cut.end);
            self.cuts.push(right);
        }
        self.palette = [[0; 3]; 256];
        for (index, cut) in self.cuts.iter().enumerate() {
            let mut sum = [0u64; 3];
            for &b in &self.bins[cut.start..cut.end] {
                let b = usize::from(b);
                for (c, total) in sum.iter_mut().enumerate() {
                    *total += u64::from(self.sums[b][c]);
                }
                self.map[b] = index as u8;
            }
            let pixels = cut.pixels.max(1);
            self.palette[index] = sum.map(|total| ((total + pixels / 2) / pixels) as u8);
        }
    }

    /// LZW-codes the first `count` indices into `coded` (minimum code size: the table's bits,
    /// at least 2).
    fn code(&mut self, count: usize) {
        let min = self.bits.max(1) + 1;
        let clear = 1u32 << min;
        let end = clear + 1;
        let mut bits = 0u64;
        let mut filled = 0u32;
        let mut width = min + 1;
        let mut next = clear + 2;
        self.coded.clear();
        self.reset_table();
        let coded = &mut self.coded;
        let mut emit = |code: u32, width: u32| {
            bits |= u64::from(code) << filled;
            filled += width;
            while filled >= 8 {
                coded.push(bits as u8);
                bits >>= 8;
                filled -= 8;
            }
        };
        emit(clear, width);
        let mut current = u32::from(self.indices[0]);
        for i in 1..count {
            let pixel = u32::from(self.indices[i]);
            let key = (current << 8) | pixel;
            let tag = (self.generation << 20) | key;
            let mut slot = (key.wrapping_mul(0x9e37_79b1) >> 19) as usize & (SLOTS - 1);
            let found = loop {
                if self.keys[slot] == tag {
                    break Some(u32::from(self.codes[slot]));
                }
                if self.keys[slot] >> 20 != self.generation {
                    break None;
                }
                slot = (slot + 1) & (SLOTS - 1);
            };
            if let Some(code) = found {
                current = code;
                continue;
            }
            emit(current, width);
            // Giflib's order: the width grows once the next free code no longer fits.
            if next >= 1 << width && width < 12 {
                width += 1;
            }
            current = pixel;
            if next >= 4095 {
                emit(clear, width);
                width = min + 1;
                next = clear + 2;
                self.generation += 1;
                if self.generation >= 1 << 12 {
                    self.keys.fill(0);
                    self.generation = 1;
                }
            } else {
                self.keys[slot] = tag;
                self.codes[slot] = next as u16;
                next += 1;
            }
        }
        emit(current, width);
        if next >= 1 << width && width < 12 {
            width += 1;
        }
        emit(end, width);
        if filled > 0 {
            coded.push(bits as u8);
        }
    }

    fn reset_table(&mut self) {
        self.generation += 1;
        if self.generation >= 1 << 12 {
            self.keys.fill(0);
            self.generation = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal GIF decoder for the tests: frames composited onto the canvas, with delays.
    fn decode(gif: &[u8]) -> (Vec<Vec<[u8; 3]>>, Vec<u16>, [usize; 2]) {
        assert_eq!(&gif[..6], b"GIF89a");
        let w = usize::from(u16::from_le_bytes([gif[6], gif[7]]));
        let h = usize::from(u16::from_le_bytes([gif[8], gif[9]]));
        assert_eq!(gif[10] & 0x80, 0, "no global table");
        let mut at = 13;
        let mut canvas = vec![[0u8; 3]; w * h];
        let (mut frames, mut delays) = (Vec::new(), Vec::new());
        let mut transparent = None;
        loop {
            match gif[at] {
                0x21 => {
                    let label = gif[at + 1];
                    at += 2;
                    if label == 0xf9 {
                        assert_eq!(gif[at], 4);
                        delays.push(u16::from_le_bytes([gif[at + 2], gif[at + 3]]));
                        transparent = (gif[at + 1] & 1 == 1).then_some(gif[at + 4]);
                    }
                    while gif[at] != 0 {
                        at += usize::from(gif[at]) + 1;
                    }
                    at += 1;
                }
                0x2c => {
                    let field =
                        |i: usize| usize::from(u16::from_le_bytes([gif[at + i], gif[at + i + 1]]));
                    let (x, y, fw, fh) = (field(1), field(3), field(5), field(7));
                    let packed = gif[at + 9];
                    assert_eq!(packed & 0x80, 0x80, "local table");
                    let entries = 2usize << (packed & 7);
                    let table: Vec<[u8; 3]> = gif[at + 10..at + 10 + entries * 3]
                        .chunks(3)
                        .map(|c| [c[0], c[1], c[2]])
                        .collect();
                    at += 10 + entries * 3;
                    let min = u32::from(gif[at]);
                    at += 1;
                    let mut data = Vec::new();
                    while gif[at] != 0 {
                        let len = usize::from(gif[at]);
                        data.extend_from_slice(&gif[at + 1..at + 1 + len]);
                        at += len + 1;
                    }
                    at += 1;
                    let indices = lzw_decode(&data, min);
                    assert_eq!(indices.len(), fw * fh);
                    for (i, &index) in indices.iter().enumerate() {
                        if Some(index) != transparent {
                            canvas[(y + i / fw) * w + x + i % fw] = table[usize::from(index)];
                        }
                    }
                    frames.push(canvas.clone());
                }
                0x3b => break,
                other => panic!("unexpected block {other:#x}"),
            }
        }
        (frames, delays, [w, h])
    }

    fn lzw_decode(data: &[u8], min: u32) -> Vec<u8> {
        let clear = 1u32 << min;
        let end = clear + 1;
        let mut table: Vec<Vec<u8>> = Vec::new();
        let reset = |table: &mut Vec<Vec<u8>>| {
            table.clear();
            table.extend((0..clear + 2).map(|i| vec![i as u8]));
        };
        reset(&mut table);
        let (mut width, mut bit, mut out) = (min + 1, 0usize, Vec::new());
        let mut previous: Option<Vec<u8>> = None;
        loop {
            let mut code = 0u32;
            for i in 0..width {
                let byte = data[(bit + i as usize) / 8];
                code |= u32::from((byte >> ((bit + i as usize) % 8)) & 1) << i;
            }
            bit += width as usize;
            if code == clear {
                reset(&mut table);
                width = min + 1;
                previous = None;
                continue;
            }
            if code == end {
                return out;
            }
            let entry = match (&previous, table.get(code as usize)) {
                (_, Some(entry)) => entry.clone(),
                (Some(prev), None) => {
                    let mut e = prev.clone();
                    e.push(prev[0]);
                    e
                }
                (None, None) => panic!("bad first code {code}"),
            };
            out.extend_from_slice(&entry);
            if let Some(mut prev) = previous.take()
                && table.len() < 4096
            {
                prev.push(entry[0]);
                table.push(prev);
            }
            if table.len() == 1 << width && width < 12 {
                width += 1;
            }
            previous = Some(entry);
        }
    }

    fn write(frames: &[Vec<u8>], size: [u32; 2], fps: u32) -> Vec<u8> {
        let mut writer = GifWriter::new(size, fps);
        let mut out = Vec::new();
        for frame in frames {
            writer.frame(frame, &mut out).unwrap();
        }
        writer.finish(&mut out).unwrap();
        assert_eq!(writer.written(), out.len() as u64);
        out
    }

    fn rgb(frame: &[u8]) -> Vec<[u8; 3]> {
        frame.chunks(4).map(|p| [p[0], p[1], p[2]]).collect()
    }

    /// Few colors (a screen recording) round-trip exactly; only changed rectangles are coded,
    /// a repeated frame lengthens the previous delay, and 24 fps keeps time in hundredths.
    #[test]
    fn frames_round_trip_with_changed_rectangles_and_exact_delays() {
        let size = [40u32, 30];
        let mut frames = Vec::new();
        let mut frame: Vec<u8> = (0..40 * 30)
            .flat_map(|i| {
                let c = [[250, 250, 250], [20, 20, 30], [0, 120, 255], [255, 80, 0]]
                    [i % 4 / 2 + i / 600];
                [c[0], c[1], c[2], 255]
            })
            .collect();
        frames.push(frame.clone());
        for step in 0..6 {
            // A small "cursor" moves; one frame repeats.
            if step != 3 {
                let at = (5 + step * 3) * 40 + 4 + step * 5;
                for p in 0..4 {
                    let i = (at + p) * 4;
                    // Distinct 5-bit bins, so the colors stay exact.
                    frame[i..i + 3].copy_from_slice(&[(step * 40 + p * 8) as u8, 0, 0]);
                }
            }
            frames.push(frame.clone());
        }
        let gif = write(&frames, size, 24);
        let (decoded, delays, dims) = decode(&gif);
        assert_eq!(dims, [40, 30]);
        // Frame 4 repeated frame 3: one fewer image, its time added to the one before.
        assert_eq!(decoded.len(), 6);
        assert_eq!(delays, [4, 4, 5, 8, 4, 4]);
        let kept = [0, 1, 2, 3, 5, 6];
        for (image, &source) in decoded.iter().zip(&kept) {
            assert_eq!(*image, rgb(&frames[source]), "frame {source}");
        }
        // Header, then a full first frame, then only the moved pixels (4×1 rectangles).
        assert!(gif.len() < 600, "{} bytes", gif.len());
    }

    /// More than 255 colors: the palette is a median cut, every pixel lands near its color,
    /// and a long run of codes (table resets) decodes.
    #[test]
    fn many_colors_quantize_close_and_long_frames_decode() {
        let size = [128u32, 96];
        let frames: Vec<Vec<u8>> = (0..3)
            .map(|f| {
                (0..128 * 96)
                    .flat_map(|i| {
                        let (x, y) = (i % 128, i / 128);
                        [
                            (x * 2) as u8,
                            (y * 2 + f * 9) as u8,
                            ((x ^ y) * 3) as u8,
                            255,
                        ]
                    })
                    .collect()
            })
            .collect();
        let gif = write(&frames, size, 15);
        let (decoded, delays, _) = decode(&gif);
        assert_eq!(delays, [7, 6, 7]);
        for (image, frame) in decoded.iter().zip(&frames) {
            let worst = image
                .iter()
                .zip(rgb(frame))
                .map(|(a, b)| (0..3).map(|c| a[c].abs_diff(b[c]) as u32).sum::<u32>())
                .max()
                .unwrap();
            // About 6 levels a channel on average for ~10k distinct colors in 255 entries.
            assert!(worst <= 150, "worst channel error sum {worst}");
            let mean: f64 = image
                .iter()
                .zip(rgb(frame))
                .map(|(a, b)| (0..3).map(|c| f64::from(a[c].abs_diff(b[c]))).sum::<f64>())
                .sum::<f64>()
                / image.len() as f64;
            assert!(mean < 22.0, "mean error {mean}");
        }
    }

    /// Steady state: frames after the first allocate nothing (buffers are sized up front).
    #[test]
    fn warm_frames_allocate_nothing() {
        let size = [96u32, 64];
        let frames: Vec<Vec<u8>> = (0..4)
            .map(|f| {
                (0..96 * 64)
                    .flat_map(|i| {
                        [
                            (i * 7 + f * 31) as u8,
                            (i / 96 * 4) as u8,
                            (f * 50) as u8,
                            255,
                        ]
                    })
                    .collect()
            })
            .collect();
        let mut writer = GifWriter::new(size, 24);
        let mut sink = io::sink();
        writer.frame(&frames[0], &mut sink).unwrap();
        let mut f = 0;
        splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
            &mut || {
                f += 1;
                writer.frame(&frames[f % 4], &mut sink).unwrap();
            },
        );
        writer.finish(&mut sink).unwrap();
    }

    /// AC-ED-2 for GIFs: Apple's decoder (ImageIO, what Safari and Preview use) reads back the
    /// rendered values exactly; the writer stores them verbatim, no colour tags (GIF is sRGB).
    #[cfg(target_os = "macos")]
    #[test]
    fn imageio_decodes_the_written_colors_exactly() {
        const PATCHES: [[u8; 3]; 8] = [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [220, 60, 30],
            [128, 128, 128],
            [16, 16, 16],
            [40, 180, 80],
            [255, 255, 255],
        ];
        let picture: Vec<u8> = (0..64)
            .flat_map(|y| {
                (0..96).flat_map(move |x| {
                    let [r, g, b] = PATCHES[y / 32 * 4 + x / 24];
                    [r, g, b, 255]
                })
            })
            .collect();
        let gif = write(std::slice::from_ref(&picture), [96, 64], 15);
        let dir = std::env::temp_dir().join(format!("splice-gif-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("patches.gif");
        std::fs::write(&file, gif).unwrap();
        let decoded = apple_sys::image::decode_rgba(&file);
        let _ = std::fs::remove_dir_all(&dir);
        let (decoded, size) = decoded.unwrap();
        assert_eq!(size, [96, 64]);
        assert_eq!(rgb(&decoded), rgb(&picture));
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_refused() {
        let mut writer = GifWriter::new([4, 4], 15);
        let mut out = Vec::new();
        assert!(writer.frame(&[0; 12], &mut out).is_err());
        assert!(writer.finish(&mut out).is_err());
    }
}
