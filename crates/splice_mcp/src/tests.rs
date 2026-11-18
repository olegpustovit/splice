use super::*;
use std::io::Cursor;
use std::path::Path;

#[test]
fn stdio_conformance_transcript() {
    let transcript = include_str!("../../../fixtures/mcp/rpc-conformance.jsonl");
    for connection in transcript.split("\n\n") {
        let mut input = Vec::new();
        let mut expected = Vec::new();
        for line in connection.lines() {
            match line.split_at(2) {
                ("> ", request) => {
                    input.extend_from_slice(request.as_bytes());
                    input.push(b'\n');
                }
                ("< ", response) => expected.push(splice_json::parse(response).unwrap()),
                _ => panic!("invalid transcript line {line:?}"),
            }
        }
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        let actual: Vec<_> = output
            .lines()
            .map(|line| splice_json::parse(line).unwrap())
            .collect();
        assert_eq!(actual.len(), expected.len());
        for (response, expected) in actual.iter().zip(&expected) {
            assert_eq!(response, expected, "wire response differs");
        }
    }
}

#[test]
fn tools_list_golden() {
    let mut input = Cursor::new(concat!(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        "\n"
    ));
    let mut output = Vec::new();
    serve(&mut input, &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    let response = output.lines().nth(1).unwrap();
    let message = splice_json::parse(response).unwrap();
    let result = message.field("result").unwrap();
    let tools = result.field("tools").unwrap().as_array().unwrap();
    assert_eq!(tools.len(), 19);
    for tool in tools {
        for field in ["inputSchema", "outputSchema"] {
            assert_eq!(
                tool.field(field).unwrap().field("type").unwrap().as_str(),
                Some("object")
            );
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp/tools-list.json");
    let mut json = String::new();
    splice_json::write(result, &mut json).unwrap();
    splice_testkit::golden(&path, json.as_bytes()).unwrap();
}

#[test]
fn stdio_recovers_after_oversized_and_invalid_utf8_lines() {
    use std::io::{BufReader, Read, repeat};

    let input = repeat(b' ').take(16 * 1024 * 1024 + 128).chain(Cursor::new(
        b"\n\xff\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n",
    ));
    let mut output = Vec::new();
    serve(&mut BufReader::new(input), &mut output).unwrap();
    let output = String::from_utf8(output).unwrap();
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        [
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Message exceeds 16 MiB"}}"#,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]
    );
}

#[test]
fn tool_transcripts_run_against_live_store() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let tmp = std::env::temp_dir().join(format!("splice-tool-transcripts-{}", std::process::id()));
    let test_runs = TestDirectory(tmp);
    std::fs::create_dir_all(&test_runs.0).unwrap();
    let tools_path = fixtures.join("mcp/tools-list.json");
    let list = splice_json::parse(&std::fs::read_to_string(tools_path).unwrap()).unwrap();
    for tool in list.field("tools").unwrap().as_array().unwrap() {
        let name = tool.field("name").unwrap().as_str().unwrap();
        let folder = test_runs.0.join(name);
        let sample = folder.join("sample.splice");
        let new = folder.join("new.splice");
        std::fs::create_dir_all(sample.join("transcripts")).unwrap();
        for relative in ["project.json", "media.json", "transcripts/med_screen.json"] {
            std::fs::copy(
                fixtures.join("projects/sample.splice").join(relative),
                sample.join(relative),
            )
            .unwrap();
        }
        let take = sample.join("takes/med_screen");
        std::fs::create_dir_all(&take).unwrap();
        std::fs::write(take.join("events.json"), r#"{"events":[{"t":0.2,"k":"move","x":0.25,"y":0.5},{"t":0.4,"k":"down","b":0},{"t":0.5,"k":"key","ch":"private","keyCode":4}]}"#).unwrap();

        let transcript =
            std::fs::read_to_string(fixtures.join(format!("mcp/{name}.jsonl"))).unwrap();
        let mut input = String::from(concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixtures","version":"1"}}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
        ));
        let mut expected = Vec::new();
        for line in transcript.lines() {
            match line.split_at(2) {
                ("> ", request) => {
                    let request = request
                        .replace(r#""$sample""#, &quoted_path(&sample))
                        .replace(r#""$new""#, &quoted_path(&new));
                    input.push_str(&request);
                    input.push('\n');
                }
                ("< ", reply) => expected.push(splice_json::parse(reply).unwrap()),
                _ => panic!("invalid transcript line {line:?}"),
            }
        }
        let mut output = Vec::new();
        serve(&mut Cursor::new(input), &mut output).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("private"), "{name} disclosed key text");
        let mut actual = output.lines().skip(1);
        for (index, expected) in expected.iter().enumerate() {
            let reply = actual
                .next()
                .unwrap_or_else(|| panic!("{name}: missing reply {}", index + 1));
            let reply = splice_json::parse(reply).unwrap();
            assert!(
                matches_fixture(expected, &reply),
                "{name} reply {} did not match {expected:?}: {reply:?}",
                index + 1
            );
        }
        assert_eq!(actual.next(), None, "{name}: extra reply");
        if name == "edit.apply" {
            let reopened = splice_core::history::Store::open(&sample).unwrap();
            assert_eq!(reopened.project().rev, 1);
            assert_eq!(reopened.records().len(), 1);
        }
        if name == "record.start" {
            assert!(!new.exists(), "capture dry run created a project");
        }
    }
}

fn quoted_path(path: &Path) -> String {
    let mut result = String::new();
    splice_json::write(&Json::String(path.to_str().unwrap().into()), &mut result).unwrap();
    result
}

fn matches_fixture(expected: &Json, actual: &Json) -> bool {
    match expected {
        Json::String(value) if value == "…" => true,
        Json::Object(fields) => {
            actual.as_object().is_some()
                && fields.iter().all(|(key, expected)| {
                    actual
                        .field(key)
                        .is_some_and(|value| matches_fixture(expected, value))
                })
        }
        Json::Array(items) => actual.as_array().is_some_and(|other| {
            items.len() == other.len()
                && items.iter().zip(other).all(|(a, b)| matches_fixture(a, b))
        }),
        _ => expected == actual,
    }
}

struct TestDirectory(std::path::PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
