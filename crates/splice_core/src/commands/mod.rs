use splice_json::Value as Json;
use splice_model::{EffectParam, EffectValue};
use std::fmt;

mod args;
pub(crate) use args::frame_arg;
mod decode;
mod op;
mod table;

pub use op::*;
pub use table::COMMANDS;

/// Splice's own rows, after the prototype's 61 (docs/reference/core.js), in table order.
pub const EXTENSIONS: &[&str] = &[
    "transcript.set",
    "item.roll",
    "item.slip",
    "item.slide",
    "clip.freeze",
    "clip.compound",
    "clip.uncompound",
    "transition.set",
    "transition.remove",
    "param.set",
    "param.keys",
    "mask.track",
    "generator.add",
    "template.add",
    "template.set",
    "media.relink",
    "media.consolidate",
];

#[cfg(test)]
mod tests;

/// Schedule for a registry command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Included in Splice 0.1.
    First,
    /// Included in Splice 0.2.
    Second,
    /// Included in Splice 0.3.
    Third,
    /// Designed, not built (none after 0.3).
    Planned,
}

impl Tier {
    /// The release that ships the row.
    pub fn cut(self) -> &'static str {
        match self {
            Self::First => "0.1",
            Self::Second => "0.2",
            Self::Third => "0.3",
            Self::Planned => "later",
        }
    }
}

/// The shared command documentation and input decoder.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// Stable wire name.
    pub id: &'static str,
    /// Human-readable name.
    pub title: &'static str,
    /// Command group used for discovery.
    pub group: &'static str,
    /// Edit or destructive action.
    pub risk: &'static str,
    /// Default keyboard shortcut, if defined.
    pub shortcut: &'static str,
    /// Release tier.
    pub tier: Tier,
    /// Parameter names and type descriptions. An initial `!` marks a required field.
    pub params: &'static [(&'static str, &'static str)],
    /// Where the corresponding controls appear in the design.
    pub ui: &'static [&'static str],
    /// Canonical JSON operation using the sample project.
    pub example: &'static str,
    decoder: fn(&Json, &mut DecodeScratch) -> Result<Op, CommandError>,
}

impl Command {
    fn decode(&self, value: &Json, scratch: &mut DecodeScratch) -> Result<Op, CommandError> {
        let fields = value
            .as_object()
            .ok_or_else(|| CommandError::bad("op", "an object"))?;
        for (name, hint) in self.params {
            if let Some(required) = hint.strip_prefix('!')
                && value.field(name).is_none()
            {
                return Err(CommandError::bad(name, required));
            }
        }
        for (name, _) in fields {
            if name != "cmd" && name != "ref" && !self.params.iter().any(|(key, _)| key == name) {
                return Err(CommandError::new(
                    "BAD_ARGS",
                    format!("{} has no argument {name:?}.", self.id),
                    format!(
                        "Call commands.describe with id {:?} for accepted arguments.",
                        self.id
                    ),
                ));
            }
        }
        (self.decoder)(value, scratch)
    }

    fn summary(&self) -> Vec<(String, Json)> {
        let mut fields = vec![
            ("id".into(), text(self.id)),
            ("title".into(), text(self.title)),
            ("risk".into(), text(self.risk)),
            ("cut".into(), text(self.tier.cut())),
        ];
        // Every 0.1 and 0.2 row applies (tier_a and tier_b tests apply each described example).
        fields.push(("available".into(), Json::Bool(self.tier != Tier::Planned)));
        if self.tier == Tier::Planned {
            fields.push(("planned".into(), Json::Bool(true)));
        }
        fields
    }
}

/// Typed edit or discovery failure with a fix hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandError {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Description of the failed input or edit.
    pub message: String,
    /// How to recover.
    pub fix: String,
    /// Suggested registry IDs, if available.
    pub valid: Vec<&'static str>,
}

impl CommandError {
    pub(crate) fn new(code: &'static str, message: String, fix: String) -> Self {
        Self {
            code,
            message,
            fix,
            valid: Vec::new(),
        }
    }

    pub(crate) fn bad(name: &str, expected: &str) -> Self {
        Self::new(
            "BAD_ARGS",
            format!("Invalid or missing {name}."),
            format!("Pass {name} as {expected}."),
        )
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} {}", self.code, self.message, self.fix)
    }
}

impl std::error::Error for CommandError {}

/// One typed operation and the optional name bound to its result.
#[derive(Debug, PartialEq)]
pub struct Decoded {
    /// Decoded arguments.
    pub op: Op,
    /// Batch-local binding, including its leading dollar sign.
    pub bind: Option<String>,
}

/// Parses a wire operation. Commands reserved by the prototype return `PLANNED`.
pub fn decode(value: &Json) -> Result<Decoded, CommandError> {
    decode_reusing(value, &mut DecodeScratch::default())
}

#[derive(Default)]
pub(crate) struct DecodeScratch {
    pub(crate) targets: Vec<Target>,
    pub(crate) name: String,
    pub(crate) color: String,
}

pub(crate) fn decode_reusing(
    value: &Json,
    scratch: &mut DecodeScratch,
) -> Result<Decoded, CommandError> {
    let id = value
        .field("cmd")
        .and_then(Json::as_str)
        .ok_or_else(|| CommandError::bad("cmd", "a command ID"))?;
    let command = lookup(id).ok_or_else(|| unknown(id))?;
    if command.tier == Tier::Planned {
        return Err(CommandError::new(
            "PLANNED",
            format!("{id} is planned for Splice 0.3."),
            "Call commands.describe for commands available in this cut.".into(),
        ));
    }
    let op = command.decode(value, scratch)?;
    let bind = value.field("ref").map(args::binding).transpose()?;
    Ok(Decoded { op, bind })
}

/// Returns the row for an exact wire name.
pub fn lookup(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|row| row.id == id)
}

/// Returns the effect schema catalog used by command discovery.
pub fn effect_schemas() -> &'static Json {
    static SCHEMAS: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
    SCHEMAS.get_or_init(|| {
        splice_json::parse(include_str!("effects.json")).expect("checked effect schemas")
    })
}

/// Returns the documented schema for an ID or a filtered catalog when the ID is absent.
pub fn describe(id: Option<&str>, query: Option<&str>) -> Result<Json, CommandError> {
    if let Some(id) = id {
        let row = lookup(id).ok_or_else(|| unknown(id))?;
        let mut fields = row.summary();
        fields.extend([
            ("group".into(), text(row.group)),
            (
                "shortcut".into(),
                if row.shortcut.is_empty() {
                    Json::Null
                } else {
                    text(row.shortcut)
                },
            ),
            (
                "params".into(),
                Json::Object(
                    row.params
                        .iter()
                        .map(|(key, hint)| ((*key).into(), text(hint)))
                        .collect(),
                ),
            ),
            (
                "ui".into(),
                Json::Array(row.ui.iter().map(|entry| text(entry)).collect()),
            ),
            (
                "example".into(),
                splice_json::parse(row.example).expect("checked registry example"),
            ),
        ]);
        if id.starts_with("caption") {
            fields.push(("captionModel".into(), text("A cue holds transcript word IDs. Edits to a cue change its display and leave its transcript intact.")));
            fields.push((
                "presets".into(),
                Json::Array(
                    args::CAPTION_PRESETS
                        .iter()
                        .map(|preset| text(preset))
                        .collect(),
                ),
            ));
        }
        if id.starts_with("effect.") {
            fields.push(("effectTypes".into(), effect_schemas().clone()));
            fields.push(("resolution".into(), text("Effects inherit each parameter through project, lane, and item. Set enabled:false at a scope to turn off an inherited effect.")));
        }
        return Ok(Json::Object(fields));
    }
    // Any whitespace- or comma-separated term, in the id, title, group or a param's doc.
    let query = query.unwrap_or("").to_lowercase();
    let terms: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|term| !term.is_empty())
        .collect();
    let commands = COMMANDS
        .iter()
        .filter(|row| {
            terms.is_empty()
                || terms.iter().any(|term| {
                    [row.id, row.title, row.group]
                        .into_iter()
                        .chain(row.params.iter().map(|(_, hint)| *hint))
                        .any(|field| field.to_lowercase().contains(term))
                })
        })
        .map(|row| Json::Object(row.summary()))
        .collect();
    Ok(Json::Object(vec![(
        "commands".into(),
        Json::Array(commands),
    )]))
}

fn unknown(id: &str) -> CommandError {
    let prefix = id.split('.').next().unwrap_or("");
    let suffix = id.rsplit('.').next().unwrap_or("");
    let mut valid: Vec<_> = COMMANDS
        .iter()
        .filter(|row| {
            row.id.split('.').next() == Some(prefix)
                || (!suffix.is_empty() && row.id.contains(suffix))
        })
        .map(|row| row.id)
        .collect();
    if valid.is_empty() {
        valid.extend(COMMANDS.iter().take(12).map(|row| row.id));
    }
    CommandError {
        code: "UNKNOWN_COMMAND",
        message: format!("No command {id:?}."),
        fix: "Call commands.describe with a query to find the command ID. Tools such as \
              export.start or view.frames are called directly, not as edit.apply ops."
            .into(),
        valid,
    }
}

fn text(value: &str) -> Json {
    Json::String(value.into())
}

/// The schema entry for one parameter of an effect type.
fn param_spec(kind: &str, name: &str) -> Result<&'static Json, CommandError> {
    let schemas = effect_schemas();
    let schema = schemas
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .ok_or_else(|| {
            CommandError::new(
                "BAD_ARGS",
                format!("No effect type {kind}."),
                "commands.describe {id:\"effect.add\"} lists types.".into(),
            )
        })?;
    schema.field(name).ok_or_else(|| {
        let names = schema
            .as_object()
            .map(|fields| {
                fields
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        CommandError::new(
            "BAD_ARGS",
            format!("{kind} has no parameter {name}."),
            format!("Parameters: {names}."),
        )
    })
}

/// Checks parameter names and values against `effects.json` (decoding, apply and package
/// validation share it).
pub fn check_effect(kind: &str, params: &[EffectParam]) -> Result<(), CommandError> {
    for param in params {
        let spec = param_spec(kind, &param.name)?;
        let number = |key| spec.field(key).and_then(Json::as_f64).unwrap_or(0.0);
        let in_range = |n: f64| n.is_finite() && n >= number("min") && n <= number("max");
        let ok = match (spec.field("type").and_then(Json::as_str), &param.value) {
            (Some("number"), EffectValue::Number(n)) => in_range(*n),
            (Some("number"), EffectValue::Keys(keys)) => {
                !keys.is_empty() && keys.iter().all(|key| in_range(key.value))
            }
            (Some("bool"), EffectValue::Bool(_)) => true,
            (Some("enum"), EffectValue::Text(text)) => spec
                .field("options")
                .and_then(Json::as_array)
                .is_some_and(|options| options.iter().any(|o| o.as_str() == Some(text))),
            (Some("color"), EffectValue::Text(text)) => {
                text.len() == 7
                    && text.starts_with('#')
                    && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
            }
            (Some("file" | "text"), EffectValue::Text(text)) => {
                text.len() <= 4096 && !text.contains('\0')
            }
            _ => false,
        };
        if !ok {
            let expected = match spec.field("type").and_then(Json::as_str) {
                Some("number") => format!(
                    "a number from {} to {}, or {{keys:[{{at, value, ease}}]}} in that range",
                    number("min"),
                    number("max")
                ),
                Some("bool") => "true or false".into(),
                Some("color") => "a #rrggbb color".into(),
                Some("file") => "a file path".into(),
                Some("text") => "text".into(),
                _ => {
                    let mut options = String::new();
                    splice_json::write(spec.field("options").unwrap_or(&Json::Null), &mut options)
                        .expect("finite schema");
                    format!("one of {options}")
                }
            };
            return Err(CommandError::bad(&param.name, &expected));
        }
    }
    Ok(())
}
