//! TypeScript declarations for code mode, generated from `COMMANDS` and the tool schemas.
use super::{Json, tools};
use splice_core::commands::{COMMANDS, Command, Tier};
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
  /** A rectangle normalized to the source frame (0–1). */
  type Rect = { x: number; y: number; w: number; h: number };
  /** "r12" or 12. */
  type Revision = number | string;
"#;

const API: &str = r#"
declare const splice: {
  /** The projectId and sessionId passed to code.run. Tool calls default to them. */
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
        .filter(|tool| !tool.name.starts_with("record.") && wanted(tool.name))
        .collect();
    for (group, names) in groups(&tools.iter().map(|tool| tool.name).collect::<Vec<_>>()) {
        let _ = writeln!(out, "  {group}: {{");
        for name in names {
            let tool = tools.iter().find(|tool| tool.name == name).expect("listed");
            tool_signature(&mut out, tool);
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
        "   * {format:\"ts\", query}); \"*\" returns everything. Commands:\n",
    ));
    for row in COMMANDS.iter().filter(|row| row.tier != Tier::Planned) {
        let _ = writeln!(out, "   *   {} — {}", row.id, row.title);
    }
    out.push_str(
        "   */\n  type Op = { cmd: string; ref?: `$${string}`; [param: string]: unknown };\n}\n",
    );
    out.push_str(API);
    let names: Vec<&str> = tools::TOOLS
        .iter()
        .map(|tool| tool.name)
        .filter(|name| !name.starts_with("record."))
        .collect();
    for (group, names) in groups(&names) {
        let _ = writeln!(out, "  {group}: {{");
        for name in names {
            let tool = tools::TOOLS
                .iter()
                .find(|tool| tool.name == name)
                .expect("listed");
            tool_signature(&mut out, tool);
        }
        out.push_str("  };\n");
    }
    out.push_str("};\n");
    out
}

fn command(out: &mut String, row: &Command) {
    let cut = match row.tier {
        Tier::First => "0.1",
        Tier::Second => "0.2",
        Tier::Planned => "0.3, planned: fails with NOT_IMPLEMENTED",
    };
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
        let point = "{ x: number; y: number }";
        return if kind.contains("null") {
            format!("{point} | null")
        } else {
            point.into()
        };
    }
    if kind.starts_with("{x,y,w,h}") {
        return "Rect | null".into();
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
    // `preset · bold|karaoke|…`: the choices follow the word.
    if !kind.contains(' ')
        && let Some(union) = words(doc.split(" · ").next().unwrap_or(""))
    {
        return union;
    }
    "string".into()
}

fn tool_signature(out: &mut String, tool: &tools::Tool) {
    let _ = writeln!(out, "    /** {} */", tool.description);
    let mut args = String::new();
    let mut any_required = false;
    for (name, spec) in tool.input {
        // code.run supplies projectId and sessionId.
        let required = tool.required.contains(name) && !matches!(*name, "projectId" | "sessionId");
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
        let _ = write!(result, " {name}{optional}: {};", schema_type(spec));
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
            .filter_map(Json::as_str)
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(" | ");
    }
    match schema.field("type") {
        Some(Json::String(kind)) if kind == "array" => match schema.field("items") {
            Some(items) => format!("{}[]", json_type(items)),
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
