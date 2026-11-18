use crate::Value;
use std::fmt::{self, Write};

/// JSON cannot represent infinity or NaN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NonFiniteNumber;

impl fmt::Display for NonFiniteNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON number must be finite")
    }
}

impl std::error::Error for NonFiniteNumber {}

/// Replaces the contents of a reusable buffer with compact JSON.
/// Object keys are emitted in their schema's order as supplied in `value`.
/// The buffer is empty if an input number cannot be represented.
pub fn write(value: &Value, out: &mut String) -> Result<(), NonFiniteNumber> {
    out.clear();
    let result = append(value, out);
    if result.is_err() {
        out.clear();
    }
    result
}

fn append(value: &Value, out: &mut String) -> Result<(), NonFiniteNumber> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(switch) => out.push_str(if *switch { "true" } else { "false" }),
        Value::Signed(n) => write!(out, "{n}").expect("writing to String"),
        Value::Unsigned(n) => write!(out, "{n}").expect("writing to String"),
        Value::Float(n) => float(*n, out)?,
        Value::String(s) => string(s, out),
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                append(value, out)?;
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (index, (key, value)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                string(key, out);
                out.push(':');
                append(value, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

pub(crate) fn string(text: &str, out: &mut String) {
    out.push('"');
    let mut run = 0;
    for (i, ch) in text.char_indices() {
        let escaped = match ch {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\u{0008}' => "\\b",
            '\u{000c}' => "\\f",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\u{0000}'..='\u{001f}' => {
                out.push_str(&text[run..i]);
                write!(out, "\\u{:04x}", ch as u32).expect("writing to String");
                run = i + 1;
                continue;
            }
            _ => continue,
        };
        out.push_str(&text[run..i]);
        out.push_str(escaped);
        run = i + 1;
    }
    out.push_str(&text[run..]);
    out.push('"');
}

fn float(value: f64, out: &mut String) -> Result<(), NonFiniteNumber> {
    if !value.is_finite() {
        return Err(NonFiniteNumber);
    }

    // LowerExp chooses the shortest round-tripping significand. Move its
    // decimal point to find the shortest JSON spelling of those digits.
    let mut scratch = FloatText::default();
    write!(&mut scratch, "{:e}", value.abs()).expect("f64 exponent fits");
    let scientific = str::from_utf8(&scratch.bytes[..scratch.length]).expect("f64 writes ASCII");
    let (mantissa, exponent) = scientific.split_once('e').expect("LowerExp emits exponent");
    let exponent: i32 = exponent
        .parse()
        .expect("LowerExp writes an integer exponent");
    let mut digits = FloatText::default();
    for part in mantissa.split('.') {
        digits.write_str(part).expect("f64 significand fits");
    }
    let digits = str::from_utf8(&digits.bytes[..digits.length]).expect("f64 writes ASCII");
    let count = i32::try_from(digits.len()).expect("at most 17 f64 digits");
    let decimal_point = exponent + 1;
    let mut length = if decimal_point <= 0 {
        2 - decimal_point + count
    } else if decimal_point < count {
        count + 1
    } else {
        decimal_point
    };
    let mut point = decimal_point;
    for candidate in 1..=count {
        let power = decimal_point - candidate;
        if power == 0 {
            continue;
        }
        let magnitude = power.unsigned_abs();
        let exponent_length =
            2 + i32::from(power < 0) + i32::from(magnitude >= 10) + i32::from(magnitude >= 100);
        let candidate_length = count + i32::from(candidate != count) + exponent_length;
        if candidate_length < length {
            length = candidate_length;
            point = candidate;
        }
    }

    if value.is_sign_negative() {
        out.push('-');
    }
    if point <= 0 {
        out.push_str("0.");
        for _ in point..0 {
            out.push('0');
        }
        out.push_str(digits);
    } else if point < count {
        let (left, right) = digits.split_at(point as usize);
        out.push_str(left);
        out.push('.');
        out.push_str(right);
    } else {
        out.push_str(digits);
        for _ in count..point {
            out.push('0');
        }
    }
    if point != decimal_point {
        write!(out, "e{}", decimal_point - point).expect("writing to String");
    }
    Ok(())
}

#[derive(Default)]
struct FloatText {
    bytes: [u8; 32],
    length: usize,
}

impl Write for FloatText {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.length + text.len();
        self.bytes
            .get_mut(self.length..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(text.as_bytes());
        self.length = end;
        Ok(())
    }
}
