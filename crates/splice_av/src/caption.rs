//! Text and UTF-16 run boundaries for the native caption raster.
use apple_sys::composition::CaptionSpan;
use splice_eval::{CaptionPreset, CaptionWord, grapheme_len};
use splice_model::Frame;

pub(crate) struct CaptionText {
    pub text: String,
    pub active: Option<[usize; 2]>,
    pub spans: Vec<CaptionSpan>,
    pub until: Frame,
}

/// Prepares one raster and the first subsequent frame that needs a different raster.
/// The lowerer already cuts on word starts, so only Pop and Typewriter add local variants.
pub(crate) fn caption_text(
    words: &[CaptionWord],
    word: Option<usize>,
    preset: Option<CaptionPreset>,
    [at, to]: [Frame; 2],
) -> CaptionText {
    let mut text = String::new();
    let mut active = None;
    let mut spans = Vec::new();
    let mut end = 0;
    let mut until = to;
    for (index, token) in words.iter().enumerate() {
        if !text.is_empty() {
            text.push(' ');
            end += 1;
        }
        let start = end;
        let token_bytes = text.len();
        text.push_str(&token.text);
        end += token.text.encode_utf16().count();
        let now = word == Some(index);
        if now && preset.is_none() {
            active = Some([start, end]);
        }
        if let Some(emoji) = &token.emoji {
            if preset.is_some() && !emoji.is_empty() {
                text.push(' ');
                end += 1;
            }
            text.push_str(emoji);
            end += emoji.encode_utf16().count();
        }
        if let Some(preset) = preset
            && start < end
        {
            let spoken = word.is_some_and(|word| index <= word);
            let mut paint = CaptionSpan::new([start, end], if token.emph { 1.12 } else { 1.0 });
            paint.highlight = u32::from(token.emph);
            paint.reserve = paint.scale
                * (if matches!(preset, CaptionPreset::Bold | CaptionPreset::Pop) {
                    1.1
                } else {
                    1.0
                });
            match preset {
                CaptionPreset::Bold if now => {
                    paint.scale *= 1.1;
                    paint.highlight = 1;
                }
                CaptionPreset::Karaoke => {
                    paint.highlight = u32::from(spoken);
                    paint.opacity = if spoken { 1.0 } else { 0.5 };
                }
                CaptionPreset::Pop if now => {
                    let k = (f64::from(at) - f64::from(token.start)).clamp(0.0, 4.0) / 4.0;
                    let d = k - 1.0;
                    paint.scale *= 0.6 + 0.4 * (1.0 + 2.70158 * d * d * d + 1.70158 * d * d);
                    paint.highlight = 1;
                    if k < 1.0 {
                        until = until.min(at.saturating_add(1));
                    }
                }
                CaptionPreset::Pop | CaptionPreset::Typewriter if !spoken => paint.opacity = 0.0,
                CaptionPreset::Box if now => {
                    paint.highlight = 2;
                }
                CaptionPreset::Typewriter if now => {
                    let (length, next) = revealed(&text[token_bytes..], token.start, token.end, at);
                    paint.visible_to = start + length;
                    until = until.min(next);
                }
                _ => {}
            }
            spans.push(paint);
        }
    }
    CaptionText {
        text,
        active,
        spans,
        until,
    }
}

/// Browser timing is in UTF-16 code units. Stop on a whole grapheme so a partial
/// surrogate, combining character or ZWJ emoji never changes into a replacement glyph.
fn revealed(text: &str, from: Frame, to: Frame, at: Frame) -> (usize, Frame) {
    let len = text.encode_utf16().count();
    let duration = (i64::from(to) - i64::from(from)).max(1) as u64;
    let elapsed = (i64::from(at) - i64::from(from) + 1).clamp(1, duration as i64) as u64;
    let requested = ((len as u128 * elapsed as u128).div_ceil(duration as u128) as usize).max(1);
    if requested >= len {
        return (len, Frame::MAX);
    }
    let mut rest = text;
    let mut end = 0;
    while !rest.is_empty() {
        let (cluster, tail) = rest.split_at(grapheme_len(rest));
        let next = end + cluster.encode_utf16().count();
        if next > requested && end != 0 {
            let offset = (next as u128 - 1) * duration as u128 / len as u128;
            return (
                end,
                (i64::from(from) + offset as i64).min(i64::from(Frame::MAX)) as Frame,
            );
        }
        end = next;
        rest = tail;
    }
    (end, Frame::MAX)
}
