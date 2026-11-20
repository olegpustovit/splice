//! Keystroke chips (A1, A11, Recorder's overlay): which category a recorded key belongs to, the
//! categories `keys.set {show}` picks, and each chip's text. Portable so takes.events, the
//! KEYS_SHOW_TYPING lint and the renderer agree on what draws.
use crate::activity::SHORTCUT_MODIFIERS;
use splice_model::{Effect, EffectValue};

/// Keys pressed with ⌘, ⌃ or ⌥.
pub const SHORTCUTS: u8 = 1;
/// Printable keys (Space and ⇧+letter included): typed text.
pub const TYPING: u8 = 2;
/// Navigation and editing keys: arrows, ⌫ ⌦ ⏎ ⇥ ⎋, Home/End/Page Up/Page Down, F-keys.
pub const NAVIGATION: u8 = 4;
/// A modifier pressed and released alone.
pub const MODIFIERS: u8 = 8;
/// Every category (`show:"all"`).
pub const ALL: u8 = 15;

/// Category names (`show` parts) and the singular takes.events `category`, by bit.
pub const CATEGORIES: [(&str, &str, u8); 4] = [
    ("shortcuts", "shortcut", SHORTCUTS),
    ("typing", "typing", TYPING),
    ("navigation", "navigation", NAVIGATION),
    ("modifiers", "modifier", MODIFIERS),
];

/// The canonical `show` value of each category set, by bit set (`effects.json` lists 1…15).
pub const SHOW: [&str; 16] = [
    "none",
    "shortcuts",
    "typing",
    "shortcuts+typing",
    "navigation",
    "shortcuts+navigation",
    "typing+navigation",
    "shortcuts+typing+navigation",
    "modifiers",
    "shortcuts+modifiers",
    "typing+modifiers",
    "shortcuts+typing+modifiers",
    "navigation+modifiers",
    "shortcuts+navigation+modifiers",
    "typing+navigation+modifiers",
    "all",
];

/// Category bits of a `show` value: `all`, `none`, or category names joined by `+` in any order.
pub fn parse_show(text: &str) -> Option<u8> {
    match text {
        "all" => return Some(ALL),
        "none" => return Some(0),
        _ => {}
    }
    let mut bits = 0;
    for part in text.split('+') {
        let bit = CATEGORIES.iter().find(|c| c.0 == part)?.2;
        if bits & bit != 0 {
            return None;
        }
        bits |= bit;
    }
    Some(bits)
}

/// The categories a clip's chips draw, from its effect scopes (project, spine, clip): 0 when the
/// `keystrokes` effect is off. Callers check the clip's keys component.
pub fn shown(scopes: &[&[Effect]]) -> u8 {
    match crate::resolve_effect(scopes, "keystrokes") {
        Some((false, _)) => 0,
        Some((true, params)) => params
            .iter()
            .find(|param| param.name == "show")
            .and_then(|param| match &param.value {
                EffectValue::Text(text) => parse_show(text),
                _ => None,
            })
            .unwrap_or(ALL),
        None => ALL,
    }
}

/// Shift, control, option and command (NSEvent modifier flags).
const MODIFIER_KEYS: u64 = (1 << 17) | SHORTCUT_MODIFIERS;

/// Glyphs for navigation and editing keys by macOS virtual key code.
const NAVIGATION_KEYS: [(u64, &str); 34] = [
    (36, "⏎"),
    (76, "⌤"),
    (48, "⇥"),
    (51, "⌫"),
    (117, "⌦"),
    (53, "⎋"),
    (115, "↖"),
    (119, "↘"),
    (116, "⇞"),
    (121, "⇟"),
    (123, "←"),
    (124, "→"),
    (125, "↓"),
    (126, "↑"),
    (122, "F1"),
    (120, "F2"),
    (99, "F3"),
    (118, "F4"),
    (96, "F5"),
    (97, "F6"),
    (98, "F7"),
    (100, "F8"),
    (101, "F9"),
    (109, "F10"),
    (103, "F11"),
    (111, "F12"),
    (105, "F13"),
    (107, "F14"),
    (113, "F15"),
    (106, "F16"),
    (64, "F17"),
    (79, "F18"),
    (80, "F19"),
    (90, "F20"),
];

/// A navigation or editing key's glyph, by key code or (logs without codes) by the control
/// character it types.
pub fn navigation_glyph(code: Option<u64>, text: &str) -> Option<&'static str> {
    if let Some(code) = code {
        return NAVIGATION_KEYS
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, glyph)| *glyph);
    }
    Some(match text.chars().next()? {
        '\r' | '\u{3}' => "⏎",
        '\t' => "⇥",
        '\u{8}' | '\u{7f}' => "⌫",
        '\u{1b}' => "⎋",
        '\u{1c}' | '\u{f702}' => "←",
        '\u{1d}' | '\u{f703}' => "→",
        '\u{1e}' | '\u{f700}' => "↑",
        '\u{1f}' | '\u{f701}' => "↓",
        _ => return None,
    })
}

/// The category bit of a recorded key event (`keyCode`, `mods`, `ch`).
pub fn category(code: Option<u64>, mods: u64, text: &str) -> u8 {
    if mods & SHORTCUT_MODIFIERS != 0 {
        SHORTCUTS
    } else if navigation_glyph(code, text).is_some() {
        NAVIGATION
    } else {
        TYPING
    }
}

/// Whether a key event makes a chip at all: something printable or a navigation key.
pub fn visible(code: Option<u64>, text: &str) -> bool {
    navigation_glyph(code, text).is_some()
        || text.chars().any(|c| !c.is_control() && !c.is_whitespace())
        || text == " "
}

fn modifier_glyphs(mods: u64, chip: &mut String) {
    for (bit, glyph) in [(18, '⌃'), (19, '⌥'), (17, '⇧'), (20, '⌘')] {
        if mods & (1 << bit) != 0 {
            chip.push(glyph);
        }
    }
}

/// One keystroke's chip text: a shortcut carries its modifier glyphs (⌃⌥⇧⌘, Apple's order) and an
/// upper-case key, a navigation key its glyph after any modifiers, a modifier tap its glyphs.
/// Redacted (renders an agent sees, D19) every typed character is a dot, so the chip keeps its
/// place, timing and rough width without the text; glyphs are not typed text and stay.
pub fn chip_text(text: &str, code: Option<u64>, mods: u64, tap: bool, redact: bool) -> String {
    let mut chip = String::new();
    if tap {
        modifier_glyphs(mods, &mut chip);
        return chip;
    }
    let shortcut = mods & SHORTCUT_MODIFIERS != 0;
    if let Some(glyph) = navigation_glyph(code, text) {
        modifier_glyphs(mods, &mut chip);
        chip.push_str(glyph);
        return chip;
    }
    if shortcut {
        modifier_glyphs(mods, &mut chip);
    }
    if redact {
        chip.extend(text.chars().map(|_| '•'));
    } else if !shortcut {
        chip.push_str(text);
    } else {
        chip.extend(text.chars().flat_map(char::to_uppercase));
    }
    chip
}

/// Finds modifier taps (a modifier pressed and released with no key between) in an event log,
/// fed in time order.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ModifierTaps {
    held: u64,
    pressed: Option<(f64, u64)>,
}

impl ModifierTaps {
    /// A key went down: the held modifiers are part of a shortcut, not a tap.
    pub fn key(&mut self) {
        self.pressed = None;
    }

    /// A `flags` event at `t` with the new `mods`. Returns the tap (its press time and the
    /// modifiers pressed) when the last modifier is released.
    pub fn flags(&mut self, t: f64, mods: u64) -> Option<(f64, u64)> {
        let mods = mods & MODIFIER_KEYS;
        let pressed = mods & !self.held;
        let tap = if pressed != 0 {
            self.pressed = Some(match self.pressed {
                Some((at, bits)) => (at, bits | pressed),
                None if self.held == 0 => (t, pressed),
                // A modifier added while another stays held after a key: not a tap.
                None => {
                    self.held = mods;
                    return None;
                }
            });
            None
        } else if mods == 0 {
            self.pressed.take()
        } else {
            None
        };
        self.held = mods;
        tap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: u64 = 1 << 20;
    const SHIFT: u64 = 1 << 17;

    #[test]
    fn chips_show_shortcut_glyphs_and_agent_renders_redact_the_text() {
        assert_eq!(chip_text("s", Some(1), COMMAND, false, false), "⌘S");
        assert_eq!(
            chip_text("s", Some(1), COMMAND | SHIFT | (1 << 18), false, false),
            "⌃⇧⌘S"
        );
        assert_eq!(chip_text("s", Some(1), COMMAND, false, true), "⌘•");
        // Shift alone is typing: the text as typed, or dots for an agent.
        assert_eq!(chip_text("P", Some(35), SHIFT, false, false), "P");
        assert_eq!(chip_text("pw", None, 0, false, true), "••");
        // Navigation keys and modifier taps are glyphs, not typed text: agents see them too.
        assert_eq!(chip_text("\u{1c}", Some(123), 0, false, true), "←");
        assert_eq!(chip_text("\t", Some(48), SHIFT, false, true), "⇧⇥");
        assert_eq!(chip_text("\u{1c}", Some(123), COMMAND, false, false), "⌘←");
        assert_eq!(chip_text("", Some(55), COMMAND, true, true), "⌘");
    }

    #[test]
    fn keys_fall_into_recorders_four_categories() {
        assert_eq!(category(Some(1), COMMAND, "s"), SHORTCUTS);
        assert_eq!(category(Some(123), COMMAND, "\u{1c}"), SHORTCUTS);
        assert_eq!(category(Some(0), SHIFT, "A"), TYPING);
        assert_eq!(category(Some(49), 0, " "), TYPING);
        for (code, text) in [
            (36, "\r"),
            (51, "\u{7f}"),
            (53, "\u{1b}"),
            (126, ""),
            (122, ""),
        ] {
            assert_eq!(category(Some(code), 0, text), NAVIGATION, "{code}");
        }
        // Logs without key codes fall back to the control character.
        assert_eq!(category(None, 0, "\r"), NAVIGATION);
        assert_eq!(category(None, 0, "a"), TYPING);
        assert!(visible(Some(49), " ") && visible(Some(123), "\u{1c}"));
        assert!(!visible(Some(57), "") && !visible(None, "\u{1}"));
    }

    #[test]
    fn show_values_parse_in_any_order_and_name_one_canonical_form() {
        assert_eq!(parse_show("all"), Some(ALL));
        assert_eq!(parse_show("none"), Some(0));
        assert_eq!(
            parse_show("navigation+shortcuts"),
            Some(SHORTCUTS | NAVIGATION)
        );
        assert_eq!(parse_show("shortcuts+shortcuts"), None);
        assert_eq!(parse_show("keys"), None);
        assert_eq!(parse_show(""), None);
        for (bits, name) in SHOW.iter().enumerate() {
            assert_eq!(parse_show(name), Some(bits as u8), "{name}");
        }
    }

    #[test]
    fn a_modifier_alone_is_a_tap_and_one_held_for_a_shortcut_is_not() {
        let mut taps = ModifierTaps::default();
        assert_eq!(taps.flags(1.0, COMMAND), None);
        assert_eq!(taps.flags(1.2, 0), Some((1.0, COMMAND)));
        // ⌘S: the modifier belongs to the shortcut.
        assert_eq!(taps.flags(2.0, COMMAND), None);
        taps.key();
        assert_eq!(taps.flags(2.2, 0), None);
        // ⇧ then ⌘ together, released one by one: one tap of both.
        assert_eq!(taps.flags(3.0, SHIFT), None);
        assert_eq!(taps.flags(3.1, SHIFT | COMMAND), None);
        assert_eq!(taps.flags(3.2, COMMAND), None);
        assert_eq!(taps.flags(3.3, 0), Some((3.0, SHIFT | COMMAND)));
        // Caps lock and fn bits are not modifiers here.
        assert_eq!(taps.flags(4.0, 1 << 16), None);
        assert_eq!(taps.flags(4.1, 0), None);
    }
}
