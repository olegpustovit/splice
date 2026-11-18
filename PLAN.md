# M1 · Splice 0.1: the headless core

**Hard milestone.** Cut 0.1 lands **today, 2026-11-18**. The loop keeps going after the cut until the
M1 exit (0.3). Scope is fixed. When a task does not fit, ship it as a `ponytail:` with an upgrade
trigger instead of cutting the task; when that is not possible, move the task to a later cut and
say so in the log. Never weaken a test to make a cut.

## What 0.1 is

A Rust core that does **all** editing, exposed 1:1 over MCP and the CLI, plus the thinnest UI:

| Surface | 0.1 behavior |
| --- | --- |
| `make dev` | Debug build (fast, incremental) → bundled, signed `Splice.app` → `~/.local/bin/splice` |
| `make install` | Same with the release profile. Both share one bundle ID and signer, so TCC grants persist |
| `splice` | Opens the viewer with the native macOS open panel (`pxui_sdl::Window::show_dialog`) to pick a `.splice` project |
| `splice <path>` / `splice open <path>` | Opens the viewer on that project directly; `splice new <path>` creates one |
| `splice mcp` | MCP over stdio. **Never opens UI.** Editing, export and (0.2) recording all work headless. If a viewer has the project open, it forwards to it over the socket so the human sees edits live |
| `splice apply ops.json` · `snapshot` · `describe` · `export` · `frame` · `friction` · `check` | Headless CLI over the same core |
| Viewer | One window: the program frame letterboxed to the project aspect, a play bar (play/pause, scrubber, timecode). Space plays/pauses, ←/→ steps a frame, ⌘Z/⇧⌘Z undo/redo. Nothing else |
| Export | AVFoundation, no approval, writes `<project>.splice/exports/<name>-r<rev>.mp4`, never overwrites unless `overwrite:true` |

**Out of 0.1:** timeline UI, inspector, menus, palette, effects UI, the Metal compositor (M3), and any
PXUI change. Whatever is generic and missing from PXUI goes to `splice_kit` (docs/KIT.md).

## Machines

Build the core on the Linux box. Reach for the Mac only for `[mac]` tasks:
`ssh snowbear@olegs-mac-mini`, driven by `make mac-sync | mac-check | mac-run | mac-pull`
(docs/LOOP.md). Tag legend: **[L]** Linux, **[M]** Mac over ssh.

## Cut lines

- **0.1 · today:** T0, T1, T2 (Tier A commands), T3, T4, T5.1–T5.5, T6.1–T6.2, T7.1–T7.5.
- **0.2:** T2.7 (Tier B commands), T3.6 evals, T6.3, T8 recording, T9 captions with transcription.
- **0.3 · M1 exit:** every registry row built (the 20 planned rows too), T7.6 notarization,
  performance budgets met, the friction backlog empty or triaged.

## Board

The loop takes the first unchecked task whose dependencies are done and whose machine is reachable.
Every task has an acceptance check. A task is done when that check passes under `cargo t` (or the named
make target), `make check` is green, and the task is committed. Mark it `[x]` with the commit's short
SHA.

### T0 · Foundations
- [x] T0.1 [L] Workspace scaffold, toolchain 1.99.1, PXUI submodule (`rust-port` @ 0baa83c), `cargo t` alias.
- [ ] T0.2 [L] Makefile (written; Linux `make check` green; `mac-check` not yet run on the Mini): `check`, `dev`, `install`, `bundle`, `sign`, `mac-sync`, `mac-check`, `mac-run`, `mac-pull`, `eval`, `loop`. Accept: `make check` green on Linux; `make mac-check` green on the Mini (empty mac crates).
- [ ] T0.3 [L] `tools/gate`: shape rules (see AGENTS.md), per-crate dependency allowlist, `splice_kit` std-only, mac crates cfg-gated, no `#[ignore]`, no HashMap iteration in output. Accept: `cargo run -p gate -- all` green; gate's own tests plant one violation per rule and see it refused.
- [ ] T0.4 [L] `splice_testkit`: `alloc::warm(|| …)` asserting zero allocations (mem_sys `counting` as a dev-dependency only; add `[profile.dev.package.mem_sys] opt-level = 3` with it), goldens (`SPLICE_BLESS=1` refreshes), seeded xorshift, MCP transcript runner stub. Accept: self-tests, including one that must detect an allocation.

### T1 · Model and file format (docs/FORMAT.md)
- [ ] T1.1 [L] `splice_model`: arenas (SoA, u32 slots), typed IDs that print as `clp_2x`, `Frame = i32`, project, spine, lanes, clips, items, markers, effects, media table, words. Accept: unit tests; capacity reserve means no growth during warm edits.
- [ ] T1.2 [L] `splice_json`: borrowing tokenizer, canonical writer (fixed key order, shortest round-trip floats, in-house), errors with byte offset and path. Accept: round-trip of every fixture is byte-identical; a warm write to a reused buffer allocates nothing; malformed-input table test.
- [ ] T1.3 [L] `.splice` package: read/write `project.json` and `media.json`, atomic writes (tmp + fsync + rename), `lock` (flock, pid, host), format version checks. Port the prototype's sample project as `fixtures/projects/sample.splice`. Accept: load → save → load gives an identical hash; a newer major is refused with `FORMAT_TOO_NEW`; a second writer gets `LOCKED`.
- [ ] T1.4 [L] Journal: append one batch per line, fsync per commit, hash chain, snapshots every 64 batches, recovery (snapshot + replay, truncate a torn tail). Accept: a crash simulated at **every byte offset** of a 3-batch journal recovers to the last complete batch with the right hash.

### T2 · Core engine
- [ ] T2.1 [L] Registry: `COMMANDS` table rows for all 61 commands (41 built in the prototype, 20 planned). `Op` enum, one decoder per command, `commands.describe` from the table; planned rows return `PLANNED`. Accept: the parity test (each row has a decoder, a variant, an example that decodes, a doc).
- [ ] T2.2 [L] `apply`: atomic batches, `$ref`s, deterministic ID minting (replay reuses recorded mints), anchor resolution, settle, changes, sideEffects, diff, `dryRun`, invariant checker after every apply in tests. Accept: prototype parity fixtures (`docs/reference/core.js` behavior) for refs, minting and cascade deletes.
- [ ] T2.3 [L] Tier A commands: `clip.insert`, `item.split`, `item.trim`, `item.move`, `item.delete`, `item.set`, `item.duplicate`, `clip.speed`, `lane.create`, `lane.delete`, `lane.rename`, `item.lane`, `item.anchor`, `overlay.add`, `overlay.set`, `title.add`, `title.set`, `marker.add`, `marker.set`, `project.set`. Accept, per command: a decoder test, an apply test, a golden snapshot, and the example from `commands.describe` applying cleanly to the sample.
- [ ] T2.4 [L] History: revisions, human undo/redo, sessions, `history.undo {scope:"last"|"session"|"to", revision}`, revert (rebuild + replay with a `skipped` list), `ifRevision` → `STALE_REVISION`, `idempotencyKey`. Accept: a seeded property test (500 random batches mixing human and agent edits) shows undo to every revision r reproduces hash(r), journal replay equals live state, undoing an undo restores, and the skipped lists are deterministic.
- [ ] T2.5 [L] `splice_eval`: timeline → per-frame render plan (layers, transforms, fades, audio gains), effect resolution project › lane › item. Accept: plan goldens at 10 frames of the sample; a warm per-frame eval allocates nothing.
- [ ] T2.6 [L] Budgets: apply p99 < 1 ms on a 2,000-item project; warm `item.trim`/`move`/`set` allocate nothing. Accept: `cargo t -p splice_core budget` (release-timed in qualification with `SPLICE_SLOW=1`).
- [ ] T2.7 [L] Tier B commands (0.2): effects (`effect.add/set/remove/paste/promote`), components (`component.set`, `audio.set`, `cursor.set`, `keys.set`, `keys.hide`), cutting (`cut.range`, `cut.words`), `clip.crop`, `motion.add/set`, `clip.lift`, `overlay.replaceSpine`, captions (`captions.generate`, `caption.set/split/merge`). Then the 20 planned rows (0.3).

### T3 · MCP and CLI, headless
- [ ] T3.1 [L] JSON-RPC 2.0 stdio, newline-delimited, hand-rolled. `initialize` negotiates 2025-11-25 and 2026-07-28. `tools/list` comes from the table in a fixed order with `inputSchema` and `outputSchema`. Results are `structuredContent` plus a text mirror; business errors are `isError` results with `code` and `fix`. Accept: a conformance transcript and a `tools/list` golden.
- [ ] T3.2 [L] The 19 tools (docs/MCP.md) wired to the core. `project.info {path, create}` opens a project headless; `ui.show` returns `{applied:false, reason:"headless"}` with no UI. Accept: one transcript per tool in `fixtures/mcp/`.
- [ ] T3.3 [L] Friction log: every `isError` result, the next calls and whether they recovered, and the optional `friction` argument every tool accepts → `<project>.splice/friction.jsonl` plus the global state dir. `splice friction [list|show|repro <id>|resolve <id> <sha>]`. Accept: transcript tests for logging, the recovery flag, and `repro` writing a failing transcript fixture.
- [ ] T3.4 [L] Transcript runner: `fixtures/mcp/*.jsonl` replays request → expected structured result with `"…"` wildcards. Accept: every T3.2 fixture runs under `cargo t -p splice_mcp transcript`.
- [ ] T3.5 [L] `splice` CLI: `new`, `open`, `mcp`, `apply`, `snapshot`, `describe`, `export`, `frame`, `friction`, `check`, `version`, and `splice <path>` = open. On Linux, `open`, `export` and `frame` return the typed error `UNSUPPORTED_PLATFORM`. Accept: CLI tests over a temp project.
- [ ] T3.6 [L] (0.2) `.mcp.json` dogfood and `make eval`: `evals/tasks/*.toml` (prompt, seed project, end-state assertions checked by `splice check`) run `claude -p` with the `splice` skill against `splice mcp`. Results go to `evals/results.tsv` (pass, calls, errors, recoveries). Accept: 5 tasks; a baseline recorded.

### T4 · Media on macOS (no ffmpeg)
- [ ] T4.1 [M] Sys crates: `objc_sys` (msgSend, autorelease pools, NSString, NSURL, minimal blocks), `av_sys`, `cm_sys`, `cv_sys`: hand-written, the only `unsafe`. Accept: probe of a fixture movie on the Mini.
- [ ] T4.2 [M] `splice_media`: import + probe (fps, duration, size, tracks, channels) into `media.json` with a bookmark; the core receives descriptors only. Accept: `splice apply` with `clip.insert {path}` on the Mini; on Linux the same transcript uses fixture descriptors.
- [ ] T4.3 [M] `splice_av`: lower the eval plan to `AVMutableComposition` + `AVMutableVideoComposition` (cuts, speed via scaleTimeRange, overlay layers with transform/opacity/fades, audio mix volumes). Preview and export use this one lowering. `ponytail:` titles and captions use the CoreAnimation tool in export and PXUI text in preview; upgrade trigger: the M3 Metal compositor, or any visible mismatch reported in friction.
- [ ] T4.4 [M] Export job: H.264 and HEVC presets, progress via `jobs.status`, cancellation, no approval, the default path rules above. Accept: exporting the sample yields an mp4 whose probed duration and frame count match the timeline.
- [ ] T4.5 [M] `view.frames` and `splice frame --at N out.png`: frame grabs from the same composition, so agents see what export makes. Accept: the frame at N matches the exported frame at N (mean abs diff ≤ 2/255).

### T5 · Viewer (minimal UI) [M]
- [ ] T5.1 `splice open`: a `pxui_host` window; with no path, `show_dialog(OpenFile)` filtered to `.splice` (the package UTI from T7.1); remembers the last project. Accept: `splice --smoke-test` opens and closes a window headlessly on the Mini.
- [ ] T5.2 Program frame: `AVPlayerItemVideoOutput` pixel buffers → RGBA → `pxui::images`, letterboxed to the project aspect. `ponytail:` one copy per frame; upgrade trigger: 4K preview over 8 ms per frame → external texture via `splice_kit`/PXUI.
- [ ] T5.3 Play bar: play/pause, scrubber (gesture phases from T6), timecode; Space, ←/→, ⌘Z/⇧⌘Z through `splice_kit` commands → core history. Accept: kit command tests + `--snapshot` golden.
- [ ] T5.4 Live: while a viewer has the project open it is the single writer and serves MCP on a unix socket (getpeereid, same user). `splice mcp` forwards to it; otherwise it runs headless with the project lock. On a revision change the composition rebuilds and the playhead is kept. Accept: an agent edit through `splice mcp` shows in the viewer's `--snapshot`, and ⌘Z in the viewer reverts it.
- [ ] T5.5 `splice open <p> --snapshot out.png [--at N]`: offscreen render of the window for ssh verification; `make mac-pull` copies it back.

### T6 · `splice_kit`: commands and undo, PXUI-shaped (docs/KIT.md)
- [ ] T6.1 [L] Command table, responder chain resolution, validation (enabled, checked, title), `Invoked {id, source, target}`, keymap file with overrides and a conflict test. Accept: unit tests; zero allocations on warm resolution.
- [ ] T6.2 [L] Undo manager: preallocated ring, `Merge::{Step, Gesture(u64), Burst{key, window}, Repair}`, `seal`, count/byte caps, payload `u64` (= core revision), titles from the core. Accept: merge-policy table tests ported from rays `editor_history`.
- [ ] T6.3 [L] (0.2) Gesture phases (`Began/Changed/Ended/Cancelled`) for continuous controls; one gesture = one undo step.

### T7 · Install, locate, sign (LocalWave model) [M]
- [ ] T7.1 `packaging/Info.plist`: `com.olegpustovit.splice`, `LSMinimumSystemVersion 15.0`, microphone/camera usage strings, a `.splice` package UTI (`com.olegpustovit.splice.project`, conforms to `com.apple.package`), document types. `packaging/splice.entitlements`: hardened runtime, `device.audio-input`, `device.camera`, no sandbox (CLI and MCP read arbitrary paths).
- [ ] T7.2 `packaging/bundle-macos.sh` (from LocalWave `packaging/bundle-macos.sh`): binary + `libpxui.1.dylib` → `Splice.app/Contents/{MacOS,Frameworks}`; rpath `@executable_path/../Frameworks`; refuse unbundled dylibs; codesign with the first of Developer ID (team 7ZH3FYV964) › Apple Development › ad-hoc; hardened runtime only with a real identity. Over ssh, unlock the keychain first (`security unlock-keychain`; Recorder keeps a passwordless `recorder-signing.keychain-db`; do the same as `splice-signing`).
- [ ] T7.3 `make dev` (debug) and `make install` (release) → `$(APPDIR)/Splice.app` (default `~/Applications`) + `ln -sfn …/Contents/MacOS/splice ~/.local/bin/splice`, then `lsregister` so `.splice` packages open in Splice. Accept: both run on the Mini; `which splice` resolves; `splice version` prints the build.
- [ ] T7.4 Locate: the binary finds its bundle through `_NSGetExecutablePath` + realpath (safe through the symlink); resources come from `Contents/Resources`; state lives in `~/Library/Application Support/Splice` (Linux: `$XDG_STATE_HOME/splice`); the socket goes in the state dir. Accept: unit tests for path resolution; `splice --smoke-test` run through the symlink.
- [ ] T7.5 `tools/check-macos-bundle.sh` (from LocalWave): Info.plist keys, minos ≤ 15.0, rpaths, no unbundled dependencies, entitlements, `--smoke-test`. Run by `make mac-check`.
- [ ] T7.6 (0.3) Notarize (`tools/notarize-macos.sh`) and a `.dmg`.

### T8 · Recording through MCP, headless (0.2) [M]
- [ ] T8.1 `record.start {display|window, mic, systemAudio, camera, events}` → ScreenCaptureKit + AVAssetWriter into `<project>.splice/takes/<med>/` with Recorder's `events.json` (cursor, clicks; keys follow the privacy policy). `record.stop` finalizes and imports the take as a clip with components. No UI.
- [ ] T8.2 TCC attribution: `splice mcp` re-spawns the bundled binary with responsibility disclaimed, so Screen Recording and Microphone grants belong to Splice.app's signature, not to the terminal or MCP client. `ponytail:` private spawn attribute; upgrade trigger: it breaks → a LaunchServices helper (`open -g -j`, Recorder D6). Accept: on the Mini, an ssh-launched `splice mcp` records 3 s after a one-time grant.

### T9 · Text and captions (0.2) [M + L]
- [ ] T9.1 [M] `splice_speech`: SpeechAnalyzer transcription job → words with frame-accurate timing (forced alignment), transcript hash. Linux uses fixture transcripts.
- [ ] T9.2 [L] Caption commands and `transcript.get {captions: json|srt|vtt}` (from the prototype). Burn-in through T4.3.

## Log

Append one line per loop iteration: date, task, result, commit, wall time of the checks.

- 2026-11-18 · T0.1 · done · scaffold builds with 1.99.1; `cargo t` 23 s cold, 0 tests
