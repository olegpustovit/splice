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
