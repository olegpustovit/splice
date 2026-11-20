//! A counter title's text (T45.4): one formatting rule for the editor's result and the evaluator.

/// The text a counter shows at `value`: rounded to `decimals` (0–4), digits grouped by threes
/// with `,` when `group`, put in place of every `{n}` in `format` (the bare number when the
/// format has none). A non-finite value shows as 0.
pub fn counter_text(value: f64, format: &str, decimals: u32, group: bool) -> String {
    let decimals = decimals.min(4);
    let scale = 10_u64.pow(decimals);
    let value = if value.is_finite() { value } else { 0.0 };
    // Values are bounded by the schema (±1e9), so the scaled integer fits.
    let scaled = (value * scale as f64).round() as i64;
    let magnitude = scaled.unsigned_abs();
    let digits = (magnitude / scale).to_string();
    let mut number = String::with_capacity(digits.len() * 4 / 3 + 6);
    if scaled < 0 {
        number.push('-');
    }
    for (index, digit) in digits.chars().enumerate() {
        if group && index > 0 && (digits.len() - index).is_multiple_of(3) {
            number.push(',');
        }
        number.push(digit);
    }
    if decimals > 0 {
        let fraction = (magnitude % scale).to_string();
        number.push('.');
        (fraction.len()..decimals as usize).for_each(|_| number.push('0'));
        number.push_str(&fraction);
    }
    if format.contains("{n}") {
        format.replace("{n}", &number)
    } else {
        number
    }
}

#[cfg(test)]
mod tests {
    use super::counter_text;

    #[test]
    fn counter_text_rounds_groups_and_formats() {
        assert_eq!(counter_text(0.0, "{n}", 0, true), "0");
        assert_eq!(counter_text(1234567.4, "{n}", 0, true), "1,234,567");
        assert_eq!(counter_text(1234567.0, "{n}", 0, false), "1234567");
        assert_eq!(counter_text(99.5, "{n}%", 0, true), "100%");
        assert_eq!(counter_text(12.05, "$ {n}", 2, true), "$ 12.05");
        assert_eq!(counter_text(3.007, "{n}", 3, true), "3.007");
        assert_eq!(counter_text(-1500.0, "{n} pts", 1, true), "-1,500.0 pts");
        assert_eq!(counter_text(-0.2, "{n}", 0, true), "0");
        assert_eq!(counter_text(f64::NAN, "x{n}", 0, true), "x0");
        assert_eq!(counter_text(42.0, "no slot", 0, true), "42");
    }
}
