# M1 · Splice 0.1: the headless core

**Cut 0.1 landed on 2026-11-18.** The signed release is installed on the Mini and the live recording
checks passed across debug and release installs. The loop keeps going until the M1 exit (0.3).
**Now: 0.1.1 (T10, opening always works), then 0.2** per the 0.2 proposal
(https://artifacts.microapps.space/a/qg46h5ht5g07, accepted 2026-11-19).
Scope is fixed. When a task does not fit, ship it as a `ponytail:` with an upgrade
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
- **0.1.1:** T10 (writer handoff, capture-agent documents, offline media, visible errors, human-path matrix).
- **0.2 · headless-complete:** T3.6 outcome evals gate the cut, T2.7 Tier B (as T11), T12 recording
  render on the Metal compositor, T13 audio, T9 captions, T14 code mode, T6.3. Exit: every eval
  passes 3/3, friction triaged, no `PLANNED` reaches an agent for a row describe calls available.
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
- [ ] T2.7 [L] Tier B commands (0.2, tracked as T11): effects (`effect.add/set/remove/paste/promote`), components (`component.set`, `audio.set`, `cursor.set`, `keys.set`, `keys.hide`), cutting (`cut.range`, `cut.words`), `clip.crop`, `motion.add/set`, `clip.lift`, `overlay.replaceSpine`, captions (`captions.generate`, `caption.set/split/merge`). Then the 20 planned rows (0.3).

### T3 · MCP and CLI, headless
- [x] T3.1 [L] JSON-RPC 2.0 stdio (33e67ce), newline-delimited, hand-rolled. `initialize` negotiates 2025-11-25 and 2026-07-28. `tools/list` comes from the table in a fixed order with `inputSchema` and `outputSchema`. Results are `structuredContent` plus a text mirror; business errors are `isError` results with `code` and `fix`. Accept: a conformance transcript and a `tools/list` golden.
- [x] T3.2 [L] The 19 tools (docs/MCP.md) wired to the core (b2a72c1). `project.info {path, create}` opens a project headless; `ui.show` returns `{applied:false, reason:"headless"}` with no UI. Accept: one transcript per tool in `fixtures/mcp/`.
- [x] T3.3 [L] Friction log (99c005a): every `isError` result, the next calls and whether they recovered, and the optional `friction` argument every tool accepts → `<project>.splice/friction.jsonl` plus the global state dir. `splice friction [list|show|repro <id>|resolve <id> <sha>]`. Accept: transcript tests for logging, the recovery flag, and `repro` writing a failing transcript fixture.
- [x] T3.4 [L] Transcript runner (3ddd172): `fixtures/mcp/*.jsonl` replays request → expected structured result with `"…"` wildcards. Accept: every T3.2 fixture runs under `cargo t -p splice_mcp transcript`.
- [x] T3.5 [L] `splice` CLI (80e8c4f): `new`, `open`, `mcp`, `apply`, `snapshot`, `describe`, `export`, `frame`, `friction`, `check`, `version`, and `splice <path>` = open. On Linux, `open`, `export` and `frame` return the typed error `UNSUPPORTED_PLATFORM`. Accept: CLI tests over a temp project.
- [ ] T3.6 [L] (0.2, gates the cut) `.mcp.json` dogfood and `make eval`: `evals/tasks/*.toml` (prompt, seed project, end-state assertions checked by `splice check`) run `claude -p` with the `splice` skill against `splice mcp`. Results go to `evals/results.tsv` (pass, calls, errors, recoveries). Accept: 5 tasks; a baseline recorded. First task: `tiktok-textedit` (9:16 without letterbox bands, visible cursor ≥ 1.5×, ≥ 1 zoom, background frame, music ≤ −14 LUFS, export exists), recorded failing before T11 starts.

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

- [x] T8.0 Stable identity (76ceb0d). `packaging/designated-requirement.txt` pins the exact Developer ID
  requirement for `com.olegpustovit.splice` and team `7ZH3FYV964`. The verifier compares and evaluates
  the requirement. Installation requests normal termination at the target path and waits. A live
  CLI/MCP process leaves installation pending until its host disconnects it. At the default path a
  signer migration reports and resets the four Splice grants individually with `tccutil`. Two
  consecutive debug installs matched the pinned requirement, and the verifier rejected a re-signed
  copy.
- [x] T8.1 Capture agent (3a73473). Both `splice mcp` recording calls and `splice record --dry-run`
  start the signed `~/Applications/Splice.app` via Launch Services as an accessory without a window.
  The agent checks the bundle path, signing requirement and launchd parent, and verifies the peer's
  effective user. It removes `capture/agent.sock` and exits after 60 s idle. Both clients shared one
  agent from SSH on the Mini. `ponytail:` use an on-demand LaunchAgent if `open` needs a GUI session
  the caller lacks. Permission handling and the native recorder follow in T8.2–T8.4.
- [x] T8.2 Permissions (d8cc54f, 8fe23f0). The signed agent reports four read-only authorization states and
  its verified team, path and designated requirement match. `splice permissions --request` requests
  the first missing grant or the selected service and opens its privacy pane. `--request all`
  starts every missing request and opens the first missing pane. Capture fails with
  `TCC_DENIED` and `needsHuman:true` when a requested input lacks a grant.
  **Initial setup:** Run `splice permissions --request all` and approve the requests in macOS.
  The Mini reported all four grants across both signed install profiles in T8.5.
  Risk: macOS 15+ can periodically ask the user to re-confirm screen-recording access for apps that
  capture without the system picker. The agent then gets `TCC_DENIED` with `needsHuman:true`, and the
  loop marks the task `HUMAN:` instead of retrying.
- [x] T8.3 `apple_sys::capture`, `events`, `screenshot`: modern ScreenCaptureKit, bound the T4.1 way
  (a3673ed). On the Mini the display take produced one 90-frame clip in 3.005 s, with a probeable
  HEVC screen track, readable microphone audio and a parseable event log.
  - Partial (958c27a): installed preflight lists active displays without prompting. The
    ScreenCaptureKit bridge can list shareable windows and apps after the grant and defines the
    same display, window and area targets for PNG screenshots and the stream writer.
  - Partial (a3585f9): ScreenCaptureKit and AVAssetWriter bridge for `screen.mov`, `mic.m4a`,
    and `system.m4a`. A native fixture verified HEVC and stereo AAC, complete-frame filtering,
    removal of paused time, rejection of samples from a pause after resume, and idle frame hold.
  - Partial (eed54ae): optional default camera via AVCaptureSession writes `camera.mov` using the
    native host clock and the take's pause offsets. Generated frames retained the camera's leading
    offset and resumed without a timestamp gap.
  - Partial (f78af91): a listen-only input tap feeds the take clock and a bounded ring. The fixture
    verified normalized positions, secure-input filtering, keyboard destination and pause cutoff.
    Rust drained and decoded 1,024 records with zero allocations.
  - `events.json` writes input relative to the screen clock, and cursor events reuse PNG and hotspot
    files by digest. Fixtures verify the format, shape reuse and allocation-free input serialization.
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
- [x] T8.4 MCP and CLI (063d7e0, bfd4a08).
  - Partial (f239a6d): the signed capture agent schedules and seals takes in a private staging
    directory. The agent validates source selection, maintains countdown and elapsed time, and can
    cancel and discard staged output.
  - Partial (7219a41): MCP starts, checks, cancels and stops recording jobs. The project writer
    publishes the take descriptor and optional clip together, including when the viewer owns the
    project.
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
- [x] T8.5 Acceptance on the Mini from SSH (8fe23f0). Each three-second capture produced one
  90-frame take with microphone audio and input events in one checked journal batch.
  1. Installed release CLI `splice record … --display main --duration 3 --mic` recorded in 3.005 s.
  2. After `make dev`, the same CLI command recorded in 3.012 s without a prompt.
  3. After `make install`, a Codex-spawned `splice mcp` transcript recorded in 3.005 s without a
     prompt. Each profile kept all four grants, the installed path and the designated requirement.

### T9 · Text and captions (0.2) [M + L]
- [ ] T9.1 [M] `splice_speech`: SpeechAnalyzer transcription job → words with frame-accurate timing (forced alignment), transcript hash. Linux uses fixture transcripts.
- [ ] T9.2 [L] Caption commands and `transcript.get {captions: json|srt|vtt}` (from the prototype). Burn-in through T4.3.

## 0.2 decisions (2026-11-19)

- **D1** Start the M3 compositor inside 0.2. It renders on **OGPU** (PXUI's portable GPU layer:
  Metal now, Vulkan later), sharing the viewer's device and textures, with decoded frames imported
  zero-copy (IOSurface on macOS, dmabuf on Linux). The layer plan (ramps, zoom keyframes, cursor
  tables) is portable Rust. Amended 2026-11-19: the first cut renders with Core Image inside a custom
  `AVVideoCompositing` as an interim `ponytail:` (T12.1); T12.7 moves it to OGPU.
- **D2** `quickjs_sys` (vendored QuickJS-NG, built by `cc`) is allowed, behind a gate allowlist
  entry with measured binary size and build time.
- **D3** Code mode is a tool profile, `splice mcp --code` (`code.run` + `commands.describe`), so
  the default profile stays under 20 tools and the two profiles A/B in evals.
- **D4** Music goes on audio lanes (`LaneKind::Audio`), not media overlays.

Process (docs/LOOP.md): outcome evals gate cuts; the human-path matrix runs in `mac-check`; a row
is available only with an apply test; visual features ship a frame golden from a real take; each
cut logs one "try it as a user" line.

### T10 · 0.1.1: opening always works [M]
- [x] T10.1 (a706ba9) Writer handoff. A headless MCP writer serves the per-project live socket. A viewer that
  gets `LOCKED` asks the holder to hand off; the holder releases the lock and forwards to the
  viewer. Accept: MCP `project.info`, then open → a window, and the next `edit.apply` on the same
  MCP connection lands in the viewer.
- [x] T10.2 (a706ba9) The capture agent never keeps a document: it relaunches a new viewer instance with the
  path. Accept: `record --dry-run`, then `open x.splice` → a viewer process with the path.
- [x] T10.3 (a706ba9) Offline media is not fatal: the viewer opens and shows an offline slate; export refuses
  with `OFFLINE_MEDIA`. Accept: the sample fixture without assets opens.
- [x] T10.4 (a706ba9) No silent failures: a viewer error shows an alert with code, message and fix and is
  appended to the global friction log.
- [x] T10.5 (a706ba9) Human-path matrix in `mac-check`: Launch Services open × {clean, MCP holding, agent
  alive, offline media} checks a live viewer process and no error. The agent-alive case needs the
  installed app and is checked by hand after `make install`.

### T11 · Headless-complete editing (0.2) [L]
- [ ] T11.1 `clip.crop` (eval and AV already render it).
- [ ] T11.2 `effect.add/set/remove/paste/promote`.
- [ ] T11.3 `component.set`, `audio.set`, `cursor.set`, `keys.set/hide`.
- [ ] T11.4 `cut.range`, `clip.lift`, `overlay.replaceSpine`.
- [ ] T11.5 `motion.add/set` with focal point and easing.
- [ ] T11.6 Describe tells the truth: `available` per row; the parity test fails a row that is
  available without an apply test.

### T12 · Recording render on the Metal compositor (0.2) [M + L]
- [ ] T12.1 `AVVideoCompositing` Metal compositor from the eval plan; parity ≤ 2/255 with the current
  lowering; zero warm allocations per frame.
- [ ] T12.2 Cursor layer from `events.json` + `cursors/`: size, smoothing, idle hide, click rings.
- [ ] T12.3 Auto-zoom solver: clicks and typing → zoom keyframes with focal points; `intent.autoZoom`.
- [ ] T12.4 Window frame: corner radius, shadow, inset, wallpaper/gradient presets.
- [ ] T12.5 Keystroke chips and the camera bubble mask.
- [ ] T12.6 `intent.reframe {aspect}`: crop to the active window or area, no letterbox bands.
- [ ] T12.7 OGPU compositor: `ogpu_metal` IOSurface import (the macOS twin of `import_dmabuf`), a
  portable `splice_gpu` renderer for the layer plan (MSL now, SPIR-V with Vulkan), used by the viewer
  (zero-copy into its texture), frame grabs and export. Accept: window_program_4k back under 8 ms,
  pixel parity ≤ 2/255 with T12.1, zero warm Rust allocations per frame.

### T13 · Audio (0.2) [L + M]
- [ ] T13.1 `LaneKind::Audio`; `clip.insert {path, lane}` for music; per-item `volume`.
- [ ] T13.2 `audio.duck {under, amount, attack, release}` as eval gain ramps.
- [ ] T13.3 Native loudness in `audio.summary`; `audio.normalize {target}`.
- [ ] T13.4 `record.start` defaults `systemAudio` on for window and app sources and warns on silent takes.

### T14 · Code mode (0.2) [L]
- [x] T14.1 (b9835d6) `quickjs_sys`: vendored QuickJS-NG via `cc`, safe `run(src, limits, host)`; no libc
  module, loader or timers; interrupt, memory and stack limits.
- [x] T14.2 (b9835d6) `splice mcp --code`: `code.run {code, projectId?, sessionId?}` →
  `{result, logs, calls, revisions, error{code, message, fix, line}}`, dispatching through
  `tools::call`; limits 64 KB source, 2 s, 32 MB, 200 calls, 32 KB output.
- [x] T14.3 (b9835d6) `.d.ts` generated from `COMMANDS` and tool schemas, `commands.describe {format:"ts"}`,
  golden-locked; optional fields omittable, never `null`.
- [x] T14.4 (b9835d6) Adversarial tests: infinite loop, memory bomb, deep recursion, `SpliceError` keeps
  `fix`, line numbers map to the user's source.
- [ ] T14.5 A/B the T3.6 evals with and without code mode (calls, tokens, wall, pass rate).

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
- 2026-11-18 · T8.0 · done · 76ceb0d · `make check` 4.15 s (76 tests, slowest: seeded history replay 2.330 s); `make mac-check` 10.77 s (18 tests and bundle checks, slowest: live MCP viewer 1.188 s). Consecutive debug installs took 3.44 s and 2.81 s and matched the installed release's requirement. A hidden AppKit fixture took the normal quit path, a live installed MCP subprocess vetoed replacement, and a mocked migration issued exactly four bundle-specific resets and kept the previous app after a simulated failure. The real grants were untouched. The matching release was reinstalled in 2.98 s.
- 2026-11-18 · T8.1 · done · 3a73473 · `make check` 3.55 s (77 tests, slowest: seeded history replay 1.690 s); `make mac-check` 14.30 s (20 tests and bundle checks, slowest: offscreen viewer snapshots 1.293 s). The installed release took 21.49 s to build and install. The 83.15 s SSH check rejected staged and direct launches, verified the shared installed process's accessory policy and absence of windows, and observed socket cleanup and process exit after 60.19 s idle.
- 2026-11-18 · T8.2 · HUMAN · d8cc54f · `make check` 4.24 s (78 tests, slowest: seeded history replay 1.787 s); `make mac-check` 24.68 s (21 tests and bundle checks, slowest: live MCP viewer 1.322 s). The 25.70 s SSH check verified all three read-only entry points, the installed identity, `TCC_DENIED` with the matching pane and no project writes, and normal agent shutdown. The Mini returned `notGranted` for screen and input monitoring and `notDetermined` for microphone and camera. Interactive grant requests await the human step.
- 2026-11-18 · T8.3 · partial · 958c27a · `make check` 3.51 s (78 tests, slowest: seeded history replay 1.730 s); `make mac-check` 27.16 s (23 tests and bundle checks, slowest: offscreen viewer snapshots 1.454 s). The installed preflight changed from no sources to one display with a valid main selector and no window or application disclosure without the grant. The native screenshot bridge rejects invalid geometry before querying permissions. Real screenshots, streams, camera, and input capture are pending.
- 2026-11-18 · T8.3 · partial · a3585f9 · `cargo t -p apple_sys native_recording_` 6.58 s (case 0.265 s); `make check` 3.37 s (78 tests, slowest: seeded history replay 1.628 s); `make mac-check` 21.92 s (24 tests and bundle checks, slowest: offscreen viewer snapshots 1.491 s). Generated 420v frames produced a 96 × 64 HEVC movie with a one-second duration after removing a one-second pause. Both optional audio writers produced stereo AAC. Live capture, camera, input events and agent job wiring remain open.
- 2026-11-18 · T8.3 · partial · eed54ae · `make check` 3.29 s (78 tests, slowest: seeded history replay 1.648 s); `make mac-check` 21.09 s (24 tests and bundle checks, slowest: offscreen viewer snapshots 1.439 s). The 0.309 s native recording fixture verified that HEVC `camera.mov` retained a two-frame offset in its QuickTime edit list, resumed at the correct sample time after a one-second pause, and ended on the screen timeline. The camera hardware was not opened during testing.
- 2026-11-18 · T8.3 · partial · f78af91 · `cargo t -p apple_sys input_events_` 10.73 s (case 0.031 s); `make check` 3.39 s (78 tests, slowest: seeded history replay 1.646 s); `make mac-check` 24.78 s (25 tests and bundle checks, slowest: offscreen viewer snapshots 1.496 s). A synthetic Quartz event verified normalization and scope without posting input or opening a live tap.
- 2026-11-18 · T8.3 · HUMAN · a3673ed · `cargo t -p apple_sys -p splice_capture` 9.46 s (10 tests, slowest: native HEVC and AAC writer 0.296 s); `make check` 3.91 s (78 tests, slowest: seeded history replay 1.707 s); `make mac-check` 17.60 s (28 tests and bundle checks, slowest: offscreen viewer snapshots 1.476 s). Generated cursor and input fixtures verified the saved event format, pixel hotspots, stable digests, asset reuse, and zero allocations during input serialization. The three-second live check requires the Mini grants and the recording job.
- 2026-11-18 · T8.4 · partial · f239a6d · `make check` 3.32 s (79 tests, slowest: seeded history replay 1.748 s); `make mac-check` 18.84 s (30 tests and bundle checks, slowest: offscreen viewer snapshots 1.543 s). Fixtures verified selector validation, ambiguous window rejection, countdown, exclusive capture and staged output cleanup on cancellation. Publication by the project writer, public MCP and CLI wiring, and the live take remain open.
- 2026-11-18 · T8.4 · partial · 7219a41 · `make check` 18.10 s (80 tests, slowest: seeded history replay 3.016 s); `make mac-check` 24.79 s (31 tests and bundle checks, slowest: offscreen viewer snapshots 1.693 s). Fixture captures verified one-batch import, media-only import, stale-write preservation, undo and journal replay. Native publication probed a movie and a generated PNG in 0.173 s. The CLI and live Mini acceptance remain open.
- 2026-11-18 · T8.4 · done · 063d7e0 · `make check` 3.97 s (83 tests, slowest: seeded history replay 1.968 s); `make mac-check` 21.77 s (32 tests and bundle checks, slowest: live MCP viewer 1.335 s). The CLI uses the same project and job routes as MCP. Fixture jobs verified timed publication and manual cancellation. The native receiver discards unfinished takes on writer disconnect (0.031 s). Live Mini acceptance remains in T8.5.
- 2026-11-18 · T8.4 · done · bfd4a08 · `make check` 3.33 s (83 tests, slowest: seeded history replay 1.566 s); `make mac-check` 19.70 s (32 tests and bundle checks, slowest: live MCP viewer and playbar golden 1.298 s). Direct area and app-name flags now cover the selectors in the cut contract. Ambiguous app selections require a title or ID.
- 2026-11-18 · T8.5 · HUMAN · bfd4a08 · Signed release installation and read-only preflight on the Mini took 89.41 s including the idle exit. The CLI showed the new selectors and the bundle matched Team ID 7ZH3FYV964 and the pinned requirement. Screen and input monitoring are `notGranted`; microphone and camera are `notDetermined`. Live capture, debug reinstallation without a prompt, and live MCP capture remain pending.
- 2026-11-18 · T8.5 · HUMAN · Signed release check from SSH took 63.07 s including the idle exit. Both CLI and MCP returned the same `TCC_DENIED` with `needsHuman:true` for Screen & System Audio Recording and left both requested project paths absent. MCP preflight still listed the main display. `make check` passed in 3.34 s (83 tests, slowest: seeded history replay 1.607 s). The 0.1 release budget check passed in 6.37 s at 236 µs p99 over 300 samples on 2,000 items, with zero warm allocations.
- 2026-11-18 · T8.2 · `--request all` starts every missing permission request and opens one Settings pane. `make check` passed in 3.69 s (83 tests, slowest: seeded history replay 1.666 s). `make mac-check` passed in 25.90 s (33 tests and bundle checks, slowest: offscreen viewer snapshots 1.533 s). The batch permission fixture verified one Settings launch and skipped granted permissions in 0.031 s.
- 2026-11-18 · T8.2–T8.5 · done · 8fe23f0 · Live acceptance from SSH took 25.80 s including `make dev` and `make install`. All four grants persisted across the signed installs. The release CLI, debug CLI and release MCP captures each published one 90-frame take in one verified journal batch (3.005 s, 3.012 s and 3.005 s). Each HEVC probe passed, microphone audio opened with `afinfo` and the event logs parsed. The installed release reports 0.1.0 and the capture agent exited. Final `make check` passed in 3.40 s (83 tests, slowest: seeded history replay 1.606 s). Final `make mac-check` passed in 35.12 s (33 tests and bundle checks, slowest: native title and audio composition 4.356 s).
- 2026-11-19 · T10.1–T10.5 · done · a706ba9 · Reproduced the 0.1 blink on the Mini: a project held by an agent's `splice mcp` exited with `LOCKED`, and a document opened within 60 s of a recording went to the windowless capture agent. Both now open, as do projects with offline media. `cargo t -p splice_cli native_viewer_takes_over` 0.446 s on the Mini (handoff keeps the agent's session; its next edit lands in the viewer at r2). `make check` 3.04 s (83 tests, slowest: seeded history replay 1.727 s); `make mac-check` 22.05 s (19 native tests plus the 3-case Finder matrix). On the installed release: MCP held → window and forwarded `timeline.snapshot`; capture agent alive → new viewer with the path; offline sample → window; corrupt package → alert plus a `viewer` friction entry. A person can now double-click any project an agent made, at any time, and see it.
- 2026-11-19 · T14.1–T14.4 · done · b9835d6 · `splice mcp --code` lists `code.run` and `commands.describe`; one `code.run` did project.info, session.start, edit.apply and timeline.snapshot on the sample (fixtures/mcp/code). QuickJS-NG v0.18.0 vendored without libc: +1.22 MB stripped release `splice`, 12.9 s clean `quickjs_sys` build. `cargo t -p quickjs_sys -p splice_mcp` 0.58 s (25 tests, slowest: infinite-loop timeout 0.30 s); `make check` 3.76 s warm, 32.9 s after the C build (93 tests, slowest: seeded history replay 1.686 s). Not run: `make mac-check` (no mac crate touched; splice_mcp now compiles QuickJS with clang on the Mini).
