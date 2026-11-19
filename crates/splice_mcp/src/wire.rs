use super::{Json, object, text};
use splice_core::history::{Source, StoreError, WriteOptions};
use splice_model::{Frame, SessionId};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn failure(code: &'static str, message: &str, fix: &str) -> StoreError {
    StoreError::new(code, message, fix)
}

pub(super) fn bad(field: &str, expected: &str) -> StoreError {
    failure(
        "BAD_ARGS",
        &format!("Invalid {field}."),
        &format!("Pass {field} as {expected}."),
    )
}

/// A required field is absent (as opposed to present with the wrong shape).
pub(super) fn missing(field: &str, expected: &str) -> StoreError {
    failure(
        "BAD_ARGS",
        &format!("Missing {field}."),
        &format!("Pass {field} as {expected}."),
    )
}

pub(super) fn count(value: usize) -> Json {
    Json::Unsigned(value as u64)
}

pub(super) fn frame(value: Frame) -> Json {
    Json::Signed(i64::from(value))
}

pub(super) fn display(value: &dyn fmt::Display) -> Json {
    Json::String(value.to_string())
}

pub(super) fn rev(value: u64) -> Json {
    Json::String(format!("r{value}"))
}

pub(super) fn hash(value: u64) -> Json {
    Json::String(format!("{value:016x}"))
}

pub(super) fn range(from: Frame, to: Frame) -> Json {
    object(vec![("from", frame(from)), ("to", frame(to))])
}

pub(super) fn source(value: Source) -> Json {
    text(match value {
        Source::Agent => "agent",
        Source::Human => "human",
    })
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or(0)
}

pub(super) trait Args {
    fn optional_string(&self, name: &str) -> Result<Option<&str>, StoreError>;
    fn string(&self, name: &str) -> Result<&str, StoreError>;
    fn flag(&self, name: &str) -> Result<bool, StoreError>;
    fn frame(&self, name: &str) -> Result<Option<Frame>, StoreError>;
    fn revision(&self, name: &str) -> Result<u64, StoreError>;
    fn limit(&self, default: usize) -> Result<usize, StoreError>;
    fn cursor(&self, prefix: char, total: usize) -> Result<usize, StoreError>;
    fn session(&self) -> Result<SessionId, StoreError>;
}

impl Args for Json {
    fn optional_string(&self, name: &str) -> Result<Option<&str>, StoreError> {
        self.field(name)
            .map(|value| value.as_str().ok_or_else(|| bad(name, "a string")))
            .transpose()
    }

    fn string(&self, name: &str) -> Result<&str, StoreError> {
        self.optional_string(name)?
            .ok_or_else(|| missing(name, "a string"))
    }

    fn flag(&self, name: &str) -> Result<bool, StoreError> {
        self.field(name).map_or(Ok(false), |value| {
            value.as_bool().ok_or_else(|| bad(name, "a boolean"))
        })
    }

    fn frame(&self, name: &str) -> Result<Option<Frame>, StoreError> {
        self.field(name)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| Frame::try_from(value).ok())
                    .ok_or_else(|| bad(name, "a nonnegative frame"))
            })
            .transpose()
    }

    fn revision(&self, name: &str) -> Result<u64, StoreError> {
        let expected =
            "the current revision (project.info, or the revision the last edit returned)";
        let value = self.field(name).ok_or_else(|| missing(name, expected))?;
        Some(value)
            .and_then(|value| {
                value
                    .as_u64()
                    .or_else(|| unsigned(value.as_str()?.strip_prefix('r')?))
            })
            .ok_or_else(|| bad(name, expected))
    }

    fn limit(&self, default: usize) -> Result<usize, StoreError> {
        self.field("limit").map_or(Ok(default), |value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|&n| n != 0)
                .ok_or_else(|| bad("limit", "a positive integer"))
        })
    }

    fn cursor(&self, prefix: char, total: usize) -> Result<usize, StoreError> {
        self.optional_string("cursor")?.map_or(Ok(0), |cursor| {
            cursor
                .strip_prefix(prefix)
                .and_then(unsigned)
                .and_then(|number| usize::try_from(number).ok())
                .filter(|&index| index <= total)
                .ok_or_else(|| bad("cursor", "the nextCursor from the previous page"))
        })
    }

    fn session(&self) -> Result<SessionId, StoreError> {
        self.optional_string("sessionId")?
            .ok_or_else(|| {
                failure(
                    "NO_SESSION",
                    "This tool needs a sessionId.",
                    "Call session.start and pass its sessionId.",
                )
            })?
            .parse()
            .map_err(|_| bad("sessionId", "a session ID from session.start"))
    }
}

fn unsigned(digits: &str) -> Option<u64> {
    digits
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| digits.parse().ok())
        .flatten()
}

pub(super) fn write_options(args: &Json) -> Result<WriteOptions, StoreError> {
    Ok(WriteOptions {
        source: Source::Agent,
        session: Some(args.session()?),
        if_revision: Some(args.revision("ifRevision")?),
        dry_run: args.flag("dryRun")?,
        key: args.optional_string("idempotencyKey")?.map(str::to_owned),
        label: args
            .optional_string("summary")?
            .filter(|label| !label.is_empty())
            .map(|summary| format!("Agent: {summary}")),
        time: now(),
    })
}

pub(super) fn next_cursor(prefix: char, end: usize, total: usize) -> Json {
    if end < total {
        Json::String(format!("{prefix}{end}"))
    } else {
        Json::Null
    }
}

pub(super) fn extend(value: Json, extra: Vec<(&str, Json)>) -> Json {
    let Json::Object(mut fields) = value else {
        unreachable!("extend an object")
    };
    fields.extend(extra.into_iter().map(|(name, value)| (name.into(), value)));
    Json::Object(fields)
}

/// Serializes the code, recovery hint, and optional operation or stale-revision context.
pub fn error_payload(error: StoreError) -> Json {
    let mut fields = vec![
        ("code", text(error.code)),
        ("message", Json::String(error.message)),
        ("fix", Json::String(error.fix)),
    ];
    if error.code == "TCC_DENIED" {
        fields.push(("needsHuman", Json::Bool(true)));
    }
    if let Some(op) = error.op {
        fields.push(("op", count(op)));
    }
    if !error.valid.is_empty() {
        fields.push((
            "valid",
            Json::Array(error.valid.iter().map(|s| text(s)).collect()),
        ));
    }
    if let Some(rule) = error.rule {
        fields.push(("param", Json::String(rule.param)));
        if let Some(got) = rule.got {
            fields.push(("got", got));
        }
        fields.push(("expected", Json::String(rule.expected)));
        if let Some(name) = rule.did_you_mean {
            fields.push(("didYouMean", Json::String(name)));
        }
    }
    if let Some(stale) = error.stale {
        fields.push(("current", rev(stale.current)));
        fields.push((
            "changedSince",
            Json::Array(
                stale
                    .changed_since
                    .into_iter()
                    .map(|change| {
                        object(vec![
                            ("revision", rev(change.revision)),
                            ("source", source(change.source)),
                            ("summary", Json::String(change.label)),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    object(fields)
}
