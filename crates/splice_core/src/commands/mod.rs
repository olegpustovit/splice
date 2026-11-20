use splice_json::Value as Json;
use splice_model::{EffectParam, EffectValue};
use std::fmt;

mod args;
pub(crate) use args::{frame_arg, reference};
mod decode;
mod op;
mod table;
mod text;

pub use op::*;
pub use table::COMMANDS;

/// Command IDs that 0.4 replaced: (old, replacement, why). Calling an old ID names its successor.
pub const RETIRED: &[(&str, &str, &str)] = &[
    (
        "motion.add",
        "zoom.add",
        "scale animations snapped back to 1x when they ended; zooms are camera segments now",
    ),
    (
        "motion.set",
        "zoom.set",
        "scale animations are camera segments now (zoom.set edits one)",
    ),
];

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
    "zoom.remove",
    "take.pointer",
    "intent.steps",
    "intent.speedTyping",
    "caption.add",
    "item.order",
    "motion.preset",
    "html.add",
    "music.add",
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
        // Unknown names first: a typo of a required argument reads as the typo.
        for (name, _) in fields {
            if name != "cmd" && name != "ref" && !self.params.iter().any(|(key, _)| key == name) {
                return Err(CommandError::unknown_name(
                    "UNKNOWN_PARAM",
                    format!("{} has no argument {name:?}.", self.id),
                    format!(
                        "Call commands.describe with id {:?} for accepted arguments.",
                        self.id
                    ),
                    name,
                    &mut self.params.iter().map(|(key, _)| *key),
                ));
            }
        }
        for (name, hint) in self.params {
            if let Some(required) = hint.strip_prefix('!')
                && value.field(name).is_none()
            {
                return Err(CommandError::bad(name, required));
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

/// The rule a refused argument broke (D18): the parameter, what was passed and what is
/// accepted. `got` is filled from the op by `apply` when the refusing code did not have it.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    /// Argument or effect parameter name.
    pub param: String,
    /// The value passed, if any.
    pub got: Option<Json>,
    /// What is accepted.
    pub expected: String,
    /// The closest known name, for an unknown one.
    pub did_you_mean: Option<String>,
}

impl Rule {
    /// Fills `got` from the arguments the refused value came from (`param` may be a dotted
    /// path such as `box.x`; its first segment names the argument).
    pub fn fill_got(&mut self, args: &Json) {
        if self.got.is_none() {
            let key = self.param.split(['.', '[']).next().unwrap_or("");
            self.got = args.field(key).cloned();
        }
    }
}

/// Typed edit or discovery failure with a fix hint.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandError {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Description of the failed input or edit.
    pub message: String,
    /// How to recover.
    pub fix: String,
    /// Suggested registry IDs, if available.
    pub valid: Vec<&'static str>,
    /// The broken rule, for argument errors and unknown names.
    pub rule: Option<Box<Rule>>,
}

impl CommandError {
    /// A failure with a code, description and recovery hint.
    pub fn new(code: &'static str, message: String, fix: String) -> Self {
        Self {
            code,
            message,
            fix,
            valid: Vec::new(),
            rule: None,
        }
    }

    /// An invalid or missing argument: `name` must be `expected`.
    pub fn bad(name: &str, expected: &str) -> Self {
        Self::rule(
            "BAD_ARGS",
            name,
            None,
            expected,
            &format!("Pass {name} as {expected}."),
        )
    }

    /// A refusal that names the broken rule.
    pub fn rule(
        code: &'static str,
        param: &str,
        got: Option<Json>,
        expected: &str,
        fix: &str,
    ) -> Self {
        let mut error = Self::new(
            code,
            match &got {
                Some(_) => format!("{param} breaks a rule: expected {expected}."),
                None => format!("Invalid or missing {param}: expected {expected}."),
            },
            fix.into(),
        );
        error.rule = Some(Box::new(Rule {
            param: param.into(),
            got,
            expected: expected.into(),
            did_you_mean: None,
        }));
        error
    }

    /// An unknown name among `known`, with the closest one when it is a likely typo.
    pub fn unknown_name(
        code: &'static str,
        message: String,
        fix: String,
        name: &str,
        known: &mut dyn Iterator<Item = &str>,
    ) -> Self {
        let did_you_mean = closest(name, known);
        // Agents read the text first: the suggestion goes there too.
        let message = match &did_you_mean {
            Some(near) => format!("{message} Did you mean {near:?}?"),
            None => message,
        };
        let mut error = Self::new(code, message, fix);
        error.rule = Some(Box::new(Rule {
            param: name.into(),
            got: None,
            expected: "a known name".into(),
            did_you_mean,
        }));
        error
    }
}

/// The candidate within a small edit distance of `name` (case-insensitive), if any.
pub(crate) fn closest(name: &str, known: &mut dyn Iterator<Item = &str>) -> Option<String> {
    let name = name.to_ascii_lowercase();
    let limit = (name.len() / 3).max(2);
    let mut synonym = None;
    let best = known
        .inspect(|candidate| {
            // Names agents carry over from other calls (items.inspect takes ids).
            if SYNONYMS.contains(&(name.as_str(), *candidate)) {
                synonym = Some(candidate.to_string());
            }
        })
        .map(|candidate| (distance(&name, &candidate.to_ascii_lowercase()), candidate))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, candidate)| candidate.to_string());
    best.or(synonym)
}

/// Argument names that mean the same thing elsewhere in the API, for didYouMean.
const SYNONYMS: &[(&str, &str)] = &[("ids", "items"), ("id", "item"), ("items", "ids")];

/// Levenshtein distance over bytes (names are ASCII).
fn distance(a: &str, b: &str) -> usize {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
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
    /// Effect parameters (names reused) and a `type.param` path, handed back by the set handlers.
    pub(crate) params: Vec<splice_model::EffectParam>,
    pub(crate) path: String,
}

/// The batch binding an op's `ref` declares, if any.
pub(crate) fn binding_of(value: &Json) -> Result<Option<String>, CommandError> {
    value.field("ref").map(args::binding).transpose()
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

/// Returns the effect schema catalog used by command discovery: current types only.
pub fn effect_schemas() -> &'static Json {
    static SCHEMAS: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
    SCHEMAS.get_or_init(|| match all_effect_schemas() {
        Json::Object(types) => Json::Object(
            types
                .iter()
                .filter(|(_, schema)| schema.field("legacy").is_none())
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    })
}

/// Every effect type, including `legacy` ones that 0.3 journals and files still hold (inert:
/// validated and replayed, never offered or rendered).
pub(crate) fn all_effect_schemas() -> &'static Json {
    static SCHEMAS: std::sync::OnceLock<Json> = std::sync::OnceLock::new();
    SCHEMAS.get_or_init(|| {
        splice_json::parse(include_str!("effects.json")).expect("checked effect schemas")
    })
}

/// Whether an effect type is retired (kept only so old journals and files load).
pub(crate) fn is_legacy_effect(kind: &str) -> bool {
    all_effect_schemas()
        .field(kind)
        .is_some_and(|schema| schema.field("legacy").is_some())
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
                Json::Array(args::caption_presets().to_vec()),
            ));
        }
        if id == "param.keys" {
            fields.push(("eases".into(), eases()));
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
    if let Some((_, replacement, why)) = RETIRED.iter().find(|(old, ..)| *old == id) {
        return CommandError {
            code: "UNKNOWN_COMMAND",
            message: format!("{id} was retired in 0.4: {why}"),
            fix: format!(
                "Use {replacement}; commands.describe {{id:\"{replacement}\"}} has its arguments."
            ),
            valid: vec![*replacement],
            rule: Some(Box::new(Rule {
                param: "cmd".into(),
                got: Some(text(id)),
                expected: "a current command".into(),
                did_you_mean: Some((*replacement).into()),
            })),
        };
    }
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
        rule: Some(Box::new(Rule {
            param: "cmd".into(),
            got: Some(text(id)),
            expected: "a command ID".into(),
            did_you_mean: closest(id, &mut COMMANDS.iter().map(|row| row.id)),
        })),
    }
}

fn text(value: &str) -> Json {
    Json::String(value.into())
}

/// The ease vocabulary for `commands.describe {id:"param.keys"}`, with measured spring settle
/// times (T42.1, D20).
fn eases() -> Json {
    let settle = [-0.5, 0.0, 0.25, 0.5]
        .into_iter()
        .map(|bounce| {
            let ease = splice_model::Ease::Spring {
                duration: 30.0,
                bounce,
            };
            Json::Object(vec![
                ("duration".into(), Json::Unsigned(30)),
                ("bounce".into(), Json::Float(bounce)),
                (
                    "settle".into(),
                    Json::Unsigned(ease.settle().unwrap_or(0.0).ceil() as u64),
                ),
            ])
        })
        .collect();
    Json::Object(vec![
        ("accepted".into(), text(args::EASES)),
        ("default".into(), text("inOut")),
        (
            "spring".into(),
            text(
                "{spring:{duration, bounce}}: a closed-form spring from this key toward the next \
                 (any frame seeks exactly). duration is the perceptual duration in frames; bounce \
                 0 is critically damped, above 0 overshoots, below 0 is overdamped. It settles \
                 (stays within 0.1%) after `settle` frames, which scale with duration; place the \
                 next key at least that far away or the value jumps there. Results report settle.",
            ),
        ),
        ("springSettle".into(), Json::Array(settle)),
        (
            "named".into(),
            text(
                "back overshoots ~10% then settles; elastic rings onto the value; bounce lands \
                 like a dropped ball; {steps:n} jumps in n equal steps.",
            ),
        ),
    ])
}

/// The schema entry for one parameter of an effect type.
fn param_spec(kind: &str, name: &str) -> Result<&'static Json, CommandError> {
    let schemas = all_effect_schemas();
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
        CommandError::unknown_name(
            "UNKNOWN_PARAM",
            format!("{kind} has no parameter {name}."),
            format!("Parameters: {names}."),
            name,
            &mut schema
                .as_object()
                .into_iter()
                .flatten()
                .map(|(key, _)| key.as_str()),
        )
    })
}

/// Whether a parameter schema entry accepts keys (`"keys": true`).
pub fn keyable(spec: &Json) -> bool {
    spec.field("keys").and_then(Json::as_bool) == Some(true)
}

/// The keyable parameters of an effect type, comma-separated (or "none").
fn keyable_names(kind: &str) -> String {
    let mut names = String::new();
    for (name, spec) in effect_schemas()
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        if keyable(spec) {
            if !names.is_empty() {
                names.push_str(", ");
            }
            names.push_str(name);
        }
    }
    if names.is_empty() {
        names.push_str("none");
    }
    names
}

/// Checks parameter names and values against `effects.json` (decoding, apply and package
/// validation share it).
pub fn check_effect(kind: &str, params: &[EffectParam]) -> Result<(), CommandError> {
    for param in params {
        let spec = param_spec(kind, &param.name)?;
        // Keys only where the evaluator samples per frame; anything else would render its
        // default silently (M20, T36.4).
        if matches!(param.value, EffectValue::Keys(_)) && !keyable(spec) {
            return Err(CommandError::rule(
                "NOT_KEYABLE",
                &param.name,
                None,
                "a single value (this parameter is not animated)",
                &format!(
                    "Pass {kind}.{} as a constant. Keyable {kind} parameters: {}.",
                    param.name,
                    keyable_names(kind)
                ),
            ));
        }
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
