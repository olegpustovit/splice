use crate::tokens::{scan, string_end};
use crate::{Error, Kind, MAX_DEPTH, Token};
use std::collections::BTreeSet;
use std::fmt::Write;

/// A parsed JSON value. Object members retain their source order.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// The JSON null value.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integral number with a leading minus sign, other than negative zero.
    Signed(i64),
    /// A nonnegative integer.
    Unsigned(u64),
    /// A floating-point number.
    Float(f64),
    /// Unescaped UTF-8 text.
    String(String),
    /// Array members in order.
    Array(Vec<Value>),
    /// Object members in order, with distinct keys.
    Object(Vec<(String, Value)>),
}

impl Value {
    /// Looks up an object member by its decoded key.
    pub fn field(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(fields) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// Reads a JSON string as UTF-8 text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    /// Reads an integer exactly when it fits in an unsigned 64-bit value.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Self::Unsigned(value) => Some(value),
            Self::Signed(value) => u64::try_from(value).ok(),
            _ => None,
        }
    }

    /// Reads an integer exactly when it fits in a signed 64-bit value.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Signed(value) => Some(value),
            Self::Unsigned(value) => i64::try_from(value).ok(),
            _ => None,
        }
    }

    /// Reads a JSON number as an f64, rounding integer values if needed.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Signed(value) => Some(value as f64),
            Self::Unsigned(value) => Some(value as f64),
            Self::Float(value) => Some(value),
            _ => None,
        }
    }

    /// Reads a JSON boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Self::Bool(value) => Some(value),
            _ => None,
        }
    }

    /// Reads the ordered contents of a JSON array.
    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    /// Reads the ordered fields of a JSON object.
    pub fn as_object(&self) -> Option<&[(String, Self)]> {
        match self {
            Self::Object(fields) => Some(fields),
            _ => None,
        }
    }
}

/// Parses one complete JSON value. Duplicate member names and nonfinite numbers are errors.
pub fn parse(input: &str) -> Result<Value, Error> {
    let mut parser = Parser {
        pos: 0,
        path: "$".into(),
    };
    let root = parser.read(input, 0)?;
    parser.expect(input, Kind::End, "trailing content")?;
    Ok(root)
}

struct Parser {
    pos: usize,
    path: String,
}

impl Parser {
    fn next(&mut self, input: &str) -> Result<Token, Error> {
        let token = scan(input, self.pos, &self.path)?;
        self.pos = token.end;
        Ok(token)
    }

    fn expect(&mut self, input: &str, kind: Kind, message: &'static str) -> Result<Token, Error> {
        let token = self.next(input)?;
        if token.kind != kind {
            return Err(Error::new(token.start, &self.path, message));
        }
        Ok(token)
    }

    fn read(&mut self, input: &str, depth: usize) -> Result<Value, Error> {
        let token = self.next(input)?;
        self.read_token(input, token, depth)
    }

    fn read_token(&mut self, input: &str, token: Token, depth: usize) -> Result<Value, Error> {
        if depth >= MAX_DEPTH && matches!(token.kind, Kind::ArrayStart | Kind::ObjectStart) {
            return Err(Error::new(
                token.start,
                &self.path,
                "JSON nesting limit exceeded",
            ));
        }
        match token.kind {
            Kind::Null => Ok(Value::Null),
            Kind::True => Ok(Value::Bool(true)),
            Kind::False => Ok(Value::Bool(false)),
            Kind::String => decode_string(input, token, &self.path).map(Value::String),
            Kind::Number => read_number(&input[token.start..token.end])
                .ok_or_else(|| Error::new(token.start, &self.path, "number is out of range")),
            Kind::ArrayStart => self.array(input, depth + 1),
            Kind::ObjectStart => self.object(input, depth + 1),
            _ => Err(Error::new(token.start, &self.path, "expected JSON value")),
        }
    }

    fn array(&mut self, input: &str, depth: usize) -> Result<Value, Error> {
        let mut values = Vec::new();
        let parent = self.path.len();
        loop {
            write!(&mut self.path, "[{}]", values.len()).expect("writing to String");
            let token = self.next(input)?;
            if values.is_empty() && token.kind == Kind::ArrayEnd {
                self.path.truncate(parent);
                break;
            }
            values.push(self.read_token(input, token, depth)?);
            self.path.truncate(parent);
            let separator = self.next(input)?;
            match separator.kind {
                Kind::ArrayEnd => break,
                Kind::Comma => {}
                _ => {
                    return Err(Error::new(
                        separator.start,
                        &self.path,
                        "expected comma or array end",
                    ));
                }
            }
        }
        Ok(Value::Array(values))
    }

    fn object(&mut self, input: &str, depth: usize) -> Result<Value, Error> {
        let mut fields = Vec::new();
        let mut names = BTreeSet::new();
        let parent = self.path.len();
        loop {
            let token = self.next(input)?;
            if fields.is_empty() && token.kind == Kind::ObjectEnd {
                break;
            }
            if token.kind != Kind::String {
                return Err(Error::new(token.start, &self.path, "expected object key"));
            }
            let key = decode_string(input, token, &self.path)?;
            append_key(&mut self.path, &key);
            if !names.insert(key.clone()) {
                return Err(Error::new(token.start, &self.path, "duplicate object key"));
            }
            self.expect(input, Kind::Colon, "expected colon after object key")?;
            let value = self.read(input, depth)?;
            fields.push((key, value));
            self.path.truncate(parent);
            let separator = self.next(input)?;
            match separator.kind {
                Kind::ObjectEnd => break,
                Kind::Comma => {}
                _ => {
                    return Err(Error::new(
                        separator.start,
                        &self.path,
                        "expected comma or object end",
                    ));
                }
            }
        }
        Ok(Value::Object(fields))
    }
}

fn append_key(path: &mut String, key: &str) {
    let mut bytes = key.bytes();
    if bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        path.push('.');
        path.push_str(key);
    } else {
        path.push('[');
        crate::write::string(key, path);
        path.push(']');
    }
}

fn decode_string(input: &str, token: Token, path: &str) -> Result<String, Error> {
    let inner = &input[token.start + 1..token.end - 1];
    if !inner.contains('\\') {
        return Ok(inner.to_owned());
    }
    let mut out = String::with_capacity(inner.len());
    string_end(input, token.start, path, Some(&mut out))?;
    Ok(out)
}

fn read_number(raw: &str) -> Option<Value> {
    if raw == "-0" {
        return Some(Value::Float(-0.0));
    }
    if !raw.contains(['.', 'e', 'E']) {
        if raw.starts_with('-') {
            if let Ok(value) = raw.parse::<i64>() {
                return Some(Value::Signed(value));
            }
        } else if let Ok(value) = raw.parse::<u64>() {
            return Some(Value::Unsigned(value));
        }
    }
    let value = raw.parse::<f64>().ok()?;
    value.is_finite().then_some(Value::Float(value))
}
