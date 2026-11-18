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
| `splice mcp` | MCP over stdio. **Never opens UI.** Editing, export and screen recording all work headless. If a viewer has the project open, it forwards to it over the socket so the human sees edits live |
| `splice apply ops.json` · `snapshot` · `describe` · `export` · `frame` · `friction` · `check` | Headless CLI over the same core |
| Viewer | One window: the program frame letterboxed to the project aspect, a play bar (play/pause, scrubber, timecode). Space plays/pauses, ←/→ steps a frame, ⌘Z/⇧⌘Z undo/redo. Nothing else |
| `splice record <new.splice> [--display main \| --window <app> \| --area …] [--duration s] [--mic] [--system-audio] [--camera]` | Records the screen into a new (or existing) project: the take lands as an editable clip with its components and events |
| `splice permissions [--request]` | Shows (or requests once) Screen Recording, Microphone, Camera and Input Monitoring for Splice.app, plus the signing identity and install path |
| Export | AVFoundation, no approval, writes `<project>.splice/exports/<name>-r<rev>.mp4`, never overwrites unless `overwrite:true` |

**Out of 0.1:** timeline UI, inspector, menus, palette, effects UI, the Metal compositor (M3), and any
PXUI change. Whatever is generic and missing from PXUI goes to `splice_kit` (docs/KIT.md).

## Machines

Build the core on the Linux box. Reach for the Mac only for `[mac]` tasks:
`ssh snowbear@olegs-mac-mini`, driven by `make mac-sync | mac-check | mac-run | mac-pull`
(docs/LOOP.md). Tag legend: **[L]** Linux, **[M]** Mac over ssh.

## Cut lines

- **0.1 · today:** T0, T1, T2 (Tier A commands), T3, T4, T5.1–T5.5, T6.1–T6.2, T7.1–T7.5, then T8
  (recording through MCP). Recording is secondary to editing but in 0.1, so agents can generate real
  projects to edit and to feed the self-improvement loop.
- **0.2:** T2.7 (Tier B commands), T3.6 evals, T6.3, T9 captions with transcription.
- **0.3 · M1 exit:** every registry row built (the 20 planned rows too), T7.6 notarization,
  performance budgets met, the friction backlog empty or triaged.

## Board

The loop takes the first unchecked task whose dependencies are done and whose machine is reachable.
Every task has an acceptance check. A task is done when that check passes under `cargo t` (or the named
make target), `make check` is green, and the task is committed. Mark it `[x]` with the commit's short
SHA.

### T0 · Foundations
- [x] T0.1 [L] Workspace scaffold, toolchain 1.99.1, PXUI submodule (`rust-port` @ 0baa83c), `cargo t` alias.
- [x] T0.2 [L] Makefile (e85a6d2): `check`, `dev`, `install`, `bundle`, `sign`, `mac-sync`, `mac-check`, `mac-run`, `mac-pull`, `eval`, `loop`. Accept: `make check` green on Linux; `make mac-check` green on the Mini (empty mac crates).
- [x] T0.3 [L] `tools/gate` (0e16998): shape rules (see AGENTS.md), per-crate dependency allowlist, `splice_kit` std-only, mac crates cfg-gated, no `#[ignore]`, no HashMap iteration in output. Accept: `cargo run -p gate -- all` green; gate's own tests plant one violation per rule and see it refused.
- [x] T0.4 [L] `splice_testkit` (748047c): `alloc::warm(|| …)` asserting zero allocations (mem_sys `counting` as a dev-dependency only; add `[profile.dev.package.mem_sys] opt-level = 3` with it), goldens (`SPLICE_BLESS=1` refreshes), seeded xorshift, MCP transcript runner stub. Accept: self-tests, including one that must detect an allocation.

### T1 · Model and file format (docs/FORMAT.md)
- [x] T1.1 [L] `splice_model` (edda50c): arenas (SoA, u32 slots), typed IDs that print as `clp_2x`, `Frame = i32`, project, spine, lanes, clips, items, markers, effects, media table, words. Accept: unit tests; capacity reserve means no growth during warm edits.
- [x] T1.2 [L] `splice_json` (2505b11): borrowing tokenizer, canonical writer (fixed key order, shortest round-trip floats, in-house), errors with byte offset and path. Accept: round-trip of every fixture is byte-identical; a warm write to a reused buffer allocates nothing; malformed-input table test.
- [x] T1.3 [L] `.splice` package (8efee3c): read/write `project.json` and `media.json`, atomic writes (tmp + fsync + rename), `lock` (flock, pid, host), format version checks. Port the prototype's sample project as `fixtures/projects/sample.splice`. Accept: load → save → load gives an identical hash; a newer major is refused with `FORMAT_TOO_NEW`; a second writer gets `LOCKED`.
- [x] T1.4 [L] Journal (8628ecf): append one batch per line, fsync per commit, hash chain, snapshots every 64 batches, recovery (snapshot + replay, truncate a torn tail). Accept: a crash simulated at **every byte offset** of a 3-batch journal recovers to the last complete batch with the right hash.

### T2 · Core engine
- [x] T2.1 [L] Registry (7bdcbc5): `COMMANDS` table rows for all 61 commands (41 built in the prototype, 20 planned). `Op` enum, one decoder per command, `commands.describe` from the table; planned rows return `PLANNED`. Accept: the parity test (each row has a decoder, a variant, an example that decodes, a doc).
- [x] T2.2 [L] `apply` (269824f): atomic batches, `$ref`s, deterministic ID minting (replay reuses recorded mints), anchor resolution, settle, changes, sideEffects, diff, `dryRun`, invariant checker after every apply in tests. Accept: prototype parity fixtures (`docs/reference/core.js` behavior) for refs, minting and cascade deletes.
- [x] T2.3 [L] Tier A commands (232b9f0): `clip.insert`, `item.split`, `item.trim`, `item.move`, `item.delete`, `item.set`, `item.duplicate`, `clip.speed`, `lane.create`, `lane.delete`, `lane.rename`, `item.lane`, `item.anchor`, `overlay.add`, `overlay.set`, `title.add`, `title.set`, `marker.add`, `marker.set`, `project.set`. Accept, per command: a decoder test, an apply test, a golden snapshot, and the example from `commands.describe` applying cleanly to the sample.
- [x] T2.4 [L] History (89ca2a7): revisions, human undo/redo, sessions, `history.undo {scope:"last"|"session"|"to", revision}`, revert (rebuild + replay with a `skipped` list), `ifRevision` → `STALE_REVISION`, `idempotencyKey`. Accept: a seeded property test (500 random batches mixing human and agent edits) shows undo to every revision r reproduces hash(r), journal replay equals live state, undoing an undo restores, and the skipped lists are deterministic.
- [x] T2.5 [L] `splice_eval` (809dcae): timeline → per-frame render plan (layers, transforms, fades, audio gains), effect resolution project › lane › item. Accept: plan goldens at 10 frames of the sample; a warm per-frame eval allocates nothing.
- [x] T2.6 [L] Budgets (ab85713): apply p99 < 1 ms on a 2,000-item project; warm `item.trim`/`move`/`set` allocate nothing. Accept: `cargo t -p splice_core budget` (release-timed in qualification with `SPLICE_SLOW=1`).
- [ ] T2.7 [L] Tier B commands (0.2): effects (`effect.add/set/remove/paste/promote`), components (`component.set`, `audio.set`, `cursor.set`, `keys.set`, `keys.hide`), cutting (`cut.range`, `cut.words`), `clip.crop`, `motion.add/set`, `clip.lift`, `overlay.replaceSpine`, captions (`captions.generate`, `caption.set/split/merge`). Then the 20 planned rows (0.3).

### T3 · MCP and CLI, headless
- [x] T3.1 [L] JSON-RPC 2.0 stdio (33e67ce), newline-delimited, hand-rolled. `initialize` negotiates 2025-11-25 and 2026-07-28. `tools/list` comes from the table in a fixed order with `inputSchema` and `outputSchema`. Results are `structuredContent` plus a text mirror; business errors are `isError` results with `code` and `fix`. Accept: a conformance transcript and a `tools/list` golden.
- [x] T3.2 [L] The 19 tools (docs/MCP.md) wired to the core (b2a72c1). `project.info {path, create}` opens a project headless; `ui.show` returns `{applied:false, reason:"headless"}` with no UI. Accept: one transcript per tool in `fixtures/mcp/`.
- [x] T3.3 [L] Friction log (99c005a): every `isError` result, the next calls and whether they recovered, and the optional `friction` argument every tool accepts → `<project>.splice/friction.jsonl` plus the global state dir. `splice friction [list|show|repro <id>|resolve <id> <sha>]`. Accept: transcript tests for logging, the recovery flag, and `repro` writing a failing transcript fixture.
- [x] T3.4 [L] Transcript runner (3ddd172): `fixtures/mcp/*.jsonl` replays request → expected structured result with `"…"` wildcards. Accept: every T3.2 fixture runs under `cargo t -p splice_mcp transcript`.
- [x] T3.5 [L] `splice` CLI (80e8c4f): `new`, `open`, `mcp`, `apply`, `snapshot`, `describe`, `export`, `frame`, `friction`, `check`, `version`, and `splice <path>` = open. On Linux, `open`, `export` and `frame` return the typed error `UNSUPPORTED_PLATFORM`. Accept: CLI tests over a temp project.
- [ ] T3.6 [L] (0.2) `.mcp.json` dogfood and `make eval`: `evals/tasks/*.toml` (prompt, seed project, end-state assertions checked by `splice check`) run `claude -p` with the `splice` skill against `splice mcp`. Results go to `evals/results.tsv` (pass, calls, errors, recoveries). Accept: 5 tasks; a baseline recorded.

### T4 · Media on macOS (no ffmpeg)
- [x] T4.1 [M] `apple_sys` (6ea33b9), in LocalWave's `lw_apple_sys` pattern: one crate, one ARC Objective-C shim per service (`media.m`, `capture.m`, `events.m`, `permissions.m`, `panel.m`, …) behind one C header `platform.h` with `_Static_assert` layout checks, built by `cc` in build.rs (macOS only), and a small safe Rust module per shim. Native → Rust traffic is a bounded event ring polled with `peek`/`ack` plus a `wake` callback; callbacks never hold Rust references; errors come back as `char **error`. This crate is the only `unsafe`. Accept: probe of a fixture movie on the Mini.
- [x] T4.2 [M] `splice_media` (db51464): import + probe (fps, duration, size, tracks, channels) into `media.json` with a bookmark; the core receives descriptors only. Accept: `splice apply` with `clip.insert {path}` on the Mini; on Linux the same transcript uses fixture descriptors.
- [x] T4.3 [M] `splice_av` (d20d276): lower the eval plan to `AVMutableComposition` + `AVMutableVideoComposition` (cuts, speed via scaleTimeRange, overlay layers with transform/opacity/fades, audio mix volumes). Preview and export use this one lowering. `ponytail:` titles and captions use the CoreAnimation tool in export and PXUI text in preview; upgrade trigger: the M3 Metal compositor, or any visible mismatch reported in friction.
- [x] T4.4 [M] Export job (7d00f36): H.264 and HEVC presets, progress via `jobs.status`, cancellation, no approval, the default path rules above. Accept: `tools/check-macos-export.sh` probes the sample MP4 for timeline duration and frame count.
- [x] T4.5 [M] `view.frames` and `splice frame --at N out.png` (b3125f2): frame grabs from the same composition, so agents see what export makes. Accept: `tools/check-macos-export.sh` compares the frame at N to both encodes at N (mean abs diff ≤ 2/255).

### T5 · Viewer (minimal UI) [M]
- [x] T5.1 `splice open` (62db338): a `pxui_host` window; with no path, `show_dialog(OpenFile)` filtered to `.splice` (the package UTI from T7.1); remembers the last project. Accept: `splice --smoke-test` opens and closes a window headlessly on the Mini.
- [x] T5.2 Program frame (df39fac): `AVPlayerItemVideoOutput` pixel buffers → RGBA → `pxui::images`, letterboxed to the project aspect. After the separate upload exceeded 8 ms at 4K, preview writes RGBA directly into three shared Metal textures sampled by PXUI.
- [x] T5.3 Play bar (0992642): play/pause, scrubber previews and cancellation, frame timecode, and Space, ←/→, ⌘Z/⇧⌘Z through `splice_kit` and core history. The offscreen golden uses the live viewer layout.
- [x] T5.4 Live (7c9a835): the viewer keeps the single writer and serves same-user MCP and CLI clients through a per-project Unix socket. A revision change rebuilds the composition and preserves the playhead. Native tests cover live snapshots, journal-backed Cmd-Z and redo, and headless fallback after the viewer closes.
- [x] T5.5 `splice open <p> --snapshot out.png [--at N]` (2eddf7e): offscreen render of the viewer at its default 960 × 640 size. The CLI reports the revision and frame, publishes a new PNG atomically, and `make mac-pull` copies it back.

### T6 · `splice_kit`: commands and undo, PXUI-shaped (docs/KIT.md)
- [x] T6.1 [L] Command table, responder chain resolution, validation (enabled, checked, title), `Invoked {id, source, target}`, keymap file with overrides and a conflict test (b84e72c). Unit tests verify zero allocations on warm resolution with default and user shortcuts.
- [x] T6.2 [L] Undo manager (4554bf1): preallocated ring, `Merge::{Step, Gesture(u64), Burst{key, window}, Repair}`, `seal`, count and byte caps, revision payloads, and owned or borrowed titles. The rays merge-policy tests pass and verify zero warm allocations across eviction, undo, and redo.
- [ ] T6.3 [L] (0.2) Gesture phases (`Began/Changed/Ended/Cancelled`) for continuous controls; one gesture = one undo step.

### T7 · Install, locate, sign (LocalWave model) [M]
- [x] T7.1 Bundle metadata (a670c56): `com.olegpustovit.splice`, macOS 15.0 minimum, camera and microphone usage strings, and the `.splice` package and editor declarations. The entitlements grant audio input and camera access for the Hardened Runtime signing in T7.2.
- [x] T7.2 Bundling and signing (3621e14): `Splice.app` embeds PXUI and the pinned SDL built for macOS 15.0. The bundler checks native dependencies, sets `@executable_path/../Frameworks`, and signs the app and its libraries with Hardened Runtime and team `7ZH3FYV964`. Both profiles use the passwordless `splice-signing.keychain-db` and require a valid Developer ID Application identity.
- [x] T7.3 Install targets and Finder launch (f05d175): `make dev` (debug) and `make install` (release) publish the same `$(APPDIR)/Splice.app` (default `~/Applications`), link its executable at `~/.local/bin/splice`, and register the app with Launch Services. Both profiles ran through the link on the Mini, and Launch Services delivered a `.splice` package to the installed app.
- [x] T7.4 Bundle and state paths (35a3859): macOS executable lookup and canonicalization resolve the installed symlink to the bundle. Smoke checks the packaged SDL license in `Contents/Resources`. The CLI and MCP share state resolution at `~/Library/Application Support/Splice` on macOS and `$XDG_STATE_HOME/splice` on Linux. Viewer sockets are under the state directory's `live/` subdirectory. The installed symlink passed smoke from a separate directory on the Mini.
- [x] T7.5 Bundle verifier (9792f32): `tools/check-macos-bundle.sh` adapts LocalWave's verifier to validate Splice's metadata, macOS 15.0 deployment ceiling, runtime paths, bundled dependencies, and signed entitlements. Bundling runs the static checks before publication. `make mac-check` also launches a hidden smoke test through a symlink and rejects a signed fixture missing a usage string.
- [ ] T7.6 (0.3) Notarize (`tools/notarize-macos.sh`) and a `.dmg`.

### T8 · Screen recording through MCP, almost headless (0.1, after T7) [M]

Recorder's capture, exposed as MCP and CLI. The key requirement: a **stable signature, a stable
location and permissions granted once**, so an agent can record from ssh or a terminal without a prompt.
Depends on T1.3, T4.1, T7.

- [ ] T8.0 Stable identity. One install location (`~/Applications/Splice.app`, symlinked from
  `~/.local/bin/splice`); capture only ever runs from that bundle, never from `target/`. One signer
  (T7.2). `packaging/designated-requirement.txt` pins `identifier "com.olegpustovit.splice" and anchor
  apple generic and certificate leaf[subject.OU] = "7ZH3FYV964"` (as `codesign -d -r-` prints it), and
  `tools/check-macos-bundle.sh` refuses a bundle whose requirement differs, because privacy grants are
  keyed on bundle ID + designated requirement. Install replaces the bundle in place after quitting
  running Splice processes normally. If the signer ever changes, it runs a targeted
  `tccutil reset {ScreenCapture,Microphone,Camera,ListenEvent} com.olegpustovit.splice` and says so;
  it never runs `tccutil reset All`. Accept: two consecutive `make dev` builds have an identical
  designated requirement, and the check refuses a re-signed copy.
- [ ] T8.1 Capture agent. macOS attributes capture to the *responsible* process, so a `splice mcp` that
  Codex or Claude spawns in a terminal would make the terminal ask for permission. Capture therefore
  runs inside Splice.app, launched by LaunchServices: `open -g -j -a ~/Applications/Splice.app --args
  --capture-agent` starts it with the accessory activation policy (no Dock icon, no window). It
  listens on the state-dir unix socket (getpeereid, same user only) and exits after 60 s idle.
  `splice mcp` and `splice record` are its clients. `ponytail:` an on-demand LaunchAgent
  plist instead, if `open` ever needs a GUI session the agent lacks. Accept: a socket round trip; an
  ssh-launched client starts the agent; the agent exits when idle.
- [ ] T8.2 Permissions. `splice permissions` and `record.start {dryRun:true}` report Screen Recording
  (`CGPreflightScreenCaptureAccess`), Microphone and Camera (`AVCaptureDevice authorizationStatus`),
  Input Monitoring (`CGPreflightListenEventAccess`), plus the identity (team, path, designated
  requirement match). `splice permissions --request` asks *from the agent process*
  (`CGRequestScreenCaptureAccess`, `requestAccessForMediaType`, `CGRequestListenEventAccess`) and opens
  the matching System Settings pane. **HUMAN, once on the Mini:** grant those four to Splice. A missing
  grant is `TCC_DENIED`, with a `fix` that names the pane and `splice permissions --request`.
  Risk: macOS 15+ can periodically ask the user to re-confirm screen-recording access for apps that
  capture without the system picker. The agent then gets `TCC_DENIED` with `needsHuman:true`, and the
  loop marks the task `HUMAN:` instead of retrying.
- [ ] T8.3 `apple_sys::capture`, `events`, `screenshot`: modern ScreenCaptureKit, bound the T4.1 way.
  - Sources: `SCShareableContent` lists displays, windows and apps, excluding Splice's own windows.
    `SCContentFilter` covers a display, a window, or a display area (`sourceRect`).
  - `SCStreamConfiguration` per Recorder SPEC §4.8: `showsCursor=false`, 60 fps, 420v for
    displays and BGRA for windows, queue depth 6, `capturesAudio` with `excludesCurrentProcessAudio`,
    `captureMicrophone` with the device ID, 48 kHz stereo.
  - Writers: `SCStream` outputs → one `AVAssetWriter` per source. Video is HEVC in realtime at about
    `pixels × 4` bps, capped at 60 Mbps; audio is AAC 48 kHz 192 kbps. Only complete frames are
    written. `t0` is the first complete screen frame; pauses are subtracted.
  - Camera: `AVCaptureSession` → `camera.mov`.
  - Events: a listen-only `CGEventTap` (move, down/up, drag, scroll, keyDown with the typed
    character, flagsChanged) in normalized source coordinates, polling the window frame at 10 Hz for
    window capture. Cursor images come from `NSCursor.currentSystem`, hashed into `cursors/`.
  - Stills: `SCScreenshotManager` for `record.start {still:true}` and agent screenshots.
  - The Rust side drains the event ring with no allocations in steady state.
  - Accept: 3 s display capture on the Mini: the files probe correctly, `events.json` parses, and the
    take duration matches wall time within 2 frames.
- [ ] T8.4 MCP and CLI.
  - `record.start {projectId | path, create, source: {display:"main"|id} | {window:{app, title}|id} | {area:{display, rect}}, mic, systemAudio, camera, events:true, duration?, countdown:0, insert:true, dryRun}`
    returns a `jobId`. `dryRun` returns the available sources and the permission states without recording.
  - `record.stop {jobId}` finalizes. `jobs.status` shows elapsed time, bytes and dropped frames;
    `jobs.cancel` discards the take.
  - On stop: the take is written to `<project>.splice/takes/<med>/` in Recorder's layout
    (`screen.mov`, `mic.m4a`, `system.m4a`, `camera.mov`, `events.json`, `cursors/`). It becomes a
    `kind:"take"` media entry with components (screen, mic, system, camera, cursor, keys) and is
    inserted on the spine as one agent batch (undoable). With `insert:false` it only lands in media.
  - `splice record` is the same thing in one command and creates the project when the path is new.
  - Linux: transcripts with fixture takes; live capture returns `UNSUPPORTED_PLATFORM`.
- [ ] T8.5 Acceptance on the Mini, after the one-time grant, all from ssh with short runs (the Mini is
  shared):
  1. `splice record /tmp/r.splice --display main --duration 3 --mic` gives a project with one take
     clip of 90 ± 6 frames and its events.
  2. `make dev`, then the same again: no prompt (stable signature).
  3. `make install`, then a `splice mcp` transcript from a Codex-spawned process: no prompt
     (responsible process = Splice.app).

### T9 · Text and captions (0.2) [M + L]
- [ ] T9.1 [M] `splice_speech`: SpeechAnalyzer transcription job → words with frame-accurate timing (forced alignment), transcript hash. Linux uses fixture transcripts.
- [ ] T9.2 [L] Caption commands and `transcript.get {captions: json|srt|vtt}` (from the prototype). Burn-in through T4.3.

## Log

Append one line per loop iteration: date, task, result, commit, wall time of the checks.

- 2026-11-18 · T0.1 · done · scaffold builds with 1.99.1; `cargo t` 23 s cold, 0 tests

- 2026-11-18 · T0.2 · done · e85a6d2 · `make check` 0.75 s, `make mac-check` 5.54 s (scaffold, 0 tests)
- 2026-11-18 · T0.3 · done · 0e16998 · `make check` 0.96 s (slowest: source shape rules 0.024 s)
- 2026-11-18 · T0.4 · done · 748047c · `make check` 0.87 s (7 tests, slowest: source shape rules 0.022 s)
- 2026-11-18 · T1.1 · done · edda50c · `cargo t -p splice_model` 0.64 s, `make check` 1.58 s (10 tests, slowest: transcript stub 0.022 s)
- 2026-11-18 · T1.2 · done · 2505b11 · `cargo t -p splice_json` 0.57 s, `make check` 1.05 s (15 tests, slowest: source shape rules 0.023 s)
- 2026-11-18 · T1.3 · done · 8efee3c · `cargo t -p splice_core package` 0.56 s, `make check` 1.12 s (21 tests, slowest: sample round-trip 0.041 s)
- 2026-11-18 · T1.4 · done · 8628ecf · `cargo t -p splice_core journal` 1.40 s, `make check` 1.65 s (25 tests, slowest: every-byte journal recovery 0.731 s)
- 2026-11-18 · T2.1 · done · 7bdcbc5 · `cargo t -p splice_core commands` 1.56 s, `make check` 2.48 s (29 tests, slowest: every-byte journal recovery 0.651 s)
- 2026-11-18 · T2.2 · done · 269824f · `cargo t -p splice_core edit::tests` 0.91 s, `make check` 1.75 s (33 tests, slowest: every-byte journal recovery 0.649 s)
- 2026-11-18 · T2.3 · done · 232b9f0 · `cargo t -p splice_core tier_a` 1.73 s, `make check` 1.70 s (39 tests, slowest: every-byte journal recovery 0.687 s)
- 2026-11-18 · T2.4 · done · 89ca2a7 · `cargo t -p splice_core history` 5.37 s, `make check` 7.30 s (44 tests, slowest: 500-batch history replay 2.142 s)
- 2026-11-18 · T2.5 · done · 809dcae · `cargo t -p splice_eval` 0.11 s, `make check` 6.01 s (48 tests, slowest: seeded history replay 1.839 s)
- 2026-11-18 · T2.6 · done · ab85713 · `cargo t -p splice_core budget` 0.20 s, release qualification 27.42 s (2,000-item apply p99 269 µs, zero warm allocations), `make check` 5.54 s (51 tests, slowest: seeded history replay 1.721 s)
- 2026-11-18 · T3.1 · done · 33e67ce · `cargo t -p splice_mcp` 0.34 s (3 tests), legacy and modern CLI smoke tests passed, `make check` 2.82 s (54 tests, slowest: seeded history replay 1.839 s)
- 2026-11-18 · T3.2 · done · b2a72c1 · `cargo t -p splice_mcp` 0.40 s (19 tool transcripts), CLI writer-lock and durable reopen smoke passed, `make check` 3.88 s (55 tests, slowest: seeded history replay 1.911 s)
- 2026-11-18 · T3.3 · done · 99c005a · `cargo t -p splice_mcp friction_transcript` 0.55 s (reconnect, tail recovery and matching logs), CLI replayed a generated repro, `make check` 3.54 s (56 tests, slowest: seeded history replay 1.930 s)
- 2026-11-18 · T3.4 · done · 3ddd172 · `cargo t -p splice_mcp transcript` 0.48 s (21 checked-in fixtures and generated repro, slowest: all tool fixtures 0.382 s), `cargo t -p splice_testkit transcript` 0.12 s, `make check` 3.14 s (56 tests, slowest: seeded history replay 2.056 s)
- 2026-11-18 · T3.5 · done · 80e8c4f · `cargo t -p splice_cli` 0.78 s (3 subprocess tests), `cargo t -p splice_core history_verify_accepts` 2.11 s with rebuild, stdin apply smoke passed, `make check` 3.36 s (60 tests, slowest: seeded history replay 1.679 s)
- 2026-11-18 · T4.1 · done · 6ea33b9 · `make check` 3.30 s (60 tests, slowest: seeded history replay 1.977 s), `make mac-check` 1.49 s (4 Apple bridge tests, all 0.030 s). The Mac target now selects the native crates as documented after the portable history test hit its unchanged 5 s limit on the Mini.
- 2026-11-18 · T4.2 · done · db51464 · `make check` 2.08 s (62 tests, slowest: seeded history replay 1.756 s), `make mac-check` 2.64 s (5 tests, slowest: stereo import 0.060 s), native `cargo t -p splice_cli cli_imports_media` 5.31 s (case 0.541 s). The same edit fixture passed with Linux descriptors. Interrupted import and undo checkpoints recover both the media table and the timeline.
- 2026-11-18 · T4.3 · done · d20d276 · `make check` 3.29 s (63 tests, slowest: seeded history replay 1.635 s), `make mac-check` 5.13 s (7 tests, slowest: stereo and title export 0.492 s). Native tests checked retiming, overlay fades, gaps, and the timing and dimensions of an MP4 with audio and a burned-in title.
- 2026-11-18 · T4.4 · done · 7d00f36 · `make check` 3.31 s (64 tests, slowest: seeded history replay 1.733 s), `make mac-check` 2.11 s (7 tests, slowest: stereo and title export 0.438 s), `cargo t -p splice_cli native_export` on the Mini 2.75 s (2 tests, slowest: CLI publication 1.354 s). `tools/check-macos-export.sh` took 14.37 s and verified 660 frames, 22.0 s, 1080 × 1920 and audio in both codecs. Cancelling a replacement at 9.3% kept the previous file.
- 2026-11-18 · T4.5 · done · b3125f2 · `make check` 3.65 s (65 tests, slowest: seeded history replay 1.673 s), `make mac-check` 4.40 s (7 tests, slowest: stereo and title export 0.442 s), `cargo t -p splice_cli native_` on the Mini 4.31 s (4 tests, frame case 1.222 s). `tools/check-macos-export.sh` took 25.95 s and matched four 1080 × 1920 PNGs against both codecs, with a worst-case mean RGB difference of 1.551/255.
- 2026-11-18 · T5.1 · done · 62db338 · `make check` 2.74 s (65 tests, slowest: seeded history replay 1.606 s), `make mac-check` 5.17 s (viewer smoke 1.108 s, first PXUI library build 42.59 s), full Mac `cargo t -p splice_cli` 2.17 s (8 tests, slowest: CLI export 1.172 s). The hidden viewer opened and closed both with and without a package and persisted the selected path.
- 2026-11-18 · T5.2 · done · df39fac · `make check` 3.59 s (65 tests, slowest: seeded history replay 1.609 s), `make mac-check` 11.40 s (9 native tests, 2 CLI smoke cases, slowest: viewer media smoke 1.633 s). The hidden viewer presented frame zero. The 4K test advanced and sought on the native player, measured zero warm Rust allocations and spent at most 4.079 ms converting, drawing and submitting after drawable acquisition.
- 2026-11-18 · T6.1 · done · b84e72c · `cargo t -p splice_kit commands` 0.38 s (3 tests, slowest: validation 0.021 s); `make check` 3.18 s (68 tests, slowest: seeded history replay 1.671 s). Default and overridden shortcut resolution allocated nothing after warm-up. Viewer wiring is part of T5.3.
- 2026-11-18 · T6.2 · done · 4554bf1 · `cargo t -p splice_kit history` 0.37 s (3 cases, each 0.021 s); `make check` 3.17 s (71 tests, slowest: seeded history replay 1.652 s). The ring keeps the latest step if its title exceeds the byte budget. The viewer will wire in labels from the core journal in T5.3.
- 2026-11-18 · T5.5 · done · 2eddf7e · `make check` 3.18 s (72 tests, slowest: seeded history replay 1.690 s); `make mac-check` 8.54 s (9 native tests and 3 viewer CLI tests, slowest: offscreen CLI snapshots 1.126 s). Frames 0 and 15 differed and an existing PNG was preserved. Pulled frame 15 with `make mac-pull` and verified the viewer label, picture, and letterboxing.
- 2026-11-18 · T5.3 · done · 0992642 · `make check` 4.78 s (73 tests, slowest: seeded history replay 1.685 s); `make mac-check` 13.49 s (13 native and 4 viewer CLI tests, slowest: playback smoke 1.796 s). Verified routed undo and redo after reopening, scrub capture cancellation, zero warm clock allocations, and the 960 × 640 play bar golden at frame 46. The 4K program test peaked at 4.338 ms excluding drawable acquisition.
- 2026-11-18 · T5.4 · done · 7c9a835 · `make check` 5.09 s (74 tests, slowest: seeded history replay 1.707 s); `make mac-check` 11.52 s (13 native and 5 viewer CLI tests, slowest: snapshot CLI 1.383 s); full Mac MCP and CLI tests 8.70 s (21 tests, slowest: tool transcripts 1.290 s). One real stdio connection edited a viewer project and read a separate headless project. Frame 15 changed after the agent edit and returned to identical pixels after undo, with Cmd-Z dispatch tested separately against an agent batch.
- 2026-11-18 · T7.1 · done · a670c56 · `make check` 3.37 s (74 tests, slowest: seeded history replay 1.729 s); `make mac-check` 7.81 s (both property lists passed `plutil`, 13 native and 5 viewer CLI tests, slowest: live MCP viewer 1.218 s). Verified the bundle ID, workspace version, deployment target, document declaration, and exact entitlement set with `plistlib`.
- 2026-11-18 · T7.2 · done · 3621e14 · `make check` 3.10 s (74 tests, slowest: seeded history replay 1.606 s); `make mac-check` 10.04 s warm (18 tests and signed bundle smoke, slowest: live MCP viewer 1.162 s). The Mini's Homebrew SDL targeted macOS 26.0, so the build now uses the checksum-verified PXUI SDL pin with a 15.0 target. First release build and signing acceptance took 86.04 s. Both bundles launched with identical designated requirements, and negative cases proved that missing credentials and unbundled dependencies refuse publication.
- 2026-11-18 · T7.3 · done · f05d175 · `make check` 3.25 s (74 tests, slowest: seeded history replay 1.566 s), `make mac-check` 14.20 s (18 tests and hidden Finder launch, slowest: offscreen viewer snapshots 1.508 s). On the Mini `make dev` took 2.29 s and `make install` took 20.38 s. The executable UUIDs matched their build profiles, both designated requirements matched, and `which splice` resolved to the installed release. Launch Services chose the installed bundle and opened a temporary project by document event.
- 2026-11-18 · T7.4 · done · 35a3859 · `cargo t -p splice_mcp paths` 0.82 s Linux, 2.55 s Mac (two cases); `make check` 3.69 s (76 tests, slowest: seeded history replay 1.687 s); `make mac-check` 14.63 s (18 tests plus symlink and Finder smoke, slowest: offscreen viewer snapshots 1.435 s). The live viewer test found its socket in a state path longer than Darwin permits in a socket address. The installed release smoke persisted the opened project to macOS state, and an ad hoc signed test copy without the license returned `BUNDLE_RESOURCES`.
- 2026-11-18 · T7.5 · done · 9792f32 · `make check` 3.22 s (76 tests, slowest: seeded history replay 1.646 s); `make mac-check` 9.98 s (18 tests, smoke, corrupt metadata regression and Finder launch, slowest: live MCP viewer 1.237 s). A 2.09 s Mini run also rejected missing metadata, a 15.0.1 deployment target, an extra rpath, an external dylib, a mismatched dylib ID and an extra entitlement. The installed release passed the full verifier.
