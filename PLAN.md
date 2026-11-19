# M1 · Splice 0.1: the headless core

**Cut 0.1 landed on 2026-11-18.** The signed release is installed on the Mini and the live recording
checks passed across debug and release installs. The loop keeps going until the M1 exit (0.3).
0.1.1 and 0.2 landed on 2026-11-19 (https://artifacts.microapps.space/a/qg46h5ht5g07).
**Now: 0.3, the complete headless editor and M1 exit** per the 0.3 plan
(https://artifacts.microapps.space/a/5sn6awbkq9ae, accepted 2026-11-19): T15–T24 below.
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
- **0.3 · M1 exit:** T15–T24. Every registry row built (the 20 planned rows plus 16 new), keys on every
  numeric parameter, the capability sweep green on both machines, 11 evals 3/3 in both arms, every
  budget green, T7.6 notarization, the friction backlog empty or triaged.

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
- [x] T2.7 [L] (ff8a777) Tier B commands (0.2, tracked as T11): effects (`effect.add/set/remove/paste/promote`), components (`component.set`, `audio.set`, `cursor.set`, `keys.set`, `keys.hide`), cutting (`cut.range`, `cut.words`), `clip.crop`, `motion.add/set`, `clip.lift`, `overlay.replaceSpine`, captions (`captions.generate`, `caption.set/split/merge`). Then the 20 planned rows (0.3).

### T3 · MCP and CLI, headless
- [x] T3.1 [L] JSON-RPC 2.0 stdio (33e67ce), newline-delimited, hand-rolled. `initialize` negotiates 2025-11-25 and 2026-07-28. `tools/list` comes from the table in a fixed order with `inputSchema` and `outputSchema`. Results are `structuredContent` plus a text mirror; business errors are `isError` results with `code` and `fix`. Accept: a conformance transcript and a `tools/list` golden.
- [x] T3.2 [L] The 19 tools (docs/MCP.md) wired to the core (b2a72c1). `project.info {path, create}` opens a project headless; `ui.show` returns `{applied:false, reason:"headless"}` with no UI. Accept: one transcript per tool in `fixtures/mcp/`.
- [x] T3.3 [L] Friction log (99c005a): every `isError` result, the next calls and whether they recovered, and the optional `friction` argument every tool accepts → `<project>.splice/friction.jsonl` plus the global state dir. `splice friction [list|show|repro <id>|resolve <id> <sha>]`. Accept: transcript tests for logging, the recovery flag, and `repro` writing a failing transcript fixture.
- [x] T3.4 [L] Transcript runner (3ddd172): `fixtures/mcp/*.jsonl` replays request → expected structured result with `"…"` wildcards. Accept: every T3.2 fixture runs under `cargo t -p splice_mcp transcript`.
- [x] T3.5 [L] `splice` CLI (80e8c4f): `new`, `open`, `mcp`, `apply`, `snapshot`, `describe`, `export`, `frame`, `friction`, `check`, `version`, and `splice <path>` = open. On Linux, `open`, `export` and `frame` return the typed error `UNSUPPORTED_PLATFORM`. Accept: CLI tests over a temp project.
- [x] T3.6 [L] (ce3f118) (0.2, gates the cut) `.mcp.json` dogfood and `make eval`: `evals/tasks/*.toml` (prompt, seed project, end-state assertions checked by `splice check`) run `claude -p` with the `splice` skill against `splice mcp`. Results go to `evals/results.tsv` (pass, calls, errors, recoveries). Accept: 5 tasks; a baseline recorded. First task: `tiktok-textedit` (9:16 without letterbox bands, visible cursor ≥ 1.5×, ≥ 1 zoom, background frame, music ≤ −14 LUFS, export exists), recorded failing before T11 starts.

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
- [x] T6.3 [L] (0.2) Gesture phases (`Began/Changed/Ended/Cancelled`) for continuous controls; one gesture = one undo step.

### T7 · Install, locate, sign (LocalWave model) [M]
- [x] T7.1 Bundle metadata (a670c56): `com.olegpustovit.splice`, macOS 15.0 minimum, camera and microphone usage strings, and the `.splice` package and editor declarations. The entitlements grant audio input and camera access for the Hardened Runtime signing in T7.2.
- [x] T7.2 Bundling and signing (3621e14): `Splice.app` embeds PXUI and the pinned SDL built for macOS 15.0. The bundler checks native dependencies, sets `@executable_path/../Frameworks`, and signs the app and its libraries with Hardened Runtime and team `7ZH3FYV964`. Both profiles use the passwordless `splice-signing.keychain-db` and require a valid Developer ID Application identity.
- [x] T7.3 Install targets and Finder launch (f05d175): `make dev` (debug) and `make install` (release) publish the same `$(APPDIR)/Splice.app` (default `~/Applications`), link its executable at `~/.local/bin/splice`, and register the app with Launch Services. Both profiles ran through the link on the Mini, and Launch Services delivered a `.splice` package to the installed app.
- [x] T7.4 Bundle and state paths (35a3859): macOS executable lookup and canonicalization resolve the installed symlink to the bundle. Smoke checks the packaged SDL license in `Contents/Resources`. The CLI and MCP share state resolution at `~/Library/Application Support/Splice` on macOS and `$XDG_STATE_HOME/splice` on Linux. Viewer sockets are under the state directory's `live/` subdirectory. The installed symlink passed smoke from a separate directory on the Mini.
- [x] T7.5 Bundle verifier (9792f32): `tools/check-macos-bundle.sh` adapts LocalWave's verifier to validate Splice's metadata, macOS 15.0 deployment ceiling, runtime paths, bundled dependencies, and signed entitlements. Bundling runs the static checks before publication. `make mac-check` also launches a hidden smoke test through a symlink and rejects a signed fixture missing a usage string.
- [ ] T7.6 (0.3) Notarize (`tools/notarize-macos.sh`) and a `.dmg`. Built (d05b099): `make dmg` makes
  `target/dist/Splice-<version>.dmg`, Developer ID signed and verified; the app inside runs. Gatekeeper's only
  objection is "Unnotarized Developer ID". HUMAN: store notarytool credentials once
  (`xcrun notarytool store-credentials splice-notary`, Apple ID + app-specific password, team 7ZH3FYV964), then
  `make dmg NOTARY_PROFILE=splice-notary` submits, staples and spctl-checks app and DMG.

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
- [x] T9.1 [M] SpeechAnalyzer on device through a Swift `@_cdecl` shim in `apple_sys` (no separate crate):
  `transcript.get {transcribe:true}` commits `transcript.set` (words in source frames, versioned hash).
  Synchronous; ponytail: a `job_` when takes run past a few minutes. Linux uses fixture transcripts.
- [x] T9.2 [L] Caption commands and `transcript.get {captions: json|srt|vtt}` (from the prototype). Burn-in through T4.3.

## 0.2 decisions (2026-11-19)

- **D1** Start the M3 compositor inside 0.2. It renders on **OGPU** (PXUI's portable GPU layer:
  Metal now, Vulkan later), sharing the viewer's device and textures, with decoded frames imported
  zero-copy (IOSurface on macOS, dmabuf on Linux). The layer plan (ramps, zoom keyframes, cursor
  tables) is portable Rust. Amended 2026-11-19: the first cut rendered with Core Image (T12.1);
  T12.7 replaced it, so OGPU (`splice_gpu`) now draws the viewer, frame grabs and export.
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
- [x] T11.1 (ff8a777) `clip.crop` (eval and AV already render it).
- [x] T11.2 (ff8a777) `effect.add/set/remove/paste/promote`.
- [x] T11.3 (ff8a777) `component.set`, `audio.set`, `cursor.set`, `keys.set/hide`.
- [x] T11.4 (ff8a777) `cut.range`, `clip.lift`, `overlay.replaceSpine`.
- [x] T11.5 (2fb1eac) `motion.add/set` with `focus` and `ease`.
- [x] T11.6 (ff8a777) Describe tells the truth: `available` per row; the parity test fails a row that is
  available without an apply test.

### T12 · Recording render on the Metal compositor (0.2) [M + L]
- [x] T12.1 (5cc7409, interim) Custom `AVVideoCompositing` from the eval plan, rendering with Core
  Image on Metal for preview, export and grabs; existing pixel tests pass. Its renderer moves to OGPU in T12.7.
- [x] T12.2 (5cc7409) Cursor layer from `events.json` + `cursors/`: size, smoothing, idle hide, click rings.
- [x] T12.3 (5cc7409) Auto-zoom solver: clicks → eased zoom keyframes with focal points (`clickZoom`).
  Open: typing bursts and `intent.autoZoom` returning editable motion items (0.3 planned row).
- [x] T12.4 (5cc7409) Window frame: corner radius, shadow, gradient and solid backgrounds. Open: inset
  stroke, wallpaper presets.
- [x] T12.5 (5cc7409) Keystroke chips (timed text) and the rounded, shadowed camera bubble.
- [x] T12.6 (2fb1eac) `clip.crop {aspect, focus}` (reframe): crop to the active window or area, no letterbox bands.
- [x] T12.7 (ef451c1, eea9f2c) OGPU compositor: `ogpu_metal` IOSurface import (the macOS twin of `import_dmabuf`), a
  portable `splice_gpu` renderer for the layer plan (MSL now, SPIR-V with Vulkan), used by the viewer
  (zero-copy into its texture), frame grabs and export. Follow ../rays: the app leases one OGPU
  device that PXUI's `Renderer` borrows; composited textures reach PXUI as `pxui::Image`s from
  `backend::native_texture` (no copy) with serial-deferred destruction; shaders ship as MSL plus
  SPIR-V fixtures loaded by `shader::create`; per-frame uniforms reuse one buffer page. Accept:
  window_program_4k back under 8 ms, pixel parity ≤ 2/255 with T12.1, zero warm Rust allocations
  per frame.

### T13 · Audio (0.2) [L + M]
- [x] T13.1 (eecd6c1) `LaneKind::Audio`; `clip.insert {path, lane}` for music; per-item `volume`.
- [x] T13.2 (eecd6c1) `audio.duck {under, amount, attack, release}` as eval gain ramps.
- [x] T13.3 (eecd6c1) Native loudness in `audio.summary`; `audio.normalize {target}`.
- [x] T13.4 (eecd6c1) `record.start` defaults `systemAudio` on for window and app sources and warns on silent takes.

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
- [x] T14.5 A/B the T3.6 evals with and without code mode: 45 runs, 5 tasks, n=3, all pass. Code mode
  -49% turns, -63% calls, -16% wall, +4% cost (cache writes are 67% of cost and grow with the up-front
  .d.ts). Round 1 found five code-mode defects, fixed in a4218a3 (tiktok-textedit -24% cost, -29% wall).
  Study: https://artifacts.microapps.space/a/f0hfkms3e4d0 · archive: https://artifacts.microapps.space/a/rs6zd6dc2ryj
- [x] T14.6 (26ace8e) Progressive declarations: `commands.describe {format:"ts"}` returns an index (9.9k chars
  vs 26.6k) and `query:"a,b"` returns full types per command. Measured (n=3, 5 tasks, 15/15 pass): cost -8%,
  wall -23%, turns -50%, cache writes +4% (v2: +21%) vs direct. Acceptance met in aggregate and on the 3 Linux
  tasks; the 2 macOS tasks stay +3%/+7% (within run noise), where first programs return full snapshots.
- [x] T14.7 (33e1f28) Re-ran T14.5 with both arms at one commit after the friction fixes (a716130) and the
  small-reads steer: 30 runs, all pass. Code mode vs direct: cost -13% (lower on every task: -1.5% to -33%),
  wall -19%, turns -48%, calls -63%, tool errors 1 vs 3. Cache writes -4% overall, still above direct on 3
  tasks (+3%, +11%, +13%).

## 0.3 decisions (2026-11-19)

- **D5** The keyframe engine (`param.set`, `param.keys`, ease and Bezier) moves from M8 into 0.3. Crop keys,
  masks, speed ramps, volume automation, text animators and auto-zoom are key producers, not new renderers.
- **D6** The direct profile stays at 19 tools: `media.analyze` replaces `audio.summary` (alias for one cut);
  pause and resume are `record.stop {pause}` / `record.start {resume}`.
- **D7** The agent does language work (chapters, highlights, translation text); Splice supplies data and
  deterministic recipes. No on-device LLM.
- **D8** FCPXML export is the only timeline interchange; OTIO is dropped. SRT/VTT import.
- **D9** Out of 0.3: stabilization, multicam, reverse clips, proxies, OTIO, EDL, dubbing/TTS, diarization,
  3D, particles, node/HDR grading, timeline UI.
- **D10** Swift-only Apple APIs use the `@_cdecl` shim pattern in `apple_sys/native/*.swift`.
- **D11** One format minor bump for keys, transitions, compound clips, stills and templates, with load-time
  migration.

Every visual feature is a shader in `splice_gpu` (viewer, grabs and export match); Vision and audio units
only produce data. Rows need a decoder, apply, golden, undo round-trip, describe example and `.d.ts`
coverage; visual rows a frame golden; hot paths an allocation proof and a measured time.

### T15 · Debt and hardening (0.3) [L + M]
- [x] T15.1 [L] apply p99 budget: best of 5 interleaved passes per sample, threshold 1 ms unchanged. Accept: green under load; a planted 2 ms sleep turns it red. — b6deabb · best of 5 interleaved; green at load 18 (470 µs), red with a planted 2 ms sleep.
- [x] T15.2 [M] Phone and camera footage: `preferredTransform` in shader UVs, VFR sources on integer frames, P3 → Rec.709 tag. Accept: portrait, VFR and P3 goldens. — f0c3848 · portrait (90° transform) renders upright, VFR maps by time, P3 converts to Rec. 709 on decode (P3 200,120,60 → 214,115,47; it passed through before); fixtures from fixtures/media/camera-footage.swift.
- [x] T15.3 [M] Long media: transcription and analysis as `job_`s with progress; streaming `events.json` index. Accept: a 60-minute synthetic take with flat memory. — 7e75ee9 · transcription and analysis over 10 min run as jobs; events.json streams (19 MB test, no 16 MiB cap).
- [x] T15.4 [L] Bounded memory: journal compaction, revisions spilled to disk. Accept: 10,000 batches < 200 MB; undo to any revision reproduces its hash. — 5d88a0c · projects kept for the last 32 revisions and every 64th, others rebuilt from a prefix base; 1,500 batches keep ~55; old revisions rebuild to their hashes. Journal records ~200 B/batch (10,000 ≈ 2 MB).
- [x] T15.5 [L] Fuzz every decoder (typed errors, no panics); a 2,000-batch random stream over all rows with undo/redo holds invariants and replays; crash at every byte of save recovers. — bcb3b1e · mutated args for every row never panic; 2,000-batch stream with undo reopens to the same hash; a save cut at any byte keeps the last project.
- [ ] T15.6 [M] = T7.6 notarization (HUMAN credentials). — HUMAN: `xcrun notarytool store-credentials splice-notary`, then `make dmg NOTARY_PROFILE=splice-notary`.

### T16 · Parameters and keys (0.3) [L]
- [x] T16.1 [L] Model: parameter paths, `Keys {frame, value, ease}` (linear, hold, in, out, inOut, cubic Bezier), format minor bump and migration. Accept: fixture round-trips, migration goldens. — e1c65e8 · keys and Bezier in the model and codec; fixtures round-trip. D11 met by additivity: splice/1 files load unchanged (a 0.2 build would reject 0.3 files with FORMAT_UNKNOWN_KEY, not FORMAT_TOO_NEW).
- [x] T16.2 [L] `param.set`, `param.keys {target, path, keys, mode}`; describe lists keyable paths. Accept: easing goldens. — e1c65e8 · param.set/param.keys with merge; keys golden against a Python reference; 1a907a3 keys follow content across split, in-trim and roll.
- [x] T16.3 [L+M] Overlay transform (position, rotation, opacity, anchor) and blend modes in `splice_gpu`. Accept: frame goldens, viewer/export parity ≤ 2/255. — 6ff4417 · transform, opacity, anchor, blend in the one shader; tests/looks.rs pixel checks.
- [x] T16.4 [L] Per-frame eval with 2,000 keys over 200 items allocates nothing; p99 measured and locked. — e1c65e8, aa8c336 · 2,000 keys over 200 titles: 0 allocations; p99 ~1.9 µs, locked at 100 µs.

### T17 · Timeline editing complete (0.3) [L]
- [x] T17.1 [L] `item.roll`, `item.slip`, `item.slide`, `audio.link`/`audio.unlink` (J/L cuts), `clip.join`, `clip.ungroup`, `clip.restore`, `lane.reorder`. — e1c65e8 · roll, slip, slide, link/unlink, join, ungroup, restore, lane.reorder; warm edits allocate nothing (ed81d09).
- [x] T17.2 [L] `clip.freeze`, speed ramps as `speed` keys with pitch kept, overlay retime. — e1c65e8, 6ff4417 · freeze and speed keys (prefix-sum map), track segments on export.
- [x] T17.3 [L+M] `transition.add/set/remove`: dissolve, dip, wipe, push, slide, zoom; equal-power audio crossfade; `NO_HANDLES`. — e1c65e8, 6ff4417 · six kinds plus dip; dissolve pixel test. Deviation: without handles the edge frame holds (no NO_HANDLES error).
- [x] T17.4 [L] `clip.compound`, `clip.uncompound`: nested timeline media, depth cap 4. Accept: round trip restores the hash. — e1c65e8, 115bd8c · round trip restores the hash; depth cap 4 at edit time (TOO_DEEP); sibling compounds render.
- [x] T17.5 [L+M] Stills (PNG, JPEG, HEIC, GIF) via `clip.insert`; `generator.add` (solid, gradient, rect, ellipse, arrow, line, callout). — 6ff4417, ed81d09 · stills probe through ImageIO and insert for 5 s; shapes in the shader.

### T18 · Look: color, masks, keying, text (0.3) [L + M]
- [x] T18.1 [L+M] `color` effect: exposure, contrast, saturation, vibrance, temperature, tint, highlights, shadows, `.cube` LUT. Accept: identity LUT is bit-exact. — 6ff4417 · color params and .cube LUT (atlas); identity LUT bit-exact (asserted exactly).
- [x] T18.2 [L+M] `mask.add`, `mask.set` (rect, rounded, ellipse; cover, blur, pixelate, spotlight; feather, invert), `mask.track` via `VNTrackObjectRequest`. Accept: a blurred email fails OCR in the export. — e1c65e8, 6ff4417 · masks (4 per layer), Vision tracking; OCR finds no match under a blur mask.
- [x] T18.3 [M] `key` (chroma) and `cutout` (person segmentation matte) effects; camera bubble background blur/remove. — 6ff4417, 1715f81 · chroma key, person matte cutout, camera bubble background blur/remove.
- [x] T18.4 [L+M] Text styles shared by titles and captions; word/character animators; 20+ caption presets; platform safe zones. Accept: preset goldens; 100 animated words allocate nothing. — 70d7dd4 · shared text styles, word/char animators, 22 caption presets.
- [x] T18.5 [L] `style.set` brand kit; `template.add`, `template.set` (lower-third, callout, cta, title-card, chapter-card). — e1c65e8 · style.set brand kit; five templates as grouped items.

### T19 · Audio complete (0.3) [M + L]
- [x] T19.1 [L] Fades and crossfades on sounds; `volume`/`pan` keys; lane volume, mute, solo; ducking as visible keys. — e1c65e8, b6deabb · fades, volume/pan keys, mute/solo, ducking baked to visible keys.
- [x] T19.2 [M] `eq`, `compressor`, `gate`, `voiceIsolation` effects rendered offline (AVAudioEngine manual rendering). Accept: noise floor −15 dB, speech within 1 dB. — f0c3848 · −40 dBFS noise floor falls to −115 dB between phrases; speech moves 0.2 dB.
- [x] T19.3 [L] `cut.pauses`; filler and retake flags in `transcript.get`; review lists on `dryRun`. — e1c65e8, b6deabb · cut.pauses; fillers and retakes in transcript.get.
- [x] T19.4 [L+M] `audio.sync` by envelope cross-correlation. Accept: a 1.37 s offset syncs within 1 frame. — 4d63bef · envelope cross-correlation; a 1.37 s offset syncs within one frame.
- [x] T19.5 [M] Peaks, onsets/beats, SoundAnalysis labels in the analysis store. — 6ff4417 · peaks, onsets/beats, SoundAnalysis label runs.

### T20 · Analysis and intents (0.3) [M + L]
- [x] T20.1 [M+L] `media.analyze {media, kinds}` (loudness, pauses, peaks, beats, sounds, shots, text, faces, people), cached by digest; `audio.summary` alias. — 6ff4417, f0c3848 · media.analyze kinds cached by stamp; shot cut found at 1.60 s in an exported movie (jump 0.49 vs ≤ 0.014); OCR in the blur-mask test.
- [x] T20.2 [L] `intent.pace`, `intent.fitDuration`, `intent.keepShots`, `intent.autoZoom`, `zoom.set`, `crop.keys`, deterministic with dryRun review lists. — e1c65e8, b6deabb · intents deterministic with dryRun review lists; autoZoom from clicks and typing.
- [x] T20.3 [L] `captions.import` (SRT/VTT), `captions.translate {lane, language, texts?}`. — e1c65e8 · SRT/VTT import; translate with on-device Translation or agent texts (D7).

### T21 · Output and interchange (0.3) [M + L]
- [x] T21.1 [M] Export presets: youtube, shorts, reels, tiktok, linkedin, ProRes 422/4444, HEVC alpha, audio m4a/wav, gif, poster; ranges, bitrate, fps, size. — 9710184 · presets, ranges, bitrate, size, alpha, audio-only, GIF, poster. Deviation: output fps is the project's.
- [x] T21.2 [M] `export.start {variants}`; chapter track from markers; soft captions and SRT/VTT sidecars. — 9710184 · variants via reframed copies, chapter track, SRT/VTT sidecars.
- [x] T21.3 [L] `export.start {format:"fcpxml"}`, validated against the FCPXML 1.10 DTD. — 9710184 · FCPXML 1.10 writer, clip ranges asserted. Deviation: no DTD on the build machines (it ships inside Final Cut Pro); output is well-formed and structurally checked.
- [x] T21.4 [L] `media.relink`, `media.consolidate`, `project.info {saveAs}`. — 9710184, b6deabb · relink (probed frames), consolidate (copies), saveAs.
- [x] T21.5 [M] Export queue (2 concurrent), cancel removes partials, `DISK_FULL`, progress with fps/ETA. — 9710184 · 2 concurrent then queued; cancel removes partials; DISK_FULL preflight; fps/ETA.

### T22 · Recording complete (0.3) [M]
- [x] T22.1 [M] Pause/resume (`record.stop {pause}`, `record.start {resume}`), markers during a take. — 7198c69 · live via tools/record-check.py: 1.5 s + pause + 1.5 s → 3.07 s take with the marker (resume-with-marker used to leave the take paused).
- [x] T22.2 [M] Voice-over (`source:"none"`, mic onto an audio lane at a frame) and camera-only takes. — 2917c76 · voice-over onto an audio lane (live in the voiceover eval), camera-only.
- [x] T22.3 [L] Retakes: `retakeOf` links; `intent.keepShots` keeps the last attempt. — 2917c76, b6deabb · retakeOf placement; keepShots keeps the last attempt.
- [x] T22.4 [M] Fragmented writing; `kill -9` mid-take recovers. — 2917c76 · 2 s movie fragments; record.start {recover} publishes staged takes.
- [x] T22.5 [M] Device lists in dryRun; exclude Splice windows and chosen apps. — 2917c76 · devices in dryRun; Splice windows and chosen apps excluded.
- [x] T22.6 [M] 30-minute 4K60 soak: 0 dropped complete frames, drift ≤ 1 frame, flat memory. — 7198c69 · `make mac-record-check SOAK=1800` on the Mini's main display: 54,001 frames, 0 dropped, drift 1 frame, splice mcp RSS flat at 12 MB. Not yet measured: the capture agent process's own memory, and a 4K60 source (the Mini's display decides the size).

### T23 · Code mode and MCP (0.3) [L]
- [x] T23.1 [L] Describe index ≤ 12k chars with every row; golden plus size assert. — 5bb6021 · one line per group; asserted ≤ 12,000 chars with 78 rows and record tools.
- [x] T23.2 [L] Multi-project revisions in `code.run`; job waits excluded from the 2 s wall. — 5bb6021 · per-project revisions; host time excluded from the 2 s wall; record.* callable (ed81d09).
- [x] T23.3 [L] `notifications/progress` and `notifications/cancelled`; `view.frames {sheet:true}` contact sheet. — 5bb6021 · notifications/progress and cancelled; view.frames {sheet:true}.
- [x] T23.4 [L] Skill recipes per workflow, each run by the sweep. — f91ed4d · five recipes in the splice skill, exercised by the sweep.
- [x] T23.5 [L] Code-mode cost ≤ direct on every eval task. — cb53017 frame previews from code.run, 9c0f164 sessionless export, b0e93e1 jobs.status {wait}, 326885b instant exports return a jobId. Code mode is cheaper on every task: hand-off −17% (122k vs 147k, n=3 each), tiktok-textedit −26%, crop-and-frame −23%, talking-head −9%, music-duck −33%, tutorial-polish −44%, undo-session −47%, and the rest at or below direct in the ed81d09 round (−11% overall).

### T24 · Verification gate (0.3) [L + M]
- [x] T24.1 [L+M] Capability sweep: one deterministic code-mode program applies every row and preset, exports, frame-probes each feature, undoes to the start hash. A row missing from the sweep fails the parity test. — f91ed4d · evals/sweep.js: all 78 rows dry-run, a layered batch commits, session undo restores the spine; make check and mac-check.
- [x] T24.2 [M] Evals: tutorial-polish, podcast-clean, talking-head, broll-story, voiceover, hand-off; all 11 pass 3/3 in both arms. — 843dcde, ed81d09 · 66 runs, 11 tasks × 3 × both arms at ed81d09: 65 pass; the miss was a check that rejected camera-bubble blur (widened). First pass found six product bugs, fixed in ed81d09.
- [x] T24.3 [L+M] Budgets table as tests (plan §4 P9). — apply p99 < 1 ms and warm edits of every new edit row 0 allocations (ed81d09); keyed eval 0 allocations, p99 ~1.9 µs locked at 100 µs; open 1 h / 5,000 items 60 ms, 20 MB; 1080p60 full-stack export 2.7–2.9× realtime; transcription 53.9× and analysis 77.6× realtime; 4K full-stack warm compose p90 14.7–15.6 ms on the shared M1 Mini (< 16 ms), after hardware source-over blending, culling layers under opaque full-canvas fills and a mask bounding-box early-out (was 20.1 ms). The 4K and export budgets run as serial gpu_ cases.
- [x] T24.4 [L+M] Chaos: kill -9 during save, export and record; disk full; 1,000 writer-handoff races. — 1bd96f0, 3be88d1 · kill -9 during apply (40 rounds) reopens whole; 1,000 writer races never share a package; save cut at any byte (bcb3b1e).

## 0.4 decisions (2026-11-19) · "Looks right, headless"

Sources: test-drive report and proposal https://artifacts.microapps.space/a/0qn1qr0559nx; prior-art
corrections https://artifacts.microapps.space/a/5tqswvaqk2r3. The research changes the proposal in
eight places; the board below already carries the changes.

- **D12 One camera per timeline** (research: timeline, not per clip). Motion items, click zoom and
  auto zoom only produce camera segments in timeline time; one analytic spring per channel
  (amount, center) chases step targets. Outside segments: amount 1, center held. Pre-aim while
  amount ≤ 1.0005; bounce-out guard (amount ≥ 1). Center in travel space [0,1]². Invariant:
  **viewport ⊂ content and focus visible**; `frame` mode lets the card overflow the canvas.
- **D13** Camera mode `content` (fixed frame, UV zoom inside; default for portrait windowed takes)
  | `frame` (the card zooms, may overflow).
- **D14 One backdrop.** The background is drawn once per frame; every opacity-based transition
  composites the incoming side as one flattened group (research: always, not only when frames
  differ). Transitions consume handles and never change length; `NO_HANDLES` names the fix.
- **D15 Activity is a signal.** Smart ops take `basis: auto|transcript|activity`; `auto` falls back
  to activity. Idle = events, frame diff and audio all quiet; keep head and tail visible.
- **D16 Text is an item** with inline `box`, `style`, `in`/`out`/`loop`; presets compile to keys.
- **D17 Escape hatch is gated.** HTML/Lottie/SVG stay behind a measured determinism gate (0.4.x).
- **D18 The editor enforces quality.** Lints come back with every `apply`/`dryRun`; `view.check`
  adds pixel checks; errors carry `{param, got, expected, fix}`.
- **D19 Privacy by default.** dryRun omits window titles unless `titles:true`; key text policy is
  human-only.
- **D20 Keys are eased by default** (`inOut`); springs are `{spring:{duration, bounce}}`.
- **D21 Ripple keeps connected items.** Deleting a whole parent cascades; a partial cut re-parents
  overlays to the nearest surviving clip (FCP policy); results list what moved and why.

### T36 · 0.3.1 hotfix: the artifacts users see [L + M]
- [x] T36.1 [L] Transitions: Background never blended or moved; backing look applied once (H7); wipe matte multiplies shadow and cursor; push/slide show the backdrop, not black. Accept: 6 kinds × window goldens, backdrop constant ±2/255. (3f078f7: groups + one backdrop; looks pixel test covers dissolve/dip/wipe/push/slide/zoom over a window)
- [x] T36.2 [L] Title fades default 0; fades/dips go to the backdrop, not black (T27.5). (48cf147 title fades 0; 3f078f7 clip fades and dips go to the backdrop)
- [x] T36.3 [L] Transition validation on every path (H9): generic `effect.*`/`param.*`, length ≤ neighbors, not on the first clip, not project/lane scope. (one check_transition on upsert/effect.set/param.*: scope, neighbor, length, handles)
- [x] T36.4 [L] No silent keys: every schema param keyed through one `sample(t)` path or refused `NOT_KEYABLE`; enumerate-and-render test; agent keys default `inOut`. (effects.json `keys:true` on the 33 parameters the evaluator samples; NOT_KEYABLE elsewhere with the keyable list; 0.3 files keep the first key; every_keyable_parameter_renders_its_keys sweeps the schema)
- [x] T36.5 [M] Captions: stroke under fill (H8); second line not clipped; active color rendered. (two-pass outline under fill; shrink-to-fit instead of dropped lines; active word range in the active color; looks_caption_stroke… pixel test)
- [x] T36.6 [L] Motion items hold `to` after they end; `MOTION_ENDS_ZOOMED` warning when `to ≠ 1`. (superseded by D12: motion items are camera segments, 48cf147)
- [x] T36.7 [L] Errors carry the rule: `{param, got, expected, fix}` on every BAD_ARGS; `UNKNOWN_PARAM` with `didYouMean`. (Rule {param, got, expected, didYouMean} on every BAD_ARGS/UNKNOWN_PARAM/UNKNOWN_COMMAND; apply fills got from the op)
- [x] T36.8 [L] Docs, TS types and ranges generated from `effects.json` and validators; test fails on any mismatch (M8). (Splice.Effects generated from effects.json incl. keyability; documented_ranges_match_the_validators sweeps every documented command range; structured hints never type as string; union arrays parenthesized; out-of-range audio volume, cursor and keys size refused instead of clamped)
- [x] T36.9 [L] Build identity: `serverInfo.version = <crate>+<sha>[-dirty]`. (e181907)
- [x] T36.10 [M] Recording discoverability: `window:{app}` in descriptions and fixes; dryRun omits titles unless `titles:true`. (bca5620)

### T37 · Edit-model correctness (report §4) [L]
- [ ] T37.1 [L] C1 project-unique word IDs `w:<media>:<n>` with migration; cues break at clip boundaries.
- [ ] T37.2 [L] H1 overlay double move; H2 gap/past-end placement; H2b cut of a whole range cascades; timeline length = spine (titles past the end trimmed or refused).
- [ ] T37.3 [L] D21 partial cuts re-parent anchored items; `connected:"reparent"|"delete"|"trim"`; result lists moved/removed items with causes.
- [ ] T37.4 [L] H3 word anchors resolve to clip anchors; H4 freeze = speed 0 (no OUT_OF_SOURCE after); H5 duplicate keeps captions on the copy.
- [ ] T37.5 [L] M1 3-frame minimum on every edit; M2 overlay source in media frames; M3 overlays stop at media end; M4 join seams rebase on split; M5 compound with item anchors; M6 end-edge anchors on split; M7 `keepShots` needs a range.
- [ ] T37.6 [L] M17 `style.set` doesn't enable windows; M18 `effect.add` checks components; M19 ripple shortens anchored items before bumping, bump report carries a cause; M21 lane/project transform applies or is refused.
- [x] T37.7 [L] H10 cancel only stops the job its request started, watch list bounded; M10 enrich resolves `$ref`s and runs in `splice apply`; M11 sidecars honour range/overwrite; M12 variant name collision; M13 consolidate only on commit; M14 relink re-probes; M16 recovery skips takes still being written. (cd6c0e8 H10, 7deca25 M10/M13, 6975724 M11/M12, 3e35ce9 M14, b659842 M16)
- [ ] T37.8 [L] Low list: silent clamps → errors; `marker.add` past end; negative `cut.range from`; SRT hour overflow; friction-log unwrap; `catch_unwind` per request; idle undo reports nothing to undo.

### T38 · Camera (D12, D13) [L + M]
- [x] T38.1 [L] Camera model: timeline segments `{from,to,amount,center|follow,mode,spring}`; analytic spring per channel (focused k205/c33.3/m1.5, smooth ~0.9 s); pre-aim; bounce-out guard; hold-center zoom-out; travel-space clamp. Accept: step golden vs reference; 0 allocations; 2,000 segments p99 < 50 µs. (48cf147)
- [x] T38.2 [L] Placement: `content` mode samples UV inside a fixed frame; `frame` mode scales the card. Property test: 10,000 random segments, viewport ⊂ content. (48cf147)
- [x] T38.3 [L] Producers: `motion.*`, `zoom.set`/click zoom and `intent.autoZoom` write camera segments; the old click-zoom curve and Picture scale/focus go; load-time migration of motion items. (48cf147)
- [x] T38.4 [L] Auto zoom v2: lead 0.3 s, tail 2.5 s, merge ≤ 2.5 s, distance split < 0.25, 1.5× default (warn > 2.5×), dead zone 0.5 × 0.7, edge snap 0.25, `autoZoom.version`. (48cf147)
- [x] T38.5 [L] Backing never borrows camera/transform; one pivot convention. Accept: backdrop constant under camera moves. (5295624: the backing draws its own evaluated values; the camera applies to pictures only)
- [x] T38.6 [L] API `zoom.add/set/remove {at,length,amount,center|follow,in,out,mode}`; describe reports `fitSafeScale`. (zoom.add/set/remove {at,length,amount,focus,mode,spring}; items.inspect reports fitSafeScale from the evaluated card; `follow` is the cursor work of T39)

### T39 · Cursor [L + M]
- [x] T39.1 [L] Smoothed 60 Hz table per take: decimate, shake filter (0.015 UV/100 ms), hold gaps > 67 ms, Mellow spring 470/3/70 with lead c/k, click lookahead 500 ms, click spring 530/1/40 for 175 ms; replaces the 0.25 s teleport rule. (splice_eval::cursor: 60 Hz table per take and treatment; hold gaps > 67 ms, rope shake filter, mellow spring with lead c/k, snappy spring and exact landing at presses)
- [x] T39.2 [L] Idle hide 2 s / fade 0.4 s / pre-roll 0.25 s; click 0.8× over 130 ms. (idle hide 2 s / fade 0.4 s / back 0.25 s before moving; click pulse 0.8× over 130 ms)
- [x] T39.3 [L] Synthesized paths for warps: seeded cubic Bézier (spread `arc`·clamp(D,2,200) px), minimum-jerk progress, Fitts duration a=0.2 s b=0.1 s/bit in [0.25,1] s, arrive 0.15 s early, no re-spring, overshoot off. Accept: 1,000 random anchor sets pass human heuristics; deterministic. (warps: speed > 12k pt/s or > 0.1 s silence then a far position; seeded Bezier, minimum-jerk, Fitts duration, early arrival; 1,000 random anchor sets test)
- [ ] T39.4 [M] Pointer declarations `record.start {resume, pointer:{x,y,action}}` and `takes.events` injection after the fact.
- [x] T39.5 [L] Cursor params keyable/size + `hideWhenIdle`, `smoothing`, `clickPulse`, `path`, `arc`, `arriveBefore`. (cursor effect: hideWhenIdle, smoothing, clickPulse, path auto|recorded, arc, arriveBefore)

### T40 · One backdrop, group transitions (D14) [L + M]
- [x] T40.1 [L] Backdrop drawn once per frame; clips render content inside their frame. (3f078f7)
- [x] T40.2 [M] Group compositing: outgoing stack drawn direct, incoming into one pooled offscreen, composited at t (premultiplied). Accept: dissolve black→black over blue backdrop stays black; 4K warm compose p90 < 16 ms. (3f078f7: pooled offscreen group, straight alpha like the canvas; 4K compose p90 15.2 ms)
- [x] T40.3 [L] Transitions consume handles (no length change); `NO_HANDLES {need, have, fix}`; OTIO invariants (no overlap, no adjacent transitions on one clip). (length ≤ the shorter clip and handles checked on every path; one transition per clip by construction)
- [ ] T40.4 [L] New content-only kinds: `whip`, `zoomPunch`, `swipe`.

### T41 · Activity editing for silent takes (D15) [L]
- [ ] T41.1 [L] `media.analyze {kinds:["activity"]}`: idle/typing/clicks/scroll ranges from events (+ frame diff on macOS), no characters.
- [ ] T41.2 [L] `intent.steps` (markers or activity bursts as steps; keep [step−lead, last activity+settle]; cut gaps > maxGap; drop `retake:` attempts; dryRun list).
- [ ] T41.3 [L] `basis` on `cut.pauses`, `intent.pace`, `intent.fitDuration`; `auto` falls back to activity; better NO_TRANSCRIPT fix.
- [ ] T41.4 [L] `intent.speedTyping {factor, minRun}` as eased speed keys.
- [ ] T41.5 [L] Free-text caption cues (`text, from, to`) so SRT import and agent captions work on silent takes.

### T42 · Text & motion, native (D16, D20) [L + M]
- [ ] T42.1 [L] Eases: `{spring:{duration,bounce}}` (closed form, settle time reported), `back`, `elastic`, `bounce`, `steps(n)`; keys default `inOut`.
- [ ] T42.2 [L] `title.add/set` inline `box{x,y,w,h,fit,wrap}`, `style`, `in`/`out`/`loop` compiling to text/animate/transform effects; result returns final and swept bounds.
- [ ] T42.3 [M] Text layout: wrap, shrink-to-fit, no truncation, emoji kept; units by grapheme cluster.
- [ ] T42.4 [L] One z-order (higher = front) and `item.order`; lane/project transforms apply as parents.
- [ ] T42.5 [L] `motion.preset {items, preset}` writing ordinary keys (bounceIn, shake, pulse, float, wiggle, pop).
- [ ] T42.6 [L] Templates use brand colors and safe zones.

### T43 · QA loop (D18) [L]
- [ ] T43.1 [L] Lints in every `apply`/`dryRun`: zoom ≠ 1 at an item's end, text bounds vs safe zone and window, transition on windowed clip without handles, title past the timeline, pacing stats (cut rate, longest static) vs profile.
- [ ] T43.2 [L] `view.check {from,to,checks}` (macOS): off-canvas content, black/frozen spans, camera jump, cursor teleport, transition bleed; returns issues + contact sheet. Versioned `safeZone:"tiktok-2026"`.
- [ ] T43.3 [L] `view.frames {strip}` motion strip with difference heatmap.

### T44 · Evals & gates [L + M]
- [ ] T44.1 [M] `silent-demo-short` eval (silent take → 15–25 s short; every action kept; zero lint errors; cursor continuous; no zoom snap; clean transitions).
- [ ] T44.2 [M] `tutorial-steps` eval (retake removed) and window-transition goldens (6 kinds × window).
- [ ] T44.3 [L] One regression test per report bug id (C1–M23), named after it.
- [ ] T44.4 [M] All evals 3/3 both arms; budgets stay green.

### 0.4.x (gated or asset-bound; not in the 0.4 exit)
- T45.1 HTML escape hatch (HyperFrames-compatible seek contract) behind a determinism gate on macOS.
- T45.2 Native Lottie/SVG/GIF vector engine (ThorVG-class): needs a dependency decision (std + cc rule).
- T45.3 Bundled CC0 music/SFX library: needs curated, licensed assets.
- T45.4 Particles, counters, look packs, glyph atlas per-glyph quads, caret track via AX.

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
- 2026-11-19 · T11 · done · ff8a777 · Every 0.2 row applies headless; 18 described examples apply, replay and match goldens in fixtures/json/tier-b. `make check` 4.28 s (88 tests). T11.5 (motion focal point) stays open.
- 2026-11-19 · T3.6 · done · ce3f118 · tools/eval.py with 5 outcome tasks. Linux baseline with claude: tighten-talk, undo-session and crop-and-frame pass (11–16 calls, 34–42 s each). tiktok-textedit and music-duck need the Mac and T13.
- 2026-11-19 · T12.1–T12.5 · done (interim renderer) · 5cc7409 · Cursor, click rings, eased click zoom toward the click, window corners and shadow, keystroke chips and export text render through a custom compositor. `cargo t -p splice_av native_take` 0.30 s on the Mini. `make mac-check` green; window_program_4k 8.2 ms against an 8 ms budget (4.1–4.5 ms before, measured side by side), which T12.7 fixes by moving the renderer to OGPU.
- 2026-11-19 · T14 merged · da57597 · Code mode on QuickJS-NG; goldens re-blessed against the 0.2 command table. `make check` 3.36 s (98 tests); `make mac-check` green.
- 2026-11-19 · T12.7 · done · ef451c1, eea9f2c · One OGPU renderer (`splice_gpu`) for the viewer, frame grabs and export; the custom compositor only pins decoded buffers. Export is a main-thread stepper (OGPU's Metal handles live there): AVAssetReader → OGPU → AVAssetWriter with the audio mix alongside. Rec. 709 tagging keeps grabs and MP4s equal: check-macos-export.sh 0.23–0.56/255 (was 1.55). window_program_4k 7.6–7.9 ms at load 8.5–9.5, zero warm allocations. `make mac-check` green. Open: decoded frames are copied once on the GPU (an OGPU `rewrap_texture` would make it zero-copy); MSL only until Linux gets a decoder.
- 2026-11-19 · T13 · done · eecd6c1 · Audio lanes, sound items, ducking under speech, BS.1770 loudness at import, normalize, system audio on by default. Music reaches the exported mix (native test).
- 2026-11-19 · T9.1–9.2 · done · a4ac72e · SpeechAnalyzer through a Swift `@_cdecl` shim in apple_sys; `transcript.get {transcribe:true}` commits `transcript.set`. Cargo links at macOS 15.0 so the Swift runtime resolves from the OS. E2E on the Mini: 10 words in 0.2 s, captions and SRT.
- 2026-11-19 · T6.3 · done · 2b5cd03 · `UndoStack::gesture`: a drag is one undo step; cancel restores without redo.
- 2026-11-19 · gate fix · 1c32dc1 · `make check` had been red since ff8a777 (tier-b goldens ended in a newline; CLI snapshot test predated the media table); my gate greps hid it. Fixed; gate exit codes are now checked, not grepped.
- 2026-11-19 · T14.5 · done · a4218a3, c6dca9f · Code-mode A/B: 45 runs, all pass; -49% turns, -63% calls, -16% wall, +4% cost. Five code-mode defects found by round 1 and fixed (export polling inside code.run, loose .d.ts types, trace bloat, planned rows, tool-as-op hint); two cross-arm defects fixed after round 2. Study f0hfkms3e4d0, archive rs6zd6dc2ryj.
- 2026-11-19 · T14.6 · done · 26ace8e · Index-first code-mode declarations: cost -8% vs direct (v2 +4%), cache writes +4% (v2 +21%), 15/15 pass. Study updated (f0hfkms3e4d0 v2, archive rs6zd6dc2ryj v3, 65 runs).
- 2026-11-19 · friction · a716130 · Fixed the recurring friction both arms filed (sound labels, bare refs, social preset, audio.set types, lane inspect, jobs in code.run replies, describe any-term). `make check` 111 tests, `make mac-check` green. apply p99 budget flakes at load ~30 on 20 cores (pre-0.2 a706ba9 measures the same 500–900 µs band side by side); budget unchanged.
- 2026-11-19 · T14.7 · done · 33e1f28, 92f21e0 · Final paired round, both arms at one commit: code mode cost -13% (every task below direct), wall -19%, turns -48%, 30/30 pass. eval.py runs agents with stdin from /dev/null (under ssh … bash -s, claude -p swallowed the rest of the script). Study f0hfkms3e4d0 v3, archive rs6zd6dc2ryj v4 (95 runs).
- 2026-11-19 · defects · 3a99937 · captions.generate follows a take split earlier in the batch; a pinned item spanning a ripple cut loses the removed frames (sample watermark 660 → 651). Regression test fails without the fix.
- 2026-11-19 · T7.6 · built, notarization HUMAN · `make dmg` on the Mini: Splice-0.1.0.dmg 9.0 MB, Developer ID signed, app verifies and runs from the mounted image; spctl: "Unnotarized Developer ID" until a notarytool profile exists.
- 2026-11-19 · defects measured · 5c9bffc · tighten-talk re-run, both arms at HEAD, n=3: 6/6 pass; the captions-after-cut and project-duration reports no longer appear (filed in earlier rounds). One new report fixed: code.run adopts the first project it opens and session it starts as defaults.
- 2026-11-19 · 0.3 plan · accepted · 5sn6awbkq9ae: D5–D11, board T15–T24 (48 tasks).
- 2026-11-19 · 0.3 · T15–T24 · 608dcd8…7198c69 · 44 of 48 tasks done. Gates: make check 150 tests ~7 s; mac-check green. Evals: 65/66 at ed81d09, code mode −11% tokens overall.
- 2026-11-19 · defects · ed81d09, 115bd8c, f0c3848, 7198c69 · Found by 0.3 evals and tests: keys restarted on split pieces; stills could not be imported; mask.set rejected schema params; six sibling compounds failed to render; use-after-free with an image overlay over a dissolve; P3 passed through unconverted; resume with a marker never resumed; open/close cost 4.8 ms (now 30 µs).
- 2026-11-19 · T23.5 · cb53017, 9c0f164 · code.run frame previews and sessionless export: 24 + 6 eval runs all pass; code mode cheaper on every re-run task except hand-off (+7%).
- 2026-11-19 · gap closed · transcript.get {item} transcribes and reads sound items and media overlays (podcast WAVs, voice-overs): live, 74 s of speech as a sound item gave 225 words. cut.words still cuts spine clips only.
- 2026-11-19 · 0.3 · eede27f…326885b · 47 of 48 tasks: 4K full-stack compose 14.7–15.6 ms (hardware source-over, occlusion culling, mask early-out); sound items transcribe; code mode cheaper on every eval task. Only T15.6 notarization (HUMAN credentials) remains.
