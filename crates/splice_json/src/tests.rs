use super::*;
use std::path::Path;

#[test]
fn tokenizer_uses_input_spans() {
    let input = " [ \"日本語\", -1.4e+2 ]";
    let mut offset = 0;
    for (kind, expected) in [
        (Kind::ArrayStart, "["),
        (Kind::String, "\"日本語\""),
        (Kind::Comma, ","),
        (Kind::Number, "-1.4e+2"),
        (Kind::ArrayEnd, "]"),
        (Kind::End, ""),
    ] {
        let token = next(input, offset).unwrap();
        assert_eq!(token.kind, kind);
        let raw = token.text(input).unwrap();
        assert_eq!(raw, expected);
        assert_eq!(raw.as_ptr(), input[token.start..].as_ptr());
        offset = token.end;
    }
    assert!(next(input, 5).is_err());
}

#[test]
fn malformed_input_reports_offset_and_path() {
    for (input, offset, path) in [
        ("", 0, "$"),
        ("{", 1, "$"),
        ("{ \"clip\": [1, 0.]} ", 16, "$.clip[1]"),
        (r#"{"a.b":[-01]}"#, 10, r#"$["a.b"][0]"#),
        (r#"{"a":[{"x":"\uD800z"}]}"#, 18, "$.a[0].x"),
        (r#"["\uDC00"]"#, 2, "$[0]"),
        (r#"{"a":1,"\u0061":2}"#, 7, "$.a"),
        (r#"[true,]"#, 6, "$[1]"),
        ("true\u{000c}", 4, "$"),
        ("01", 1, "$"),
        ("[1e+]", 4, "$[0]"),
        ("{\"n\":1e900}", 5, "$.n"),
        ("[1 2]", 3, "$"),
    ] {
        let error = parse(input).unwrap_err();
        assert_eq!(
            (error.offset, error.path.as_str()),
            (offset, path),
            "{input:?}: {error}"
        );
    }
    let deeply_nested = "[".repeat(MAX_DEPTH + 2);
    assert!(
        parse(&deeply_nested)
            .unwrap_err()
            .message
            .contains("nesting")
    );
}

#[test]
fn canonical_fixtures_round_trip_byte_for_byte() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut verified = 0;
    round_trip_tree(&directory, &mut verified);
    assert!(verified > 0);
}

fn round_trip_tree(directory: &Path, verified: &mut usize) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            round_trip_tree(&path, verified);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let input = std::fs::read_to_string(&path).unwrap();
            let value = parse(&input).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            let mut out = String::new();
            write(&value, &mut out).unwrap();
            assert_eq!(out, input, "{}", path.display());
            *verified += 1;
        }
    }
}

#[test]
fn warm_write_to_reused_buffer_allocates_nothing() {
    let value = parse(r#"{"clip":"clp_2x","escaped":"\uD83C\uDFAF\n","numbers":[1e100,-1.23,5e-324,18446744073709551615]}"#).unwrap();
    let mut out = String::new();
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            write(&value, &mut out).unwrap();
        },
    );
    assert_eq!(
        out,
        "{\"clip\":\"clp_2x\",\"escaped\":\"🎯\\n\",\"numbers\":[1e100,-1.23,5e-324,18446744073709551615]}"
    );
}

#[test]
fn float_round_trips_include_extremes_and_negative_zero() {
    let mut rng = splice_testkit::Xorshift::new(17);
    let mut out = String::new();
    for number in [
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        -0.0,
        1.0,
        1e-5,
    ]
    .into_iter()
    .chain((0..4_096).map(|_| f64::from_bits(rng.next_u64())))
    .filter(|n| n.is_finite())
    {
        write(&Value::Float(number), &mut out).unwrap();
        assert_eq!(
            out.parse::<f64>().unwrap().to_bits(),
            number.to_bits(),
            "{out}"
        );
        let value = parse(&out).unwrap();
        write(&value, &mut out).unwrap();
        assert_eq!(
            out.parse::<f64>().unwrap().to_bits(),
            number.to_bits(),
            "{out}"
        );
    }
    write(&Value::Float(123_400_000.0), &mut out).unwrap();
    assert_eq!(out, "1234e5");
    assert!(write(&Value::Float(f64::NAN), &mut out).is_err());
    assert!(out.is_empty());
}
