use super::*;

fn limits() -> Limits {
    Limits {
        memory: 32 << 20,
        stack: 1 << 20,
        wall: Duration::from_millis(300),
        calls: 3,
        source: 64 << 10,
        output: 64,
    }
}

fn eval(source: &str) -> Outcome {
    run("", source, &limits(), &mut |name, args| {
        if name == "fail" {
            Err(r#"{"code":"BAD","fix":"Do it right."}"#.into())
        } else {
            Ok(format!(r#"{{"name":"{name}","args":{args}}}"#))
        }
    })
}

fn kind(outcome: &Outcome) -> Option<Kind> {
    outcome.error.as_ref().map(|error| error.kind)
}

#[test]
fn returns_json_logs_and_host_results() {
    let outcome = eval(
        "const r = __host('a.b', JSON.stringify({x: 1}));\nconsole.log('got', r.args, 2);\nreturn {r, n: 1 + 1};",
    );
    assert_eq!(outcome.error, None);
    assert_eq!(
        outcome.result,
        r#"{"r":{"name":"a.b","args":{"x":1}},"n":2}"#
    );
    assert_eq!(outcome.logs, [r#"got {"x":1} 2"#]);
    assert_eq!(eval("1 + 1").result, "null");
}

#[test]
fn host_errors_throw_their_json_and_keep_the_fix() {
    let caught = eval("try { __host('fail', '{}') } catch (e) { return e.fix }");
    assert_eq!(caught.result, r#""Do it right.""#);
    let thrown = eval("\n\n__host('fail', '{}')");
    let error = thrown.error.unwrap();
    assert_eq!(error.kind, Kind::Exception);
    assert_eq!(error.data, r#"{"code":"BAD","fix":"Do it right."}"#);
}

#[test]
fn error_lines_map_to_the_user_source() {
    let outcome = eval("const a = 1;\n\nfunction f() {\n  return a.b.c;\n}\nreturn f();");
    let error = outcome.error.unwrap();
    assert_eq!(error.kind, Kind::Exception);
    assert_eq!(error.line, Some(4), "{}", error.message);
    let syntax = eval("let ok = 1;\nlet = = 2;").error.unwrap();
    assert!(
        syntax.message.starts_with("SyntaxError"),
        "{}",
        syntax.message
    );
    assert_eq!(syntax.line, Some(2));
    assert_eq!(eval("throw new Error('x')").error.unwrap().line, Some(1));
}

#[test]
fn infinite_loop_times_out_uncatchably() {
    let started = Instant::now();
    let outcome = eval("for (;;) { try { while (true) {} } catch (e) {} }");
    assert_eq!(kind(&outcome), Some(Kind::Timeout));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn memory_bomb_hits_the_heap_limit() {
    let outcome = eval("const a = []; for (;;) a.push('x'.repeat(1 << 16) + a.length);");
    assert_eq!(kind(&outcome), Some(Kind::Memory), "{outcome:?}");
}

#[test]
fn deep_recursion_hits_the_stack_limit() {
    let outcome = eval("function f(n) { return f(n + 1) + 1; }\nreturn f(0);");
    let error = outcome.error.unwrap();
    assert_eq!(error.kind, Kind::Stack, "{}", error.message);
    assert_eq!(error.line, Some(1));
}

#[test]
fn call_source_and_output_limits() {
    let outcome = eval("for (let i = 0; i < 10; i++) { try { __host('a', '{}') } catch (e) {} }");
    assert_eq!(kind(&outcome), Some(Kind::Calls));
    let big = "x".repeat(65 << 10);
    assert_eq!(kind(&eval(&big)), Some(Kind::Source));
    let outcome = eval("console.log('y'.repeat(100)); console.log('z'); return 'x'.repeat(100);");
    assert_eq!(kind(&outcome), Some(Kind::Output));
    assert_eq!(outcome.logs.len(), 1);
    assert!(outcome.logs[0].ends_with("(logs truncated at 64 bytes)"));
}

#[test]
fn clock_and_random_are_deterministic() {
    let source = "return [Date.now(), new Date().getTime(), Math.random(), typeof performance];";
    let first = eval(source);
    assert_eq!(first.error, None);
    assert_eq!(first, eval(source));
    assert!(first.result.starts_with("[1767225600000,1767225600000,0."));
    assert!(first.result.ends_with(r#""undefined"]"#));
}
