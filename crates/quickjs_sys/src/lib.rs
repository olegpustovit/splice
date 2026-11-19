//! Runs untrusted JavaScript in a fresh, bounded QuickJS-NG runtime.
//!
//! There is no quickjs-libc: scripts get no modules, files, network, timers or clock. The
//! only ways out are `__host(name, argsJson)`, which calls the host and returns its parsed
//! JSON (or throws it), and `console.log`, which appends to the bounded logs.
#![allow(unsafe_code)]
#![deny(missing_docs)]

use std::any::Any;
use std::ffi::{c_int, c_void};
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// Bounds for one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Heap bytes the runtime may allocate.
    pub memory: usize,
    /// Native stack bytes the interpreter may use.
    pub stack: usize,
    /// Wall time in JavaScript, excluding time spent in host calls.
    pub wall: Duration,
    /// Host calls per run.
    pub calls: u32,
    /// Source bytes.
    pub source: usize,
    /// Bytes of logs, and separately of the result JSON.
    pub output: usize,
}

/// Why a run failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The script threw (including syntax errors).
    Exception,
    /// The wall-time budget ran out.
    Timeout,
    /// The heap limit was reached.
    Memory,
    /// The stack limit was reached.
    Stack,
    /// The host-call budget ran out.
    Calls,
    /// The source exceeds its limit.
    Source,
    /// The result JSON exceeds the output limit.
    Output,
    /// The runtime could not be created.
    Runtime,
}

/// A failed run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// The failure class.
    pub kind: Kind,
    /// The thrown value as text, e.g. `TypeError: x is not a function`.
    pub message: String,
    /// The innermost line of the user's source in the stack, 1-based.
    pub line: Option<u32>,
    /// The thrown value as JSON (`{}` for plain errors, own fields for others).
    pub data: String,
}

/// The result of one run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The returned value as JSON (`null` for `undefined` or on failure).
    pub result: String,
    /// `console.log` lines, truncated at the output limit.
    pub logs: Vec<String>,
    /// Set when the run failed.
    pub error: Option<Failure>,
}

/// Fixed clock and seeded random so runs replay identically; console writes to the logs.
const BUILTINS: &str = r#"(() => {
  const g = globalThis, log = g.__log, D = g.Date, T = 1767225600000;
  delete g.__log;
  const show = (v) => {
    if (typeof v === "string") return v;
    if (v instanceof Error) return String(v);
    try { const s = JSON.stringify(v); return s === undefined ? String(v) : s; } catch (e) { return String(v); }
  };
  const write = (...a) => log(a.map(show).join(" "));
  g.console = { log: write, info: write, warn: write, error: write, debug: write };
  D.now = () => T;
  g.Date = new Proxy(D, {
    construct: (t, a, n) => Reflect.construct(t, a.length ? a : [T], n),
    apply: () => new D(T).toString(),
  });
  let s = 0x2545f491;
  Math.random = () => { s ^= s << 13; s ^= s >>> 17; s ^= s << 5; return (s >>> 0) / 4294967296; };
})();
"#;

#[repr(C)]
struct Host {
    opaque: *mut c_void,
    call: extern "C" fn(
        *mut c_void,
        *const u8,
        usize,
        *const u8,
        usize,
        *mut *const u8,
        *mut usize,
    ) -> c_int,
    log: extern "C" fn(*mut c_void, *const u8, usize),
    interrupt: extern "C" fn(*mut c_void) -> c_int,
    finish: extern "C" fn(*mut c_void, c_int, *const u8, usize, *const u8, usize, *const u8, usize),
}

unsafe extern "C" {
    fn splice_qjs_run(
        prelude: *const u8,
        prelude_len: usize,
        source: *const u8,
        source_len: usize,
        memory: usize,
        stack: usize,
        host: *const Host,
    ) -> c_int;
}

struct State {
    /// Points at the caller's `&mut dyn FnMut` for the duration of `run`.
    host: *mut c_void,
    limits: Limits,
    started: Instant,
    in_host: Duration,
    timed_out: bool,
    calls: u32,
    calls_exceeded: bool,
    reply: Vec<u8>,
    logs: Vec<String>,
    log_bytes: usize,
    finished: Option<(c_int, String, String, String)>,
    panic: Option<Box<dyn Any + Send>>,
}

/// Runs `prelude` (trusted setup, global scope), then `source` as a function body, so a
/// top-level `return` sets the result. `host(name, argsJson)` returns the result JSON, or
/// an error JSON that is thrown into the script as a plain value.
pub fn run(
    prelude: &str,
    source: &str,
    limits: &Limits,
    host: &mut dyn FnMut(&str, &str) -> Result<String, String>,
) -> Outcome {
    if source.len() > limits.source {
        return failed(
            Kind::Source,
            format!(
                "The code is {} bytes; the limit is {}.",
                source.len(),
                limits.source
            ),
        );
    }
    let mut setup = String::with_capacity(BUILTINS.len() + prelude.len() + 2);
    setup.push_str(BUILTINS);
    setup.push_str(prelude);
    setup.push('\0');
    // Same line numbering as the user's source: the wrapper adds no lines before it.
    let mut wrapped = String::with_capacity(source.len() + 20);
    wrapped.push_str("(function () {");
    wrapped.push_str(source);
    wrapped.push_str("\n})()\0");
    let mut host = host;
    let mut state = State {
        host: (&mut host as *mut &mut dyn FnMut(&str, &str) -> Result<String, String>).cast(),
        limits: *limits,
        started: Instant::now(),
        in_host: Duration::ZERO,
        timed_out: false,
        calls: 0,
        calls_exceeded: false,
        reply: Vec::new(),
        logs: Vec::new(),
        log_bytes: 0,
        finished: None,
        panic: None,
    };
    let vtable = Host {
        opaque: (&mut state as *mut State).cast(),
        call: on_call,
        log: on_log,
        interrupt: on_interrupt,
        finish: on_finish,
    };
    // SAFETY: both sources are NUL-terminated and outlive the call; `vtable` and `state`
    // live on this frame for the whole run and the shim does not keep them afterwards.
    let status = unsafe {
        splice_qjs_run(
            setup.as_ptr(),
            setup.len() - 1,
            wrapped.as_ptr(),
            wrapped.len() - 1,
            limits.memory,
            limits.stack,
            &vtable,
        )
    };
    if let Some(payload) = state.panic.take() {
        panic::resume_unwind(payload);
    }
    let logs = std::mem::take(&mut state.logs);
    let Some((code, json, message, stack)) = state.finished.take().filter(|_| status == 0) else {
        return failed(
            Kind::Runtime,
            "The JavaScript runtime could not start.".into(),
        );
    };
    if code == 0 {
        if json.len() > limits.output {
            let mut outcome = failed(
                Kind::Output,
                format!(
                    "The result is {} bytes of JSON; the limit is {}.",
                    json.len(),
                    limits.output
                ),
            );
            outcome.logs = logs;
            return outcome;
        }
        return Outcome {
            result: json,
            logs,
            error: None,
        };
    }
    let kind = if state.timed_out {
        Kind::Timeout
    } else if state.calls_exceeded && message.contains("call limit") {
        Kind::Calls
    } else if message.contains("out of memory") {
        Kind::Memory
    } else if message.contains("call stack size exceeded") || message.contains("stack overflow") {
        Kind::Stack
    } else {
        Kind::Exception
    };
    Outcome {
        result: "null".into(),
        logs,
        error: Some(Failure {
            kind,
            message,
            line: user_line(&stack),
            data: json,
        }),
    }
}

fn failed(kind: Kind, message: String) -> Outcome {
    Outcome {
        result: "null".into(),
        logs: Vec::new(),
        error: Some(Failure {
            kind,
            message,
            line: None,
            data: "null".into(),
        }),
    }
}

/// The first `code.js:LINE` frame is the innermost frame in the user's source.
fn user_line(stack: &str) -> Option<u32> {
    let (_, rest) = stack.split_once("code.js:")?;
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..digits].parse().ok()
}

fn text(ptr: *const u8, len: usize) -> String {
    if len == 0 {
        return String::new();
    }
    // SAFETY: the shim passes a live QuickJS string of `len` bytes for this call only.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(ptr, len) }).into_owned()
}

fn state(opaque: *mut c_void) -> &'static mut State {
    // SAFETY: `opaque` is the `State` on `run`'s frame; callbacks happen only during
    // `splice_qjs_run` on this thread, and never re-enter one another.
    unsafe { &mut *opaque.cast::<State>() }
}

extern "C" fn on_call(
    opaque: *mut c_void,
    name: *const u8,
    name_len: usize,
    args: *const u8,
    args_len: usize,
    out: *mut *const u8,
    out_len: *mut usize,
) -> c_int {
    let state = state(opaque);
    let (status, reply) = if state.panic.is_some() {
        (2, "The host failed.".to_string())
    } else if state.calls >= state.limits.calls {
        state.calls_exceeded = true;
        (
            2,
            format!("Reached the call limit of {} per run.", state.limits.calls),
        )
    } else {
        state.calls += 1;
        let (name, args) = (text(name, name_len), text(args, args_len));
        // SAFETY: `host` points at `run`'s `&mut dyn FnMut`, borrowed for the whole run.
        let host = unsafe {
            &mut *state
                .host
                .cast::<&mut dyn FnMut(&str, &str) -> Result<String, String>>()
        };
        let began = Instant::now();
        let result = panic::catch_unwind(AssertUnwindSafe(|| host(&name, &args)));
        state.in_host += began.elapsed();
        match result {
            Ok(Ok(json)) => (0, json),
            Ok(Err(json)) => (1, json),
            Err(payload) => {
                state.panic = Some(payload);
                (2, "The host failed.".to_string())
            }
        }
    };
    state.reply = reply.into_bytes();
    let len = state.reply.len();
    state.reply.push(0);
    // SAFETY: the shim passes valid out-pointers; `reply` lives until the next call.
    unsafe {
        *out = state.reply.as_ptr();
        *out_len = len;
    }
    status
}

extern "C" fn on_log(opaque: *mut c_void, line: *const u8, len: usize) {
    let state = state(opaque);
    let limit = state.limits.output;
    if state.log_bytes >= limit {
        return;
    }
    let mut line = text(line, len);
    state.log_bytes += line.len() + 1;
    if state.log_bytes > limit {
        let mut keep = line.len().saturating_sub(state.log_bytes - limit);
        while !line.is_char_boundary(keep) {
            keep -= 1;
        }
        line.truncate(keep);
        line.push_str(&format!("… (logs truncated at {limit} bytes)"));
    }
    state.logs.push(line);
}

extern "C" fn on_interrupt(opaque: *mut c_void) -> c_int {
    let state = state(opaque);
    if !state.timed_out && state.started.elapsed().saturating_sub(state.in_host) > state.limits.wall
    {
        state.timed_out = true;
    }
    c_int::from(state.timed_out || state.panic.is_some())
}

#[allow(clippy::too_many_arguments)]
extern "C" fn on_finish(
    opaque: *mut c_void,
    status: c_int,
    json: *const u8,
    json_len: usize,
    message: *const u8,
    message_len: usize,
    stack: *const u8,
    stack_len: usize,
) {
    state(opaque).finished = Some((
        status,
        text(json, json_len),
        text(message, message_len),
        text(stack, stack_len),
    ));
}

#[cfg(test)]
mod tests;
