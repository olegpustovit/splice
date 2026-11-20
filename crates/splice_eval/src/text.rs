//! Text units for animators (T42.3): words, lines and user-perceived characters. Characters are
//! extended grapheme clusters (UAX #29) so a ZWJ family, a flag, a skin-toned or keycap emoji
//! and a letter with combining marks animate as one unit. A small rule set with compact tables
//! (no Unicode database): CR LF, Extend/ZWJ/SpacingMark (GB9, GB9a), emoji ZWJ sequences
//! (GB11) and regional-indicator pairs (GB12, GB13).

/// UTF-16 ranges of the words (0), characters (1) or lines (2) of `text`, skipping whitespace.
pub fn text_units(text: &str, unit: u8) -> Vec<[usize; 2]> {
    let mut out = Vec::new();
    if unit == 1 {
        let (mut at, mut rest) = (0, text);
        while !rest.is_empty() {
            let (cluster, tail) = rest.split_at(grapheme_len(rest));
            let width: usize = cluster.chars().map(char::len_utf16).sum();
            if !cluster.chars().all(char::is_whitespace) {
                out.push([at, at + width]);
            }
            at += width;
            rest = tail;
        }
        return out;
    }
    let mut start = None;
    let mut at = 0;
    for c in text.chars() {
        let inside = if unit == 2 {
            c != '\n'
        } else {
            !c.is_whitespace()
        };
        if inside {
            start.get_or_insert(at);
        } else if let Some(s) = start.take() {
            out.push([s, at]);
        }
        at += c.len_utf16();
    }
    if let Some(s) = start {
        out.push([s, at]);
    }
    out
}

/// Byte length of the extended grapheme cluster that starts `text` (0 when it is empty).
pub fn grapheme_len(text: &str) -> usize {
    let mut chars = text.char_indices();
    let Some((_, first)) = chars.next() else {
        return 0;
    };
    let mut previous = first;
    // GB11: inside ExtPict Extend* ZWJ? GB12/13: regional indicators so far in the run.
    let mut emoji = pictographic(first);
    let mut regionals = usize::from(regional(first));
    for (index, c) in chars {
        let joins = if previous == '\r' {
            c == '\n'
        } else if control(previous) || control(c) {
            false
        } else {
            extend(c)
                || c == ZWJ
                || (previous == ZWJ && emoji && pictographic(c))
                || (regional(previous) && regional(c) && regionals % 2 == 1)
        };
        if !joins {
            return index;
        }
        if pictographic(c) {
            emoji = true;
        } else if !(extend(c) || c == ZWJ) {
            emoji = false;
        }
        regionals = if regional(c) { regionals + 1 } else { 0 };
        previous = c;
    }
    text.len()
}

const ZWJ: char = '\u{200d}';

fn control(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')
}

fn regional(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

fn within(c: char, ranges: &[(u32, u32)]) -> bool {
    let c = u32::from(c);
    let index = ranges.partition_point(|&(_, hi)| hi < c);
    ranges.get(index).is_some_and(|&(lo, _)| lo <= c)
}

/// Grapheme_Extend and SpacingMark for the scripts titles use: combining marks, Hebrew and
/// Arabic points, the Indic vowel signs and viramas, Thai, kana voicing, Hangul medial and final
/// jamo, variation selectors, emoji modifiers and tags.
fn extend(c: char) -> bool {
    let code = u32::from(c);
    // Indic blocks share a layout: signs at 00–03, 3A–4F (minus a few letters), 51–57, 62–63.
    if (0x0900..=0x0d7f).contains(&code) {
        let offset = code & 0x7f;
        return matches!(offset, 0x00..=0x03 | 0x3a..=0x4d | 0x4f | 0x51..=0x57 | 0x62..=0x63)
            && !matches!(offset, 0x3d);
    }
    within(c, EXTEND)
}

const EXTEND: &[(u32, u32)] = &[
    (0x0300, 0x036f),
    (0x0483, 0x0489),
    (0x0591, 0x05bd),
    (0x05bf, 0x05bf),
    (0x05c1, 0x05c2),
    (0x05c4, 0x05c5),
    (0x05c7, 0x05c7),
    (0x0610, 0x061a),
    (0x064b, 0x065f),
    (0x0670, 0x0670),
    (0x06d6, 0x06dc),
    (0x06df, 0x06e4),
    (0x06e7, 0x06e8),
    (0x06ea, 0x06ed),
    (0x0e31, 0x0e31),
    (0x0e34, 0x0e3a),
    (0x0e47, 0x0e4e),
    (0x1160, 0x11ff),
    (0x1ab0, 0x1aff),
    (0x1dc0, 0x1dff),
    (0x200c, 0x200c),
    (0x20d0, 0x20ff),
    (0x302a, 0x302f),
    (0x3099, 0x309a),
    (0xfe00, 0xfe0f),
    (0xfe20, 0xfe2f),
    (0x1f3fb, 0x1f3ff),
    (0xe0020, 0xe007f),
    (0xe0100, 0xe01ef),
];

/// Extended_Pictographic, by block (the emoji blocks whole, less regional indicators and skin
/// tone modifiers, which have their own rules).
fn pictographic(c: char) -> bool {
    within(c, PICTOGRAPHIC)
}

const PICTOGRAPHIC: &[(u32, u32)] = &[
    (0x00a9, 0x00a9),
    (0x00ae, 0x00ae),
    (0x203c, 0x203c),
    (0x2049, 0x2049),
    (0x2122, 0x2122),
    (0x2139, 0x2139),
    (0x2194, 0x2199),
    (0x21a9, 0x21aa),
    (0x231a, 0x231b),
    (0x2328, 0x2328),
    (0x23cf, 0x23cf),
    (0x23e9, 0x23f3),
    (0x23f8, 0x23fa),
    (0x24c2, 0x24c2),
    (0x25aa, 0x25ab),
    (0x25b6, 0x25b6),
    (0x25c0, 0x25c0),
    (0x25fb, 0x25fe),
    (0x2600, 0x27bf),
    (0x2934, 0x2935),
    (0x2b05, 0x2b07),
    (0x2b1b, 0x2b1c),
    (0x2b50, 0x2b50),
    (0x2b55, 0x2b55),
    (0x3030, 0x3030),
    (0x303d, 0x303d),
    (0x3297, 0x3297),
    (0x3299, 0x3299),
    (0x1f000, 0x1f1e5),
    (0x1f200, 0x1f3fa),
    (0x1f400, 0x1faff),
    (0x1fc00, 0x1fffd),
];

/// A unit's raster cut to its ink (T45.4): `rgba` is a `size` RGBA8 image of the whole text box
/// with one unit drawn; returns the `[x, y, w, h]` box of the pixels with any alpha (shadow and
/// stroke included) and those pixels, or `None` when nothing is drawn. A per-character animator
/// then uploads and composites each glyph's own bounds instead of a full box per character.
pub fn ink_crop(rgba: &[u8], [width, height]: [u32; 2]) -> Option<([u32; 4], Vec<u8>)> {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() != w * h * 4 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (y, row) in rgba.chunks_exact(w * 4).enumerate() {
        let alpha = || row.iter().skip(3).step_by(4);
        let Some(first) = alpha().position(|&a| a != 0) else {
            continue;
        };
        let last = alpha().rposition(|&a| a != 0)?;
        x0 = x0.min(first);
        x1 = x1.max(last + 1);
        y0 = y0.min(y);
        y1 = y + 1;
    }
    if x1 <= x0 {
        return None;
    }
    let mut pixels = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for row in rgba.chunks_exact(w * 4).take(y1).skip(y0) {
        pixels.extend_from_slice(&row[x0 * 4..x1 * 4]);
    }
    let rect = [x0, y0, x1 - x0, y1 - y0].map(|v| v as u32);
    Some((rect, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_crop_keeps_only_the_drawn_pixels() {
        let (w, h) = (6, 4);
        let mut image = vec![0u8; w * h * 4];
        assert_eq!(ink_crop(&image, [6, 4]), None);
        for (x, y, alpha) in [(2, 1, 255u8), (4, 2, 7)] {
            image[(y * w + x) * 4..][..4].copy_from_slice(&[9, 8, 7, alpha]);
        }
        let (rect, pixels) = ink_crop(&image, [6, 4]).unwrap();
        assert_eq!(rect, [2, 1, 3, 2]);
        assert_eq!(pixels.len(), 3 * 2 * 4);
        assert_eq!(&pixels[..4], &[9, 8, 7, 255]);
        assert_eq!(&pixels[pixels.len() - 4..], &[9, 8, 7, 7]);
        assert_eq!(ink_crop(&image[4..], [6, 4]), None, "wrong size");
    }

    /// T42.3: ZWJ families, flags, skin tones, keycaps, tags and combining marks stay whole;
    /// unit ranges are UTF-16 (what the native text renderer takes).
    #[test]
    fn graphemes_keep_emoji_sequences_and_marks_whole() {
        let cases: &[(&str, &[&str])] = &[
            ("abc", &["a", "b", "c"]),
            ("👨‍👩‍👧‍👦!", &["👨‍👩‍👧‍👦", "!"]),
            ("🇺🇦🇯🇵🇺", &["🇺🇦", "🇯🇵", "🇺"]),
            ("👍🏽ok", &["👍🏽", "o", "k"]),
            ("1️⃣2", &["1️⃣", "2"]),
            ("🏴󠁧󠁢󠁳󠁣󠁴󠁿x", &["🏴󠁧󠁢󠁳󠁣󠁴󠁿", "x"]),
            ("❤️‍🔥", &["❤️‍🔥"]),
            ("e\u{301}te\u{301}", &["e\u{301}", "t", "e\u{301}"]),
            ("नमस्ते", &["न", "म", "स्", "ते"]),
            ("a\r\nb", &["a", "\r\n", "b"]),
            ("\n\u{301}", &["\n", "\u{301}"]),
            ("🐉 cat", &["🐉", " ", "c", "a", "t"]),
            ("a\u{200d}b", &["a\u{200d}", "b"]),
        ];
        for (text, expected) in cases {
            let mut got = Vec::new();
            let mut rest = *text;
            while !rest.is_empty() {
                let (cluster, tail) = rest.split_at(grapheme_len(rest));
                got.push(cluster);
                rest = tail;
            }
            assert_eq!(&got, expected, "{text:?}");
        }
        // Characters skip whitespace; ranges count UTF-16 units (🐉 is two).
        assert_eq!(text_units("🐉 hi", 1), [[0, 2], [3, 4], [4, 5]]);
        assert_eq!(text_units("👨‍👩‍👧 go", 1), [[0, 8], [9, 10], [10, 11]]);
        assert_eq!(
            text_units("cat ➜ DRAGON 🐉", 0),
            [[0, 3], [4, 5], [6, 12], [13, 15]]
        );
        assert_eq!(text_units("one two\nthree", 2), [[0, 7], [8, 13]]);
    }
}
