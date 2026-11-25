//! TypeScript declarations for code mode, generated from `COMMANDS` and the tool schemas.
use super::{Json, tools};
use splice_core::commands::{COMMANDS, Command, Tier, effect_schemas, keyable};
use std::fmt::Write;

const HEADER: &str = r#"// Splice code mode: the global `splice` inside code.run. Generated from the command registry
// and the MCP tool schemas. Tool calls are synchronous. Omit optional fields; never pass null
// to mean "unset" (null is only a value where a type says so).

/** A tool's business error, thrown by every `splice` call. `fix` says what to change. */
declare class SpliceError extends Error {
  code: string;
  fix: string;
  valid?: unknown[];
  current?: unknown;
}

declare namespace Splice {
  /** A frame number, a "$ref" bound earlier in the batch, or a position relative to an item, marker or word. */
  type At =
    | number
    | string
    | { item: string; edge?: "start" | "end"; offset?: number }
    | { ref: string; edge?: "start" | "end"; offset?: number }
    | { marker: string; offset?: number }
    | { word: string; transcriptHash?: string; edge?: "start" | "end"; offset?: number };
  /** "r12" or 12. */
  type Revision = number | string;
"#;

/// Types only command interfaces use (the index leaves them out).
const COMMAND_TYPES: &str = r#"  /** A rectangle normalized to the source frame (0–1). */
  type Rect = { x: number; y: number; w: number; h: number };
  /** Curve toward the next key (agent keys default to "inOut"). A spring's duration is in frames;
   * bounce 0 is critically damped, above 0 overshoots, below 0 is overdamped. */
  type Ease =
    | "linear" | "hold" | "in" | "out" | "inOut" | "back" | "elastic" | "bounce"
    | [number, number, number, number]
    | { steps: number }
    | { spring: { duration: number; bounce?: number } };
  /** A keyframe; frames are relative to the owner's start. */
  type Key = { at: number; value: number; ease?: Ease };
"#;

const API: &str = r#"
declare const splice: {
  /** The projectId and sessionId passed to code.run, else the first project this code opens and session it
   * starts. Tool calls default to them. */
  readonly projectId: string | undefined;
  readonly sessionId: string | undefined;
  /** Calls a tool by name and returns its structured result. */
  call(name: string, args?: object): any;
  /** These declarations, filtered by a substring of a command or tool name. */
  describe(query?: string): string;
"#;

/// Without a filter: the index (shared types, every tool signature, one line per command), so a
/// session does not pay for the whole surface up front. `"*"`: everything. Otherwise the commands
/// and tools whose name contains any comma-separated term, with full types.
pub(super) fn declarations(filter: Option<&str>) -> String {
    let filter = filter.unwrap_or("").trim().to_lowercase();
    if filter.is_empty() {
        return index();
    }
    let terms: Vec<&str> = filter
        .split(',')
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .collect();
    let all = terms.contains(&"*");
    let wanted = |name: &str| all || terms.iter().any(|term| name.to_lowercase().contains(term));
    let filter = if all { "" } else { filter.as_str() };
    let mut out = String::from(HEADER);
    out.push_str(COMMAND_TYPES);
    // Planned rows only fail with NOT_IMPLEMENTED; commands.describe still lists them.
    let commands: Vec<&Command> = COMMANDS
        .iter()
        .filter(|row| row.tier != Tier::Planned && wanted(row.id))
        .collect();
    let ids: Vec<&str> = commands.iter().map(|row| row.id).collect();
    if !commands.is_empty() {
        out.push_str("\n  namespace op {\n");
        out.push_str("    interface Base {\n      /** Binds the result for later ops as \"$name\" (\"$name.new\": its first created item). Pass \"$music\" (a bare name binds the same). */\n      ref?: `$${string}`;\n    }\n");
        for row in &commands {
            command(&mut out, row);
        }
        out.push_str("  }\n");
        if ids
            .iter()
            .any(|id| id.starts_with("effect.") || id.starts_with("param."))
        {
            effects(&mut out);
        }
        if filter.is_empty() {
            out.push_str("  /** Any registry command, for edit.apply's ops. */\n  type Op =\n");
            for id in &ids {
                let _ = writeln!(out, "    | op.{}", pascal(id));
            }
            out.truncate(out.len() - 1);
            out.push_str(";\n");
        }
    }
    out.push_str("}\n");
    out.push_str(API);
    if !commands.is_empty() {
        out.push_str("  /** Op builders: `splice.op.item.split({item, at})` returns `{cmd: \"item.split\", item, at}`. */\n  op: {\n");
        for (group, rows) in groups(&ids) {
            let _ = writeln!(out, "    {group}: {{");
            for id in rows {
                let name = pascal(id);
                let _ = writeln!(
                    out,
                    "      {}(args: Omit<Splice.op.{name}, \"cmd\">): Splice.op.{name};",
                    method(id)
                );
            }
            out.push_str("    };\n");
        }
        out.push_str("  };\n");
    }
    let tools: Vec<&tools::Tool> = tools::TOOLS
        .iter()
        .filter(|tool| wanted(tool.name))
        .collect();
    for (group, names) in groups(&tools.iter().map(|tool| tool.name).collect::<Vec<_>>()) {
        let _ = writeln!(out, "  {group}: {{");
        for name in names {
            let tool = tools.iter().find(|tool| tool.name == name).expect("listed");
            tool_signature(&mut out, tool, false);
        }
        out.push_str("  };\n");
    }
    out.push_str("};\n");
    out
}

/// Shared types, the command list and every tool signature; command params come from a query.
fn index() -> String {
    let mut out = String::from(HEADER);
    out.push_str(concat!(
        "  /** edit.apply ops are `{cmd, ...params}`; refs are \"$name\". Before using commands, get\n",
        "   * their types in one call: splice.describe(\"clip.crop,effect.add\") (or commands.describe\n",
        "   * {format:\"ts\", query}); \"*\" returns everything. Tool docs below are first\n",
        "   * sentences; splice.describe(\"record.start\") gives a tool's full text. Commands:\n",
    ));
    // One line per group keeps discovery small as rows grow (T23.1).
    let mut by_group: Vec<(&str, Vec<&str>)> = Vec::new();
    for row in COMMANDS.iter().filter(|row| row.tier != Tier::Planned) {
        match by_group.iter_mut().find(|(group, _)| *group == row.group) {
            Some((_, ids)) => ids.push(row.id),
            None => by_group.push((row.group, vec![row.id])),
        }
    }
    for (group, ids) in by_group {
        let _ = writeln!(out, "   *   {group}: {}", ids.join(", "));
    }
    out.push_str(
        "   */\n  type Op = { cmd: string; ref?: `$${string}`; [param: string]: unknown };\n}\n",
    );
    out.push_str(API);
    let names: Vec<&str> = tools::TOOLS.iter().map(|tool| tool.name).collect();
    for (group, names) in groups(&names) {
        let _ = writeln!(out, "  {group}: {{");
        for name in names {
            let tool = tools::TOOLS
                .iter()
                .find(|tool| tool.name == name)
                .expect("listed");
            tool_signature(&mut out, tool, true);
        }
        out.push_str("  };\n");
    }
    out.push_str("};\n");
    out
}

fn command(out: &mut String, row: &Command) {
    let cut = row.tier.cut();
    let _ = writeln!(out, "    /** {} ({}, {cut}). */", row.title, row.risk);
    let _ = writeln!(out, "    interface {} extends Base {{", pascal(row.id));
    let _ = writeln!(out, "      cmd: \"{}\";", row.id);
    for (name, hint) in row.params {
        let (required, hint) = match hint.strip_prefix('!') {
            Some(hint) => (true, hint),
            None => (false, *hint),
        };
        let (kind, doc) = hint.split_once(" · ").unwrap_or((hint, ""));
        let (kind, default) = kind.split_once(" = ").unwrap_or((kind, ""));
        let ts = param_type(kind, doc);
        let mut notes = Vec::new();
        // Keep what the type drops: ID prefixes, ranges, defaults in prose.
        let plain = [
            "string", "id", "id[]", "frame", "frames", "int", "bool", "object",
        ];
        // The anchor shape is fully typed; other object prose carries ranges and defaults.
        if !plain.contains(&kind) && !ts.contains('"') && !kind.starts_with("{clip") {
            notes.push(kind.to_string());
        }
        if !doc.is_empty() {
            notes.push(doc.to_string());
        }
        if !default.is_empty() {
            notes.push(format!("@default {default}"));
        }
        if !notes.is_empty() {
            let _ = writeln!(out, "      /** {} */", notes.join(" · "));
        }
        let optional = if required { "" } else { "?" };
        let _ = writeln!(out, "      {name}{optional}: {ts};");
    }
    out.push_str("    }\n");
}

/// Every effect type and parameter, straight from `effects.json` (the validator's schema): types,
/// ranges, defaults and which numbers accept keys (T36.8).
fn effects(out: &mut String) {
    out.push_str("  /** Effect types and parameters (effects.json, the validator's own schema). */\n  interface Effects {\n");
    let schemas = effect_schemas()
        .as_object()
        .map_or(&[][..], |fields| fields);
    for (kind, schema) in schemas {
        let title = schema.field("title").and_then(Json::as_str).unwrap_or(kind);
        let applies = schema
            .field("appliesTo")
            .and_then(Json::as_str)
            .unwrap_or("");
        let _ = writeln!(out, "    /** {title}: {applies}. */\n    {kind}: {{");
        let params = schema.field("params").and_then(Json::as_object);
        for (name, spec) in params.into_iter().flatten() {
            let text = |key: &str| {
                let mut value = String::new();
                if let Some(field) = spec.field(key) {
                    splice_json::write(field, &mut value).expect("finite schema");
                }
                value
            };
            let ts = match spec.field("type").and_then(Json::as_str) {
                Some("number") if keyable(spec) => "number | { keys: Key[] }".to_string(),
                Some("number") => "number".into(),
                Some("bool") => "boolean".into(),
                Some("enum") => {
                    let options = spec
                        .field("options")
                        .and_then(Json::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Json::as_str)
                        .map(|option| format!("\"{option}\""))
                        .collect::<Vec<_>>()
                        .join(" | ");
                    // A keyed enum (camera.layout) keys its option names.
                    if keyable(spec) {
                        format!(
                            "{options} | {{ keys: {{ at: number; value: {options}; ease?: Ease }}[] }}"
                        )
                    } else {
                        options
                    }
                }
                _ => "string".into(),
            };
            let mut doc = text("label").trim_matches('"').to_string();
            if spec.field("min").is_some() {
                let _ = write!(doc, " · {}–{}", text("min"), text("max"));
            }
            if spec.field("type").and_then(Json::as_str) == Some("color") {
                doc.push_str(" · #rrggbb");
            }
            if spec.field("def").is_some() {
                let _ = write!(doc, " · @default {}", text("def"));
            }
            let _ = writeln!(out, "      /** {doc} */\n      {name}?: {ts};");
        }
        out.push_str("    };\n");
    }
    out.push_str("  }\n  type EffectType = keyof Effects;\n");
}

/// Maps the registry's parameter mini-spec (`type = default · doc`) to a TypeScript type.
fn param_type(kind: &str, doc: &str) -> String {
    let words = |text: &str| -> Option<String> {
        let tokens: Vec<&str> = text.split('|').map(str::trim).collect();
        let literal = |token: &str| {
            !token.is_empty()
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_' | '.'))
        };
        (tokens.len() > 1 && tokens.iter().all(|token| literal(token))).then(|| {
            tokens
                .iter()
                .map(|token| match *token {
                    "null" => "null".to_string(),
                    "string" => "string".to_string(),
                    token => format!("\"{token}\""),
                })
                .collect::<Vec<_>>()
                .join(" | ")
        })
    };
    let kind = kind.trim();
    if kind.starts_with("{clip") {
        return "{ clip: string; sourceFrame?: number } | { pinned: true; frame?: number }".into();
    }
    if kind.starts_with("{amount,attack,release}") {
        return "{ amount: number; attack?: number; release?: number } | null".into();
    }
    if kind.starts_with("{x,y}") {
        let mut point = "{ x: number; y: number }".to_string();
        // A zoom can follow the cursor instead of a point (T46.2).
        if kind.contains("\"cursor\"") {
            point.push_str(" | \"cursor\"");
        }
        if kind.contains("null") {
            point.push_str(" | null");
        }
        return point;
    }
    if kind.starts_with("{x,y,w,h}") {
        return "Rect | null".into();
    }
    if kind.starts_with("[{at, value") {
        return "Key[]".into();
    }
    if kind.starts_with("[{at, x, y, w, h") {
        return "{ at: number; x: number; y: number; w: number; h: number; ease?: Ease }[]".into();
    }
    if kind.starts_with("[{at, x, y}]") {
        return "{ at: number; x: number; y: number }[]".into();
    }
    if kind.starts_with("[[from, to]]") {
        return "[number, number][]".into();
    }
    if kind.starts_with("{x, y, w, h}") {
        return "Rect".into();
    }
    if kind.starts_with("{title, subtitle}") {
        return "{ title?: string; subtitle?: string }".into();
    }
    if kind.starts_with("number | {keys}") {
        return "number | { keys: Key[] }".into();
    }
    if kind.starts_with("effect type") {
        return "EffectType".into();
    }
    if kind.starts_with("ease") {
        return "Ease".into();
    }
    if kind.starts_with("\"timeline\" | frames") {
        return "\"timeline\" | number".into();
    }
    // A title's inline look (T42.2): effect parameters without keys, and motion presets.
    for (fields, effect) in [
        ("{x, y, w, h, fit, wrap}", "layout"),
        ("{font, weight, size", "text"),
        ("{preset, unit, duration, stagger}", "animate"),
    ] {
        if kind.starts_with(fields) {
            return inline_type(effect);
        }
    }
    if kind.starts_with("{preset, duration, intensity}") {
        return "{ preset: \"fade\" | \"sink\" | \"shrink\" | \"slide\"; duration?: number; intensity?: number }".into();
    }
    if kind.starts_with("{preset, period, intensity}") {
        return "{ preset: \"pulse\" | \"float\" | \"wiggle\" | \"shake\"; period?: number; intensity?: number }".into();
    }
    if kind.starts_with("front | back") {
        return "\"front\" | \"back\" | { above: string } | { below: string } | number".into();
    }
    if kind.starts_with("{wordId") {
        return "Record<string, { text?: string; emph?: boolean; emoji?: string; hidden?: boolean } | null>".into();
    }
    if kind.contains("rectangle") {
        return "Rect".into();
    }
    if kind.contains("[]") {
        let number = kind.starts_with("int");
        return if number { "number[]" } else { "string[]" }.into();
    }
    if kind.starts_with("bool") {
        return "boolean".into();
    }
    if let Some(union) = words(kind) {
        return union;
    }
    if kind == "object" {
        return "Record<string, unknown>".into();
    }
    if kind.starts_with("frames")
        || kind.starts_with("int")
        || kind.starts_with("number")
        || kind.starts_with("scale")
        || kind.starts_with(|c: char| c.is_ascii_digit())
        || kind.contains("media frame")
    {
        return "number".into();
    }
    if kind.starts_with("frame") {
        return "At".into();
    }
    // Any other `{a, b}` / `[{a, b}]` shape: text-like fields are strings, the rest numbers.
    if let Some(inner) = kind.strip_prefix('{').or_else(|| kind.strip_prefix("[{"))
        && let Some((fields, _)) = inner.split_once('}')
    {
        let fields = fields
            .split(',')
            .map(|field| field.trim().trim_end_matches('?'))
            .filter(|field| !field.is_empty())
            .map(|field| {
                let text = matches!(
                    field,
                    "text" | "name" | "id" | "label" | "action" | "app" | "media"
                );
                format!("{field}: {}", if text { "string" } else { "number" })
            })
            .collect::<Vec<_>>()
            .join("; ");
        let shape = format!("{{ {fields} }}");
        return if kind.starts_with('[') {
            format!("{shape}[]")
        } else {
            shape
        };
    }
    // `preset · bold|karaoke|…`: the choices follow the word.
    if !kind.contains(' ')
        && let Some(union) = words(doc.split(" · ").next().unwrap_or(""))
    {
        return union;
    }
    "string".into()
}

/// The constant parameters of an effect type as an object type (inline, no keys).
fn inline_type(kind: &str) -> String {
    let mut fields = Vec::new();
    for (name, spec) in effect_schemas()
        .field(kind)
        .and_then(|schema| schema.field("params"))
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        let ty = match spec.field("type").and_then(Json::as_str) {
            Some("number") => "number".to_string(),
            Some("bool") => "boolean".to_string(),
            Some("enum") => spec
                .field("options")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(Json::as_str)
                .map(|option| format!("\"{option}\""))
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "string".to_string(),
        };
        fields.push(format!("{name}?: {ty}"));
    }
    format!("{{ {} }}", fields.join("; "))
}

/// One tool's declaration; `brief` (the index) keeps the description's first sentence.
fn tool_signature(out: &mut String, tool: &tools::Tool, brief: bool) {
    let description = match tool.description.split_once(". ") {
        Some((first, _)) if brief => first,
        _ => tool.description.trim_end_matches('.'),
    };
    let _ = writeln!(out, "    /** {description}. */");
    let mut args = String::new();
    let mut any_required = false;
    for (name, spec) in tool.input {
        // Ending a session requires an explicit project and session even in code.run.
        let required = tool.required.contains(name)
            && (tool.name == "session.end" || !matches!(*name, "projectId" | "sessionId"));
        any_required |= required;
        let kind = if *name == "ops" {
            "Splice.Op[]".to_string()
        } else {
            schema_type(spec)
        };
        let _ = write!(args, " {name}{}: {kind};", if required { "" } else { "?" });
    }
    let mut result = String::new();
    let guaranteed = tool.success.first().copied().unwrap_or(&[]);
    for (name, spec) in tool.output {
        let optional = if guaranteed.contains(name) { "" } else { "?" };
        // The index lists only guaranteed fields; describe gives the whole result (T23.1).
        if brief && !optional.is_empty() {
            continue;
        }
        let _ = write!(result, " {name}{optional}: {};", schema_type(spec));
    }
    if brief && tool.output.len() > guaranteed.len() {
        result.push_str(" /* … */");
    }
    let _ = writeln!(
        out,
        "    {}(args{}: {{{args} }}): {{{result} }};",
        method(tool.name),
        if any_required { "" } else { "?" }
    );
}

/// Maps a tool's JSON schema shorthand to a TypeScript type.
fn schema_type(spec: &str) -> String {
    if !spec.starts_with('{') {
        return primitive(spec).into();
    }
    let schema = splice_json::parse(spec).expect("checked tool schema");
    json_type(&schema)
}

fn json_type(schema: &Json) -> String {
    if schema.field("pattern").is_some() || schema.field("anyOf").is_some() {
        // The only alternatives in the tool schemas are revisions.
        return "Splice.Revision".into();
    }
    if let Some(values) = schema.field("enum").and_then(Json::as_array) {
        return values
            .iter()
            .filter_map(|value| match value {
                Json::String(text) => Some(format!("\"{text}\"")),
                // Numeric choices (export.start resolution).
                value => value.as_u64().map(|n| n.to_string()),
            })
            .collect::<Vec<_>>()
            .join(" | ");
    }
    if let Some(fields) = schema.field("properties").and_then(Json::as_object)
        && schema.field("required").is_none()
    {
        // An options object (commands.describe {music}): every field optional.
        let fields: Vec<String> = fields
            .iter()
            .map(|(name, field)| format!("{name}?: {}", json_type(field)))
            .collect();
        return format!("{{ {} }}", fields.join("; "));
    }
    match schema.field("type") {
        Some(Json::String(kind)) if kind == "array" => match schema.field("items") {
            // A union element type needs parentheses: ("a" | "b")[].
            Some(items) => match json_type(items) {
                item if item.contains('|') => format!("({item})[]"),
                item => format!("{item}[]"),
            },
            None => "unknown[]".into(),
        },
        Some(Json::String(kind)) => primitive(kind).into(),
        Some(Json::Array(kinds)) => kinds
            .iter()
            .filter_map(Json::as_str)
            .map(primitive)
            .collect::<Vec<_>>()
            .join(" | "),
        _ => "unknown".into(),
    }
}

fn primitive(kind: &str) -> &'static str {
    match kind {
        "string" => "string",
        "integer" | "number" => "number",
        "boolean" => "boolean",
        "array" => "unknown[]",
        "object" => "Record<string, unknown>",
        "null" => "null",
        _ => "unknown",
    }
}

/// Groups dotted names by their first segment, keeping first-seen order.
fn groups(names: &[&'static str]) -> Vec<(&'static str, Vec<&'static str>)> {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for &name in names {
        let group = name.split('.').next().unwrap_or(name);
        match groups.iter_mut().find(|(known, _)| *known == group) {
            Some((_, names)) => names.push(name),
            None => groups.push((group, vec![name])),
        }
    }
    groups
}

fn method(name: &str) -> &str {
    name.split_once('.').map_or(name, |(_, rest)| rest)
}

/// `overlay.replaceSpine` → `OverlayReplaceSpine`.
fn pascal(id: &str) -> String {
    id.split('.')
        .flat_map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase())
                .into_iter()
                .chain(chars)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T36.8 (M8): a structured argument (`[…]`, `{…}`) never degrades to `string` in the
    /// declarations, and every effects.json type and parameter is declared.
    #[test]
    fn structured_hints_get_structured_types_and_effects_are_complete() {
        for row in COMMANDS {
            for (name, hint) in row.params {
                let hint = hint.trim_start_matches('!');
                let (kind, doc) = hint.split_once(" · ").unwrap_or((hint, ""));
                let kind = kind.split_once(" = ").map_or(kind, |(kind, _)| kind);
                if kind.starts_with(['[', '{']) {
                    let ts = param_type(kind, doc);
                    assert!(ts != "string", "{} {name}: {kind} typed as string", row.id);
                }
            }
        }
        let mut out = String::new();
        effects(&mut out);
        for (kind, schema) in effect_schemas().as_object().unwrap() {
            assert!(out.contains(&format!("\n    {kind}: {{")), "{kind}");
            for (name, _) in schema.field("params").and_then(Json::as_object).unwrap() {
                assert!(out.contains(&format!("{name}?:")), "{kind}.{name}");
            }
        }
        assert!(out.contains("scale?: number | { keys: Key[] }"));
        assert_eq!(
            json_type(
                &splice_json::parse(r#"{"type":"array","items":{"enum":["a","b"]}}"#).unwrap()
            ),
            "(\"a\" | \"b\")[]"
        );
    }
}
