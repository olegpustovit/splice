use crate::Error;

/// Token type. Text and numbers are represented by spans of the caller's original input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `{`
    ObjectStart,
    /// `}`
    ObjectEnd,
    /// `[`
    ArrayStart,
    /// `]`
    ArrayEnd,
    /// `:`
    Colon,
    /// `,`
    Comma,
    /// A quoted JSON string, possibly containing escapes.
    String,
    /// A JSON number, including its sign and exponent when present.
    Number,
    /// `true`
    True,
    /// `false`
    False,
    /// `null`
    Null,
    /// End of input.
    End,
}

/// One token in a borrowed input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// Type of the token.
    pub kind: Kind,
    /// Inclusive start in bytes.
    pub start: usize,
    /// Exclusive end in bytes. Pass this as the next offset.
    pub end: usize,
}

impl Token {
    /// Borrows the original JSON spelling from the same input.
    pub fn text(self, input: &str) -> Option<&str> {
        input.get(self.start..self.end)
    }
}

/// Reads the token at or after a byte offset, skipping JSON whitespace.
/// Successful reads never allocate.
pub fn next(input: &str, offset: usize) -> Result<Token, Error> {
    scan(input, offset, "$")
}

pub(crate) fn scan(input: &str, offset: usize, path: &str) -> Result<Token, Error> {
    if !input.is_char_boundary(offset) {
        return Err(Error::new(offset, path, "invalid token offset"));
    }
    let bytes = input.as_bytes();
    let mut start = offset;
    while matches!(bytes.get(start), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        start += 1;
    }
    let (kind, length) = match bytes.get(start) {
        None => (Kind::End, 0),
        Some(b'{') => (Kind::ObjectStart, 1),
        Some(b'}') => (Kind::ObjectEnd, 1),
        Some(b'[') => (Kind::ArrayStart, 1),
        Some(b']') => (Kind::ArrayEnd, 1),
        Some(b':') => (Kind::Colon, 1),
        Some(b',') => (Kind::Comma, 1),
        Some(b'"') => (Kind::String, string_end(input, start, path, None)? - start),
        Some(b'-' | b'0'..=b'9') => (Kind::Number, number_end(bytes, start, path)? - start),
        Some(b't') if input[start..].starts_with("true") => (Kind::True, 4),
        Some(b'f') if input[start..].starts_with("false") => (Kind::False, 5),
        Some(b'n') if input[start..].starts_with("null") => (Kind::Null, 4),
        _ => return Err(Error::new(start, path, "invalid JSON token")),
    };
    Ok(Token {
        kind,
        start,
        end: start + length,
    })
}

fn number_end(bytes: &[u8], start: usize, path: &str) -> Result<usize, Error> {
    let mut index = start + usize::from(bytes[start] == b'-');
    match bytes.get(index) {
        Some(b'0') => {
            index += 1;
            if bytes.get(index).is_some_and(u8::is_ascii_digit) {
                return Err(Error::new(index, path, "leading zero in number"));
            }
        }
        Some(b'1'..=b'9') => index = consume_digits(bytes, index, path)?,
        _ => return Err(Error::new(index, path, "expected digit")),
    }
    if bytes.get(index) == Some(&b'.') {
        index = consume_digits(bytes, index + 1, path)?;
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        index = consume_digits(bytes, index, path)?;
    }
    Ok(index)
}

fn consume_digits(bytes: &[u8], start: usize, path: &str) -> Result<usize, Error> {
    let mut index = start;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    if index == start {
        return Err(Error::new(index, path, "expected digit"));
    }
    Ok(index)
}

pub(crate) fn string_end(
    input: &str,
    start: usize,
    path: &str,
    mut decoded: Option<&mut String>,
) -> Result<usize, Error> {
    let bytes = input.as_bytes();
    let mut index = start + 1;
    let mut run = index;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'"' => {
                if let Some(out) = decoded {
                    out.push_str(&input[run..index]);
                }
                return Ok(index + 1);
            }
            b'\\' => {
                let (escape, end) = decode_escape(bytes, index, path)?;
                if let Some(out) = decoded.as_deref_mut() {
                    out.push_str(&input[run..index]);
                    out.push(escape);
                }
                run = end;
                index = end;
            }
            0..=31 => return Err(Error::new(index, path, "unescaped control character")),
            _ => index += 1,
        }
    }
    Err(Error::new(index, path, "unterminated string"))
}

fn decode_escape(bytes: &[u8], start: usize, path: &str) -> Result<(char, usize), Error> {
    let escape = match bytes.get(start + 1) {
        Some(b'"') => '"',
        Some(b'\\') => '\\',
        Some(b'/') => '/',
        Some(b'b') => '\u{0008}',
        Some(b'f') => '\u{000c}',
        Some(b'n') => '\n',
        Some(b'r') => '\r',
        Some(b't') => '\t',
        Some(b'u') => return decode_unicode(bytes, start, path),
        _ => return Err(Error::new(start + 1, path, "invalid string escape")),
    };
    Ok((escape, start + 2))
}

fn decode_unicode(bytes: &[u8], start: usize, path: &str) -> Result<(char, usize), Error> {
    let first = hex_quad(bytes, start + 2, path)?;
    let mut end = start + 6;
    let scalar = match first {
        0xd800..=0xdbff => {
            if bytes.get(end..end + 2) != Some(b"\\u") {
                return Err(Error::new(end, path, "expected low surrogate escape"));
            }
            let second = hex_quad(bytes, end + 2, path)?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(Error::new(end, path, "expected low surrogate"));
            }
            end += 6;
            0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        }
        0xdc00..=0xdfff => return Err(Error::new(start, path, "lone low surrogate")),
        _ => u32::from(first),
    };
    let ch =
        char::from_u32(scalar).ok_or_else(|| Error::new(start, path, "invalid unicode scalar"))?;
    Ok((ch, end))
}

fn hex_quad(bytes: &[u8], start: usize, path: &str) -> Result<u16, Error> {
    let mut number = 0;
    for index in start..start + 4 {
        let digit = bytes
            .get(index)
            .and_then(|&byte| char::from(byte).to_digit(16))
            .ok_or_else(|| Error::new(index, path, "expected hex digit"))?;
        number = number * 16 + digit as u16;
    }
    Ok(number)
}
