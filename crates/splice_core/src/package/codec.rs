use super::PackageError;
use splice_json::Value as Json;
use splice_model::*;
use std::fmt::Display;

pub(crate) const FORMAT: &str = "splice/1";

pub(crate) fn shape(value: &Json, known: &[&str]) -> Result<(), PackageError> {
    let fields = value
        .as_object()
        .ok_or_else(|| PackageError::invalid(&"expected object"))?;
    for (name, _) in fields {
        if !known.contains(&name.as_str()) {
            return Err(PackageError::new(
                "FORMAT_UNKNOWN_KEY",
                format!("unknown field {name:?}"),
                "Use fields from docs/FORMAT.md for this project version.",
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_version(value: &Json) -> Result<(), PackageError> {
    let text = value.string("format")?;
    if text == FORMAT {
        return Ok(());
    }
    if let Some(major) = text.strip_prefix("splice/")
        && !major.is_empty()
        && major.bytes().all(|b| b.is_ascii_digit())
        && major.trim_start_matches('0') != "1"
        && !major.trim_start_matches('0').is_empty()
    {
        return Err(PackageError::new(
            "FORMAT_TOO_NEW",
            format!("this build cannot open {text}"),
            "Open the project with a newer version of Splice.",
        ));
    }
    Err(PackageError::invalid(&"expected format splice/1"))
}

pub(crate) fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

pub(crate) fn text(value: &str) -> Json {
    Json::String(value.into())
}

pub(crate) fn display(value: &dyn Display) -> Json {
    Json::String(value.to_string())
}

pub(crate) trait ReadFields {
    fn required(&self, name: &'static str) -> Result<&Json, PackageError>;
    fn optional(&self, name: &'static str) -> Option<&Json>;
    fn array(&self, name: &'static str) -> Result<&[Json], PackageError>;
    fn array_or_empty(&self, name: &'static str) -> Result<&[Json], PackageError>;
    fn string(&self, name: &'static str) -> Result<&str, PackageError>;
}

impl ReadFields for Json {
    fn required(&self, name: &'static str) -> Result<&Json, PackageError> {
        self.field(name)
            .ok_or_else(|| PackageError::invalid(&format!("missing {name}")))
    }

    fn optional(&self, name: &'static str) -> Option<&Json> {
        self.field(name).filter(|field| **field != Json::Null)
    }

    fn array(&self, name: &'static str) -> Result<&[Json], PackageError> {
        self.required(name)?
            .as_array()
            .ok_or_else(|| bad(name, "array"))
    }

    fn array_or_empty(&self, name: &'static str) -> Result<&[Json], PackageError> {
        self.field(name).map_or(Ok(&[]), |field| {
            field.as_array().ok_or_else(|| bad(name, "array"))
        })
    }

    fn string(&self, name: &'static str) -> Result<&str, PackageError> {
        self.required(name)?
            .as_str()
            .ok_or_else(|| bad(name, "string"))
    }
}

pub(crate) fn optional_string(
    value: &Json,
    name: &'static str,
) -> Result<Option<String>, PackageError> {
    value
        .optional(name)
        .map(|field| {
            field
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| bad(name, "string"))
        })
        .transpose()
}

pub(crate) fn optional_bool(
    value: &Json,
    name: &'static str,
) -> Result<Option<bool>, PackageError> {
    value
        .optional(name)
        .map(|v| v.as_bool().ok_or_else(|| bad(name, "boolean")))
        .transpose()
}

pub(crate) fn boolean(value: &Json, name: &'static str) -> Result<bool, PackageError> {
    value
        .required(name)?
        .as_bool()
        .ok_or_else(|| bad(name, "boolean"))
}

pub(crate) fn hash(value: &Json, name: &'static str) -> Result<u64, PackageError> {
    let text = value.string(name)?;
    if text.len() != 16 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad(name, "16-digit hexadecimal hash"));
    }
    u64::from_str_radix(text, 16).map_err(|_| bad(name, "16-digit hexadecimal hash"))
}

pub(crate) fn hash_text(value: u64) -> Json {
    Json::String(format!("{value:016x}"))
}

pub(crate) fn u64_field(value: &Json, name: &'static str) -> Result<u64, PackageError> {
    value
        .required(name)?
        .as_u64()
        .ok_or_else(|| bad(name, "unsigned integer"))
}

pub(crate) fn u32_field(value: &Json, name: &'static str) -> Result<u32, PackageError> {
    u32::try_from(u64_field(value, name)?).map_err(|_| bad(name, "unsigned 32-bit integer"))
}

pub(crate) fn frame(value: &Json, name: &'static str) -> Result<Frame, PackageError> {
    value
        .required(name)?
        .as_i64()
        .and_then(|n| Frame::try_from(n).ok())
        .ok_or_else(|| bad(name, "integer frame"))
}

pub(crate) fn frame_json(value: Frame) -> Json {
    Json::Signed(i64::from(value))
}

pub(crate) fn number(value: &Json, name: &'static str) -> Result<f64, PackageError> {
    value
        .required(name)?
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| bad(name, "finite number"))
}

pub(crate) fn bad(name: &str, expected: &str) -> PackageError {
    PackageError::invalid(&format!("{name} must be {expected}"))
}

pub(crate) fn bad_id(name: &str) -> PackageError {
    bad(name, "an ID with the expected prefix and serial")
}

pub(crate) fn push_text(
    fields: &mut Vec<(&'static str, Json)>,
    name: &'static str,
    value: &Option<String>,
) {
    if let Some(value) = value {
        fields.push((name, text(value)));
    }
}

pub(crate) fn push_bool(
    fields: &mut Vec<(&'static str, Json)>,
    name: &'static str,
    value: Option<bool>,
) {
    if let Some(value) = value {
        fields.push((name, Json::Bool(value)));
    }
}
