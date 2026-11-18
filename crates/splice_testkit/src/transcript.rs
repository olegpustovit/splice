use splice_json::Value as Json;
use std::{fs, io, path::Path};

/// Replays a transcript one connection at a time. Blank lines reconnect to the server.
///
/// `prelude` contains optional fixture lines to replay before each connection. `paths`
/// binds exact JSON string values of `path` fields to disposable files or packages.
/// The transport receives newline-delimited requests and returns its wire replies.
///
/// Replies must appear in order. Expectation objects specify fields to check. Arrays
/// match by position and length, and the string `"…"` matches any present value.
pub fn run(
    path: &Path,
    prelude: &str,
    paths: &[(&str, &Path)],
    transport: &mut dyn FnMut(&[u8]) -> io::Result<Vec<u8>>,
) -> io::Result<()> {
    let transcript = fs::read_to_string(path)?;
    let mut opening = Connection::default();
    for (index, line) in prelude.lines().enumerate() {
        if !line.trim().is_empty() {
            opening.push(Path::new("<transcript prelude>"), index + 1, line, paths)?;
        }
    }
    opening.check_complete(Path::new("<transcript prelude>"))?;
    let mut connection = opening.clone();
    let mut first_line = None;
    let mut count = 0;
    for (index, line) in transcript.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() {
            if let Some(start) = first_line.take() {
                connection.replay(path, start, transport)?;
                count += 1;
                connection.clone_from(&opening);
            }
        } else {
            first_line.get_or_insert(number);
            connection.push(path, number, line, paths)?;
        }
    }
    if let Some(start) = first_line {
        connection.replay(path, start, transport)?;
    } else if count == 0 {
        return Err(invalid(path, 1, "empty transcript"));
    }
    Ok(())
}

#[derive(Clone, Default)]
struct Connection {
    input: Vec<u8>,
    expected: Vec<(usize, Json)>,
    unanswered: Option<usize>,
}

impl Connection {
    fn push(
        &mut self,
        file: &Path,
        line: usize,
        content: &str,
        paths: &[(&str, &Path)],
    ) -> io::Result<()> {
        if let Some(request) = content.strip_prefix("> ") {
            self.check_complete(file)?;
            let mut parsed = splice_json::parse(request).ok();
            let notification = parsed.as_ref().is_some_and(is_notification);
            if let Some(value) = &mut parsed
                && bind_paths(value, paths).map_err(|error| invalid(file, line, &error))?
            {
                let mut wire = String::new();
                splice_json::write(value, &mut wire).map_err(io::Error::other)?;
                self.input.extend_from_slice(wire.as_bytes());
            } else {
                // Invalid requests can be intentional transport tests.
                self.input.extend_from_slice(request.as_bytes());
            }
            self.input.push(b'\n');
            self.unanswered = (!notification).then_some(line);
        } else if let Some(reply) = content.strip_prefix("< ") {
            if self.unanswered.take().is_none() {
                return Err(invalid(file, line, "an expected reply requires a request"));
            }
            let value = splice_json::parse(reply)
                .map_err(|error| invalid(file, line, &format!("invalid expected JSON: {error}")))?;
            self.expected.push((line, value));
        } else {
            return Err(invalid(file, line, "expected a `> ` request or `< ` reply"));
        }
        Ok(())
    }

    fn check_complete(&self, file: &Path) -> io::Result<()> {
        if let Some(line) = self.unanswered {
            return Err(invalid(file, line, "request has no expected reply"));
        }
        Ok(())
    }

    fn replay(
        &self,
        file: &Path,
        start: usize,
        transport: &mut dyn FnMut(&[u8]) -> io::Result<Vec<u8>>,
    ) -> io::Result<()> {
        self.check_complete(file)?;
        let output = transport(&self.input).map_err(|error| {
            io::Error::new(error.kind(), format!("{}:{start}: {error}", file.display()))
        })?;
        if !output.is_empty() && !output.ends_with(b"\n") {
            return Err(invalid(file, start, "reply is missing a newline delimiter"));
        }
        let mut replies = str::from_utf8(&output)
            .map_err(|error| invalid(file, start, &format!("output is not UTF-8: {error}")))?
            .lines();
        for (line, expected) in &self.expected {
            let reply = replies
                .next()
                .ok_or_else(|| invalid(file, *line, "missing reply"))?;
            let actual = splice_json::parse(reply)
                .map_err(|error| invalid(file, *line, &format!("invalid reply JSON: {error}")))?;
            if actual.field("result").is_some() && actual.field("error").is_some() {
                return Err(invalid(
                    file,
                    *line,
                    "a reply cannot contain both result and error",
                ));
            }
            compare(expected, &actual, "$").map_err(|error| invalid(file, *line, &error))?;
        }
        if let Some(extra) = replies.next() {
            return Err(invalid(
                file,
                start,
                &format!("unexpected extra reply: {extra}"),
            ));
        }
        Ok(())
    }
}

fn is_notification(value: &Json) -> bool {
    value.field("jsonrpc").and_then(Json::as_str) == Some("2.0")
        && value.field("method").and_then(Json::as_str).is_some()
        && ["id", "result", "error"]
            .iter()
            .all(|field| value.field(field).is_none())
}

fn bind_paths(value: &mut Json, paths: &[(&str, &Path)]) -> Result<bool, String> {
    let mut bound = false;
    match value {
        Json::Object(fields) => {
            for (key, item) in fields {
                if key == "path"
                    && let Json::String(name) = item
                {
                    if let Some((_, path)) = paths.iter().find(|(token, _)| token == name) {
                        *name = path
                            .to_str()
                            .ok_or_else(|| format!("{name} path is not UTF-8"))?
                            .into();
                        bound = true;
                    } else if matches!(name.as_str(), "$sample" | "$new" | "$project") {
                        return Err(format!("provide a disposable path for {name}"));
                    }
                } else {
                    bound |= bind_paths(item, paths)?;
                }
            }
        }
        Json::Array(items) => {
            for item in items {
                bound |= bind_paths(item, paths)?;
            }
        }
        _ => {}
    }
    Ok(bound)
}

fn compare(expected: &Json, actual: &Json, path: &str) -> Result<(), String> {
    match (expected, actual) {
        (Json::String(value), _) if value == "…" => Ok(()),
        (Json::Object(fields), Json::Object(_)) => {
            for (key, expected) in fields {
                let path = format!("{path}.{key}");
                let actual = actual
                    .field(key)
                    .ok_or_else(|| format!("{path}: missing field"))?;
                compare(expected, actual, &path)?;
            }
            Ok(())
        }
        (Json::Array(expected), Json::Array(actual)) => {
            if expected.len() != actual.len() {
                return Err(format!(
                    "{path}: expected {} elements, got {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                compare(expected, actual, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ if expected == actual => Ok(()),
        _ => Err(format!("{path}: expected {expected:?}, got {actual:?}")),
    }
}

fn invalid(path: &Path, line: usize, message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}:{line}: {message}", path.display()),
    )
}
