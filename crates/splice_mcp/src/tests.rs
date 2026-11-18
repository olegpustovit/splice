use super::*;
use std::io::{self, BufRead, Cursor, Write};
use std::path::Path;

fn isolated_serve(input: &mut dyn BufRead, output: &mut dyn Write) -> io::Result<()> {
    rpc::serve_with_log(input, output, None)
}

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
        isolated_serve(&mut Cursor::new(input), &mut output).unwrap();
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
    isolated_serve(&mut input, &mut output).unwrap();
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
    isolated_serve(&mut BufReader::new(input), &mut output).unwrap();
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
        isolated_serve(&mut Cursor::new(input), &mut output).unwrap();
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

#[test]
fn friction_transcript_tracks_session_recovery_and_writes_repro() {
    let root =
        TestDirectory(std::env::temp_dir().join(format!("splice-friction-{}", std::process::id())));
    let global = root.0.join("state");
    let fixtures = root.0.join("fixtures");
    let project = root.0.join("new.splice");
    std::fs::create_dir_all(&root.0).unwrap();
    let transcript = include_str!("../../../fixtures/mcp/friction-recovery.jsonl");
    for (index, connection) in transcript.split("\n\n").enumerate() {
        let mut input = String::from(concat!(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"fixtures","version":"1"}}}"#,
            "\n",
        ));
        let mut expected = Vec::new();
        for line in connection.lines() {
            match line.split_at(2) {
                ("> ", request) => {
                    input.push_str(&request.replace(r#""$new""#, &quoted_path(&project)));
                    input.push('\n');
                }
                ("< ", reply) => expected.push(splice_json::parse(reply).unwrap()),
                _ => panic!("invalid transcript line {line:?}"),
            }
        }
        let mut output = Vec::new();
        rpc::serve_with_log(&mut Cursor::new(input), &mut output, Some(global.clone())).unwrap();
        let output = String::from_utf8(output).unwrap();
        let replies: Vec<_> = output.lines().skip(1).collect();
        assert_eq!(replies.len(), expected.len());
        for (line, expected) in replies.into_iter().zip(&expected) {
            assert!(
                matches_fixture(expected, &splice_json::parse(line).unwrap()),
                "{line}"
            );
        }
        if index == 0 {
            for path in [
                global.join("friction.jsonl"),
                project.join("friction.jsonl"),
            ] {
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)
                    .unwrap()
                    .write_all(b"{\"partial\":\"\xff")
                    .unwrap();
            }
        }
    }

    let entries = friction::read_entries(&global.join("friction.jsonl")).unwrap();
    assert_eq!(
        entries,
        friction::read_entries(&project.join("friction.jsonl")).unwrap()
    );
    assert_eq!(entries.len(), 3);
    let error = &entries[0];
    assert_eq!(
        error.field("client").and_then(Json::as_str),
        Some("fixtures 1")
    );
    assert_eq!(error.field("session").and_then(Json::as_str), Some("ses_1"));
    assert_eq!(error.field("recovered").and_then(Json::as_bool), Some(true));
    assert_eq!(
        error.field("command").and_then(Json::as_str),
        Some("marker.add")
    );
    let next: Vec<_> = error
        .field("next")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|call| call.field("tool").and_then(Json::as_str).unwrap())
        .collect();
    assert_eq!(next, ["timeline.snapshot", "edit.apply", "history.undo"]);
    assert_eq!(
        entries[1].field("code").and_then(Json::as_str),
        Some("NOTE")
    );
    assert_eq!(
        entries[2].field("code").and_then(Json::as_str),
        Some("AGENT_UNDO")
    );

    let id = error.field("id").and_then(Json::as_str).unwrap().to_owned();
    let mut cli = Vec::new();
    friction::run(
        &global,
        &fixtures,
        &["list".into(), "--open".into()],
        &mut cli,
    )
    .unwrap();
    assert!(
        String::from_utf8(cli.clone())
            .unwrap()
            .contains("marker.add")
    );
    cli.clear();
    friction::run(&global, &fixtures, &["show".into(), id.clone()], &mut cli).unwrap();
    assert_eq!(
        splice_json::parse(std::str::from_utf8(&cli).unwrap()).unwrap(),
        *error
    );
    let arguments = ["repro".into(), id.clone()];
    cli.clear();
    friction::run(&global, &fixtures, &arguments, &mut cli).unwrap();
    let reproduced =
        std::fs::read_to_string(fixtures.join(format!("friction-{id}.jsonl"))).unwrap();
    assert!(reproduced.contains(r#""name":"edit.apply""#));
    assert!(reproduced.contains(r#""code":"BAD_ARGS""#));
    assert!(reproduced.contains(r#""cmd":"marker.add""#));
    assert!(!reproduced.contains(project.to_str().unwrap()));
    assert_eq!(
        friction::run(&global, &fixtures, &arguments, &mut cli)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );

    friction::run(
        &global,
        &fixtures,
        &["resolve".into(), id.clone(), "1234abc".into()],
        &mut cli,
    )
    .unwrap();
    let updated = friction::read_entries(&global.join("friction.jsonl")).unwrap();
    assert_eq!(
        updated[0].field("resolved").and_then(Json::as_str),
        Some("1234abc")
    );
    assert_eq!(
        updated,
        friction::read_entries(&project.join("friction.jsonl")).unwrap()
    );
    cli.clear();
    friction::run(
        &global,
        &fixtures,
        &["list".into(), "--open".into()],
        &mut cli,
    )
    .unwrap();
    assert!(!std::str::from_utf8(&cli).unwrap().contains(&id));
}
