# Splice development guide

Splice is an agent-first video editor for macOS. A headless Rust core owns the project, and every
capability is an MCP tool first. The UI is a client of the same core (M1: a viewer window with a
play bar). The living spec is the **main spec**: https://artifacts.microapps.space/a/cgzmrdd19w24
(prototype + design). Its JavaScript prototype is the behavioral reference for the core:
`docs/reference/core.js` (registry, apply, Store) and `docs/reference/agent.js` (MCP tools).

Current milestone and task board: [PLAN.md](PLAN.md). File format: [docs/FORMAT.md](docs/FORMAT.md).
MCP contract: [docs/MCP.md](docs/MCP.md). Command/undo kit: [docs/KIT.md](docs/KIT.md).
Build loop, self-improvement and Mac testing: [docs/LOOP.md](docs/LOOP.md).

## Layout

- `crates/splice_model`, `splice_json`, `splice_core`, `splice_eval`, `splice_mcp`, `splice_cli`,
  `splice_testkit`, `splice_kit`: portable, build and test on Linux.
- `crates/apple_sys` (LocalWave's `lw_apple_sys` pattern: ARC Objective-C shims + one C header + safe
  Rust modules; ScreenCaptureKit, AVFoundation, events, permissions), `splice_media`, `splice_av`,
  `splice_capture`, `splice_app`: macOS only. Their contents sit behind
  `#![cfg(target_os = "macos")]` so `cargo check --workspace` stays green on Linux.
- `pxui/`: PXUI submodule pinned on its `rust-port` branch. Apps use `pxui` (public) and the open tier
  (`pxui_host`, `mem_sys`, `file_access_sys`, …). Never name `pxui/crates/private`.
- `tools/gate`: the policy gate. `packaging/`: Info.plist, entitlements, bundle and sign scripts.
- `fixtures/`: projects, MCP transcripts, goldens. `evals/`: agent tasks.

## Machines

- **Linux box (here)**: everything portable: model, format, core, MCP, CLI, kit, gate, evals. This is
  where the loop runs.
- **Mac mini** `ssh snowbear@olegs-mac-mini` (macOS 27, arm64, Xcode, rust 1.99.1, Developer ID
  team 7ZH3FYV964): media, viewer, bundle, sign, install. Drive it from Linux with `make mac-check`,
  `make mac-run CMD='…'` and `make mac-pull` (docs/LOOP.md). Never edit files on the Mac; the
  Linux tree is the source and `make mac-sync` overwrites `~/projects/editor` there.

## Fast iteration (rays rules)

- Read the flow and callers first. Smallest correct change; delete obsolete code; no new abstractions.
- Refactor freely when it makes the code simpler or faster. Hot paths stay fast and allocation-free,
  and any change to them is measured.
- Inner loop `cargo check -p <crate>`; focused tests `cargo t -p <crate> [filter]`. **Never run raw
  `cargo test` or test binaries**: `cargo t` kills a case at 5 s and a run at 60 s. Never raise a
  deadline or weaken an assertion to go green.
- Warm shared `target/`; no `cargo clean`, private target dirs or RUSTFLAGS overrides.
- `make check` before a commit (Linux); `make mac-check` before a commit that touches mac crates.
- Leave one small runnable regression for nontrivial logic. Report commands, wall times, slowest cases.

## Contracts

- **One writer.** Everything that changes a project is `apply(ops)` in `splice_core`. CLI, MCP and the
  UI only build ops and read snapshots. The UI returns intents; it never mutates during a frame.
- **Agent-first, no approvals, no drafts.** Agent batches commit live. Safety comes from history:
  every batch is one undo step, `history.undo` reverts the last batch, a whole session, or back to any
  revision, and undo itself is a new revision (nothing is ever destroyed). The journal is fsynced.
- **Every capability is an MCP tool or an `edit.apply` command** before it is a UI control.
- Errors are results with a code and a fix hint. A failing agent call is a product bug: it is logged
  to the friction log and the loop fixes the cause (docs/LOOP.md).
- Integer frames, stable prefixed IDs (`clp_`, `ovl_`, `lan_`, `mrk_`, `med_`, `ses_`, `job_`, `w`).
- Zero allocations in steady state (warm apply of edit commands, per-frame eval, playback). Prove it
  with `splice_testkit::alloc` checks.
- Native media only (AVFoundation, VideoToolbox, CoreMedia, CoreVideo, Metal, ScreenCaptureKit,
  Speech). No ffmpeg or similar dependencies.
- Dependencies: std + cc (build only). Additions need measured evidence and a gate allowlist entry.
- `unsafe` only in `*_sys` crates. No async, `macro_rules!`, proc macros, `impl Trait` or generic
  items in product crates. Derives: Clone, Copy, Debug, PartialEq, Eq, Hash, Default.
- `splice_kit` depends on std (and `pxui_types` if needed) only: it is PXUI-shaped code that moves into
  PXUI once PXUI is mature (docs/KIT.md). Anything generic the app needs and PXUI lacks goes there.

Mark deliberate shortcuts with `ponytail:` and an upgrade trigger. Never shortcut correctness, input
validation, lifetime safety, security or accessibility.
