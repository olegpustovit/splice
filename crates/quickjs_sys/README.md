# QuickJS-NG bridge

The sandbox behind `code.run` (PLAN.md D2, T14). `vendor/` holds the QuickJS-NG engine sources,
unmodified; `native/shim.c` is the only C that touches its API; `src/lib.rs` is the safe Rust
`run(prelude, source, limits, host)`.

- Version: QuickJS-NG **v0.18.0** (released 2026-11-01), from
  `https://github.com/quickjs-ng/quickjs/archive/refs/tags/v0.18.0.tar.gz`,
  sha256 `28bcd89aa3683fd2a946537e5ce3f11cc46b5cbf3383c7561aaef3df5ff9f1dd`.
- Vendored: `quickjs.c`, `libregexp.c`, `libunicode.c`, `dtoa.c` and the headers they include.
  Not vendored or compiled: `quickjs-libc` (std/os modules, files, sockets, timers), `qjs`,
  `qjsc`, tests. License: MIT, `vendor/LICENSE`.
- Built by `cc` (`build.rs`, `-O2`, no debug info). QuickJS asserts stay on outside release.

Each run gets a fresh runtime and context with base objects, Date, eval, RegExp, JSON, Proxy,
Map/Set, typed arrays, Promise and BigInt; no `performance` and no WeakRef. A heap limit, a
native stack limit and an interrupt handler (wall time minus time spent in host calls) bound it.
`Date.now()`/`new Date()` are fixed at 2026-01-01T00:00:00Z and `Math.random` is a seeded
xorshift, so evals replay. The script reaches the host only through `__host(name, argsJson)`
and `console.*`.

To upgrade: download the release tarball, check its sha256, copy the files listed above over
`vendor/`, update this README and run `cargo t -p quickjs_sys -p splice_mcp`.

## Evidence for the dependency (AGENTS.md)

Measured on the Linux box (20 threads, rust 1.99.1, gcc), 2026-11-19:

| | before (c1ad8ea) | with quickjs_sys |
| --- | --- | --- |
| `target/release/splice` | 10,506,328 B | 12,213,824 B (+1.71 MB) |
| stripped | 1,816,864 B | 3,040,136 B (+1.22 MB) |

Clean build of `quickjs_sys` (cc crate plus the C sources, fresh target dir): 12.9 s release,
13.7 s dev. `quickjs.c` is one translation unit and dominates; it rebuilds only when `vendor/` or
`native/` change.
