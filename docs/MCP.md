# MCP contract (0.4)

`splice mcp` speaks MCP over stdio: JSON-RPC 2.0, newline-delimited and hand-rolled (no SDK). It never
opens UI. When a viewer has the target project open, the server forwards calls to the viewer over
the unix socket in the state dir, so the viewer stays the single writer and the human sees the edit
live. The main spec's "MCP surface" section is the design; `docs/reference/agent.js` is the behavioral
reference implementation.

One stdio connection can open projects in different viewers and projects that run headless.
If a viewer closes during a call, read the current revision with `project.info {path}` before
retrying the edit. The client returns `VIEWER_DISCONNECTED` when the outcome of a sent request
is uncertain. Use `idempotencyKey` on edits that a client may retry.

On macOS, recording calls and `splice permissions` connect to the installed Splice.app over the
same-user socket at `capture/agent.sock` in the state directory. The client starts the app through
Launch Services when needed. The accessory process runs without a window and exits after 60 seconds
of inactivity.

`splice permissions`, `splice record --dry-run` and `record.start {dryRun:true}` report authorization
for `screen`, `microphone`, `camera` and `inputMonitoring` with the signed agent's path, team and
designated requirement match. Only `splice permissions --request [all|screen|microphone|camera|inputMonitoring]`
opens System Settings and starts permission requests. Use `all` to request every missing grant
and open the first missing privacy pane. The JSON reply lists the requested permissions and their
panes. Omit the selector to request just the first missing grant. macOS asks you to approve each
permission separately. Run `splice permissions` after responding to check the updated states.
A capture blocked by a missing grant returns `TCC_DENIED` with `needsHuman:true` and the matching
privacy pane. Preflight lists active displays in points, with `main` as the default display ID.
With screen recording permission, it also lists shareable windows and applications, excluding
Splice. Window IDs use `win_<id>`. Window titles can carry private text (D19), so a dry run lists
them only when you pass `titles:true`; `source:{window:{app}}` needs no title or ID.

## Stdio lifecycle

Clients on `2025-11-25` send `initialize` and then `notifications/initialized`. The compatibility
handshake accepts `2026-07-28` explicitly too. Modern clients can call `server/discover` or send their
first tool request directly with the protocol version and client capabilities in `params._meta`.
See the [MCP stdio specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
and [versioning rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).

`serverInfo.version` names the exact build: `<crate version>+<git short sha>`, with `-dirty` when
the tree had uncommitted changes, or `+unknown` when built outside a git checkout.

Send one UTF-8 JSON-RPC message per newline, up to 16 MiB. The server flushes each response, ignores
one-way notifications, and exits at EOF. Unknown methods and malformed RPC requests use JSON-RPC
errors. Invalid tool arguments and editor failures return `isError:true`. An argument the tool does
not take is refused with `UNKNOWN_PARAM`, its `param` and the closest accepted name as `didYouMean`
(it is never silently ignored). A tool call that hits a Splice bug returns `INTERNAL` as an error
result, is logged to the friction log, and the server keeps serving.

## Tools (20, fixed order)

| # | Tool | Hint | Purpose |
| --- | --- | --- | --- |
| 1 | `project.info` | read | Open by `path` (`create:true` makes a new project) or by `projectId`: fps, size, duration, lanes, offline media, revision |
| 2 | `timeline.snapshot` | read | Whole timeline with resolved frames and labels |
| 3 | `items.inspect` | read | Full properties of IDs (`ids`, or `items`), including effects and their resolved values; an `fx_` ID returns that effect with its `owner` |
| 4 | `timeline.find` | read | Query by kind, text, lane, range, word |
| 5 | `takes.events` | read | Cursor, clicks and keys of a take, in timeline frames (keys redacted by policy) |
| 6 | `transcript.get` | read | Words with IDs and timing; `captions: json\|srt\|vtt` |
| 7 | `media.analyze` | read | Per clip or media, by `kinds`: loudness, pauses, activity, peaks, beats, sounds, shots, text, faces, people, matte. Cached by file stamp; long media runs as a job. `audio.summary` stays an alias |
| 8 | `view.frames` | read | PNG frames at frames N from the export composition (what the agent sees = what export makes); `sheet:true` returns one contact sheet; `strip:true` adds a heatmap of what changed between consecutive frames |
| 9 | `view.check` | read | Problems a few frames miss, with spans and fixes: camera jumps and off-canvas pictures (every platform), black or frozen spans and backdrop bleed in transitions (macOS, rendered at 5/s, with a contact sheet) |
| 10 | `commands.describe` | read | The command registry: params, examples, status (built or planned) |
| 11 | `session.start` | edit | A session id and a checkpoint revision |
| 12 | `edit.apply` | edit | One atomic batch of registry commands; `dryRun`, `ifRevision`, `idempotencyKey` |
| 13 | `history.list` | read | Revisions with labels, sources, sessions, and what each changed |
| 14 | `history.undo` | edit | `scope: "last"` (the session's last batch) · `"session"` (all of it) · `"to"` + `revision` (restore that exact state). Always a new revision; undo an undo to redo. With nothing left to undo it returns `NOTHING_TO_UNDO` and changes nothing |
| 15 | `ui.show` | view | Playhead, selection, framing in a viewer if one is open; headless: `{applied:false, reason:"headless"}` |
| 16 | `export.start` | edit | Export job, **no approval**: presets, `variants`, `range`, chapters, caption sidecars, `format:"fcpxml"`. Default path `<project>.splice/exports/<name>-r<rev>.mp4`; refuses to overwrite unless `overwrite:true` |
| 17 | `jobs.status` | read | Progress and result of export, transcription and recording jobs |
| 18 | `jobs.cancel` | edit | Cancel a job |
| 19 | `record.start` | edit | Headless capture of a display, window or area, with mic, system audio, camera and input events, into a project (`create:true` makes one). `dryRun:true` lists sources, devices and permission states without recording; `duration` auto-stops. Also voice-overs, retakes, `resume` with `marker`, `exclude`, `recover` |
| 20 | `record.stop` | edit | Finish recording: the take lands in `takes/` and on the spine as one undoable agent batch (`insert:false` keeps it in media only); `pause:true` pauses instead |

Changes from the prototype's 17 tools: no export approval (`NEEDS_APPROVAL` and `DENIED` are gone),
`history.undo` gains `scope:"to"`, `project.info` opens by path, and the recording pair is added.
Recording runs in Splice.app's capture agent (LaunchServices-launched, accessory, no window), so
privacy grants belong to the stably signed `~/Applications/Splice.app`, never to the terminal or MCP
client that spawned `splice mcp` (PLAN.md T8).

Stay under 20 tools in the default profile: new capabilities become `edit.apply` commands or parameters, not tools. Code mode is a separate profile, `splice mcp --code` (PLAN.md D3, T14).

## Every call

- Project tools use `projectId`. Use `path` with `project.info` to open a package or with
  `record.start` to record into one. A `projectId` identifies an open package in the current server
  process. Pass `sessionId` and `ifRevision` for edits. Recording job calls can use the session
  and current revision of the job's project.
  `commands.describe` works before a project is opened.
- Any tool accepts an optional `friction` string: a note from the agent about what was hard. It is
  logged and has no effect (docs/LOOP.md).
- Results carry `structuredContent` matching the tool's `outputSchema` and a text block. The text
  mirrors success as compact JSON and summarizes a business error with its fix.
- Business errors are results with `isError:true`:
  `{code, message, fix, param?, got?, expected?, didYouMean?, current?, valid?, changedSince?}`.
  When an argument is at fault the error names the rule it broke (D18): `param`, the value `got`
  (filled from the failing op), and what is `expected`; an unknown argument, effect parameter or
  command is `UNKNOWN_PARAM`/`UNKNOWN_COMMAND` with the closest known name in `didYouMean`.
  Out-of-range values are refused, never clamped. The `fix` says exactly what to call or change. If an agent had to guess, the fix hint was wrong;
  that is a friction entry and a bug.

## Lints (0.4, D18)

Every `edit.apply` result, dry run or committed, carries `lints` and `pacing` for the project the
batch leads to. A lint is a warning, never a refusal: `{code, severity:"warning", message, items,
fix, ops?}`, where `ops` is a batch that fixes it when one exists. Codes: `OUTSIDE_SAFE_ZONE` (a title's box leaves the versioned 9:16 safe zone
`tiktok-2026`: top 8.4 %, bottom 18.75 %, left 8.4 %, right 11.2 % of the canvas),
`TEXT_OVER_WINDOW` (a title covers over 30 % of itself with the recorded window), `ZOOM_OVERFLOW`
(a frame-mode zoom past `fitSafeScale`), `LONG_STATIC` (over 8 s without a visual change on a
vertical short). `pacing` is `{cutsPerMinute, longestStatic, safeZone}`.

## Command descriptions

`commands.describe {id}` returns the argument descriptions and an example operation for the
sample project. A leading `!` in a description marks a required argument. With `query` it
searches command IDs, titles and groups without regard to case.

The `cut` field gives the command's release tier. `available:true` means the command applies
today (every 0.1 and 0.2 row). The 20 registry-only commands carry `available:false`,
`planned:true` and return `PLANNED` if submitted as edits. In `edit.apply`, an operation can bind a result with `ref:"$cut"`. Later operations
can use `"$cut"` for its primary item and `"$cut.right"` or `"$cut.new"` for its first created item.

## Batch results

An edit batch reports direct `changes`, implicit `sideEffects`, and a `diff` with before and
after timeline ranges. An operation error includes its zero-based index when the operation can
be identified. A dry run uses the same validation, resolves references, and previews the IDs that
would be minted. It leaves the project and its next ID serial untouched.

Every side effect carries a `cause`: `cascade` (its parent was deleted), `follow` (it followed
its content through a split, trim or join), `reparent`, `cut`, `spineEnd`, `words` (its caption
words left the timeline or were re-captioned), `laneDeleted`, `replaced` (a re-run of `intent.autoZoom` replaced its earlier zooms), or `overlap` with `with` naming the
item that held the lane.

Use `at` for a placement calculated from a frame or anchor. Use `anchor` to retain an explicit
item, marker, or transcript relation. A frame in a gap between clips keeps its position, anchored
to the end of the clip before it (or the start of the first clip). Overlapping items can move to a
free lane during settle. Passing an explicit lane turns an overlap into `LANE_CONFLICT`.

Cuts keep connected items (D21). `cut.range`, `cut.words` and `item.delete` take
`connected:"reparent"|"delete"|"trim"`. Unset, an item on a piece of a clip that partly survives
moves to the nearest surviving clip at the cut point (`cause:"reparent"`), and a clip removed
whole takes its items with it (`cause:"cascade"`). `delete` removes the items on cut frames
(`cause:"cut"`); `trim` keeps the part of each item that runs past the cut, on the same content.
A ripple cut also shortens any item that runs into the cut, so the items after it close up
instead of bumping to new lanes. `cut.range` refuses a negative `from`.

`style.set` colors recolor the project's window (or, without one, the windows that exist) and
never turn a window on. `effect.add` of `camera`, `cursor` or `keystrokes` on a clip needs that
take component (`WRONG_KIND` names it). A `transform` goes on clips and items; lane and project
transforms are refused until the evaluator draws them as parents.

The timeline ends where the spine ends. An item that would run past the last clip is trimmed
there; one placed at or past the end (fewer than three frames before it) is refused with
`PAST_END`; one an edit leaves past the end is removed (`cause:"spineEnd"`).

A committed edit advances the revision after the journal is synced. A stale write returns the
current revision and up to eight intervening changes. A retry of the original operation batch
with its session's `idempotencyKey` returns the original revision and result with `replayed:true`.

Use `history.undo` with `scope:"last"` or `"session"` to remove edits from that session and replay
the remaining edits in order. A later edit that depended on a removed object appears in
`skipped`. Use `scope:"to"` with a retained revision to restore its exact project content.
Agents cannot restore a revision that would change the project's access level.

## Headless media reads

`takes.events` reports timeline frame numbers and source frame numbers. Key events expose their
indexes, visibility and categories. Captured characters and raw key codes stay in the take.
`media.analyze {clip|media, kinds}` returns track settings and transcript-derived pauses, plus the
requested native kinds: `loudness`, `peaks`, `beats` (onsets and tempo), `sounds` (SoundAnalysis
labels as runs), `shots`, `text` (Vision OCR with boxes), `faces`, `people` and `matte` (a person
matte movie for `cutout`). `activity` (portable, D15) reads a screen take's event log and transcript
and returns `{idle, typing, clicks, scroll, busy}` as `{from, to}` timeline ranges: `busy` merges
every input event and spoken word within 0.5 s; `idle` is a stretch with no input and no speech for
at least 2 s, inset 0.25 s at each end so its head and tail stay visible; typing runs join keys
within 1 s (command, control and option keys are shortcuts, not typing). Typed characters are never
read or returned (D19). A clip that is not a take is `WRONG_KIND`. Results are cached in `<package>/analysis/<media>/`, valid while the
file's size and mtime match. Media over 10 minutes (or `async:true`) analyze as a job; poll
`jobs.status`. A missing file is `MEDIA_OFFLINE`. `audio.summary` is an alias with no kinds.
`transcript.get` also lists `fillers` and `retakes` (repeated phrases) for `cut.words`. Pass `item` (a sound item or media overlay: voice-overs, podcast tracks) instead of `clip` to transcribe or read that item's media; its words map to the timeline through the item's start.

`transcript.get {transcribe:true, sessionId}` recognizes a spine clip's speech on device
(SpeechAnalyzer, macOS 26+; the first run downloads the language model) and commits the words as one
`transcript.set` batch in that session, so `history.undo` removes them. It then answers like a plain
`transcript.get`. Other platforms return UNSUPPORTED_PLATFORM.

`view.frames` accepts 1–16 exact frame numbers in `at` or a `count` from 1–16 for a filmstrip.
A filmstrip of two or more samples spans the first through last timeline frames. Each result
includes the full-size PNG as a local `file:` resource under the package cache, plus an inline
PNG with a longest edge of up to 640 pixels. Out-of-range frame numbers return `OUT_OF_RANGE`.
`sheet:true` tiles the frames (360-pixel thumbnails in a near-square grid) into one PNG: one
inline image and a `sheet` file URI.

## Silent takes (D15)

Screen takes often have no speech. The smart cuts `cut.pauses`, `intent.pace` and
`intent.fitDuration` take `basis: "auto"|"transcript"|"activity"`. `auto` (the default) measures
pauses between transcript words when the timeline has any and otherwise between bursts of a
take's recorded activity; `transcript` refuses a timeline without words with `NO_TRANSCRIPT`,
whose fix names `basis:"activity"`. On activity, `cut.pauses` cuts idle stretches of 2 s or more
keeping 0.25 s at each end, `intent.pace` leaves at most 1 s between bursts, and
`intent.fitDuration` cuts idle and then the shortest bursts. Only gaps between activity in one
clip are cut, so each clip's head and tail stay.

`intent.steps {item}` trims a demo to its steps. Steps start at the clip's markers, or at bursts
after a quiet stretch over `maxGap` when it has none. Each step keeps `lead` frames before it
(0.4 s) through its last activity plus `settle` (1 s); quiet stretches over `maxGap` (1.5 s)
inside a step are cut. A marker named with `retakePrefix` (`retake:`) drops the attempt since the
step before it and restarts that step. The change summary lists every step with the frames it
keeps, so a dry run is the review list.

`intent.speedTyping {item, factor, minRun}` turns typing runs of at least `minRun` (1 s) into eased
`speed.rate` keys up to `factor` (3, 1.25–16) and shortens the clip so it shows the same source
frames (never past its media). A clip with a speed ramp is `RETIMED`. Items anchored inside a
ramped clip resolve at its constant rate.

`caption.add {text, from, to, lane?}` adds a free-text caption cue that needs no transcript, and
`captions.import` keeps an SRT or WebVTT cue without words under it as free text, so captions work
on silent takes. Free cues keep their own timing (trim and move them like titles) and retype with
`caption.set {text}`; word operations (`caption.split`, `caption.merge`, `words`) refuse them with
`WRONG_KIND`. `transcript.get {captions}` and export sidecars list them with `words: []`.

## Inputs edit.apply fills

Some 0.3 commands need inputs the core cannot read (it never opens media). `edit.apply`, `splice apply`
and viewer edits fill them the same way before the batch applies, so the journal records plain ops
that replay without media. A `$ref` to an object the same batch creates (`"item":"$cut.right"`,
`"mask":"$mask"`) is read from the batch staged up to that op; an item the fill cannot read (an
overlay for clicks or a cursor crop) is refused with `WRONG_KIND` and `param:"item"`. The fills:
`intent.autoZoom` (clicks and typing bursts from the take's events; it writes `zoom` items on a
camera lane), `intent.steps` (`activity`: the take's busy source ranges), `intent.speedTyping`
(`typing`: its typing runs), the smart cuts `cut.pauses`, `intent.pace` and `intent.fitDuration`
(`activity: [{media, from, to}]` for every take on the spine, when `basis` is `activity`, or
`auto` on a timeline without transcript words; activity as in `media.analyze`, never characters), `crop.keys {follow:"cursor"|"face",
aspect}` (keys every half second), `media.consolidate` (copies files into `media/` only when
the batch commits, never on a dry run or a stale or failing batch; an existing copy is reused only
when its bytes match the source), `mask.track` (Vision object tracking), `audio.sync` (envelope
cross-correlation offset) and `captions.translate` without `texts` (on-device Translation).
Pass the inputs yourself to skip the native step.

`media.relink` always probes the new file (the journal keeps the probe, so replay never opens it):
fps, size and audio tracks follow the file, and a file of a different length or kind is refused with
`MEDIA_MISMATCH` and the broken rule (`param:"path"`, or `param:"frames"` when the `frames` you
passed disagrees with the probe) unless `force:true`. A missing file is `MEDIA_OFFLINE`.

On Linux, rendering, export and capture return `UNSUPPORTED_PLATFORM`. A job ID with no retained
job returns `NOT_FOUND`. Headless `ui.show` returns `{"applied":false,"reason":"headless"}`.
In the 0.1 viewer, `ui.show` moves the playhead using a frame or an item, marker, or word anchor.
`range.from` seeks to the start of a range. The view tool reports the live playhead and leaves the
project revision unchanged.

## Export jobs

`export.start` encodes the revision identified by `ifRevision`. Omit `preset` for `social-hevc`
or pass `social-h264` for H.264. The default output is `exports/<project-name>-r<rev>.mp4` inside
the project package. An explicit MP4 path can be absolute or relative to that package.
Pass `overwrite:true` to replace an existing movie after encoding succeeds.
By default, publishing a new filename uses a hard link on the destination volume.

`preset` is one of `social-hevc`, `social-h264`, `youtube`, `shorts`, `reels`, `tiktok`,
`linkedin`, `prores-422`, `prores-4444`, `hevc-alpha`, `audio-m4a`, `audio-wav`, `gif` or
`poster` (a PNG still). `from`/`to` (whole frames from 0 to the timeline's end; anything else is
refused with the broken rule, never clamped) or `marker` export part of the timeline; `bitrate`
overrides the preset (the frame rate is the project's). `variants:[{aspect, preset?}]` exports
reframed copies, one job each; variants that would write the same file (same size and container)
are refused, and if one variant cannot start, the ones already started are cancelled. Markers
become a chapter track; `captions:"srt"|"vtt"` writes a sidecar with the cues inside the exported
range, timed from its start, and replaces an existing sidecar only with `overwrite:true`; burned-in
captions stay as they are. Without a `sessionId` the export opens its own session, and closes it
again when the export fails to start. `format:"fcpxml"` writes FCPXML 1.10 for Final
Cut Pro instead of encoding. Two exports encode at once and later ones queue (`queued` in
`jobs.status`, with `fps` and `eta` while running). A full disk fails early with `DISK_FULL`, and
cancelling removes the partial file.

Clients that pass `_meta.progressToken` on `export.start` receive `notifications/progress`
(`progress` 0–1, `total` 1) as frames encode. `notifications/cancelled` with the request's ID
cancels the job that request started (never one a `jobs.status` or `jobs.cancel` call only names).

The server publishes the finished MP4 even when the client makes no status calls. Use
`jobs.status {jobId}` for progress and the final path. `jobs.cancel {jobId}` requests cancellation,
and `jobs.status` reports `cancelled` once the encoder stops. Jobs are retained by the connection
that started them. A clean end to MCP input lets active exports finish.

## Recording jobs

Use `record.start` with an open `projectId`, `sessionId` and `ifRevision`, or pass a `path` ending
in `.splice`. With `path`, use `create:true` for a new project. The server opens a session if you
omit `sessionId` and `ifRevision` with `path`. The start receipt identifies the project, session,
and job. Include the session in later edits or undo calls.

Pass one `source` containing `display`, `window`, or `area`. A display is `"main"` or a display ID.
A window is `{app:"Safari"}` (an app name or bundle ID; add `title` when the app has several
windows) or a `win_<id>` from preflight. Selector errors name this form. An area has a display
and a `rect` with `x`, `y`, `width` and `height` in display-local points. The default is the main
display. Enable `mic`, `systemAudio` and `camera` to add those tracks. Set `events:false` to omit
cursor and input events. `countdown` and optional `duration` are in seconds.

`jobs.status {jobId}` reports elapsed seconds, bytes, dropped frames, and whether the capture is
pending, recording, ready, done, or cancelled. A timed capture stops automatically and becomes
ready. Call `record.stop {jobId}` to stop a manual capture or publish a ready capture. It returns
the new revision and `mediaId` and, when inserted, a `clipId`. The media entry and clip form one
undoable edit. Set `insert:false` at start or stop to save the media without a clip. `jobs.cancel`
discards a take before publication. If publishing fails after the capture stopped (a full disk,
a stale revision), the take stays staged and the job remembers the stop: fix the cause and call
`record.stop` again to publish it. `still:true` captures a PNG that inserts as a 5-second
still. A successful still stop also returns its package-relative `output` path.

`record.stop {jobId, pause:true}` pauses a take; `record.start {resume:jobId}` continues it into
the same take, and `marker:"Name"` with `resume` drops a marker (markers land on the timeline at
publish). `voiceover:true, at` records only the microphone onto a new audio lane at frame `at`.
`camera:true, screen:false` is a camera-only take. `retakeOf:"clp_…"` places the take right after
an earlier one, named after it. `exclude:["com.app.id"]` hides apps (Splice's own windows are
always excluded). Takes write fragmented movies every 2 s; after a crash, `record.start
{recover:true}` publishes what was staged. It skips a take the capture agent is still recording or
holding for another session (or when the agent cannot say).

## Code mode profile

`splice mcp --code` lists exactly two tools: `code.run` and `commands.describe`. The other tools
are reached from JavaScript, so a whole read–edit–check loop is one round trip.

`code.run {code, projectId?, sessionId?}` runs `code` as a function body (top-level `return` sets
the result) in a fresh QuickJS-NG runtime (`crates/quickjs_sys`): no modules, files, network,
timers or clock. `Date.now()` and `new Date()` are fixed at 2026-01-01T00:00:00Z and
`Math.random` is seeded, so runs replay identically. The reply is
`{result, logs, calls:[{tool, ok, ms}], revisions:{from, to}|null, error?:{code, message, fix, line}}`
with `isError:true` when `error` is set; logs and calls are kept on failure. `revisions` covers
the first project the code touched, with `projects:{prj_…:{from, to}}` for any other; `line` is
the line in `code`.

Inside the code, the global `splice` has:

- `splice.<group>.<name>(args)` for every default-profile tool, e.g.
  `splice.project.info({path})`, `splice.edit.apply({...})`, `splice.timeline.snapshot()`.
  `splice.call(name, args)` is the same by name. Calls are synchronous and return the tool's
  structured result; `view.frames` returns paths, and the code.run reply carries its preview images (at most 6 per run, 640 px). `projectId` and
  `sessionId` default to the ones passed to `code.run` (`splice.projectId`, `splice.sessionId`).
- `splice.op.<group>.<name>(fields)` builds a registry op, e.g. `splice.op.item.split({item, at})`
  is `{cmd:"item.split", item, at}`.
- `splice.describe(query?)` returns the TypeScript declarations below, filtered by name.

A business error throws `SpliceError` (`code`, `message`, `fix`, `valid?`, `current?`), which code
can catch. Uncaught, it becomes the reply's `error` with the tool's own code and fix. JavaScript
errors use `CODE_ERROR`. Limits per run, each with its own code: 64 KB of source (`CODE_TOO_LARGE`),
2 s of JavaScript excluding time inside tool calls (`CODE_TIMEOUT`), 32 MB heap (`CODE_MEMORY`),
1 MB stack (`CODE_STACK`), 200 tool calls (`CODE_CALLS`), 32 KB of result JSON (`CODE_OUTPUT`);
logs are truncated at 32 KB. Calling `code.run` from code throws `UNKNOWN_TOOL`; `record.start` returns a job at once, so code can start, mark and stop takes.

Without a query the declarations are an index (asserted at most 12,000 characters): every command
by group, and each tool's signature with the first sentence of its description.

`commands.describe {format:"ts", query?}` returns `{format:"ts", text}`: TypeScript declarations
generated from the command registry and the tool schemas, with an interface per command
(`Splice.op.ItemSplit`), the `Splice.Op` union, builders and tool signatures. Optional fields are
optional (`?`); `null` appears only where it is a value (e.g. `item.set {name: null}` clears the
name). `fixtures/mcp/code/splice.d.ts` is the golden.

```js
// code.run {"code": "..."}
const p = splice.project.info({path: "/Users/me/Movies/demo.splice"});
const s = splice.session.start({projectId: p.projectId, agent: "claude"});
const e = splice.edit.apply({projectId: p.projectId, sessionId: s.sessionId, ifRevision: p.revision,
  ops: [splice.op.item.set({items: ["clp_take"], name: "Intro"})]});
const t = splice.timeline.snapshot({projectId: p.projectId});
return {revision: e.revision, clips: t.spine.length};
```

## Error codes

`BAD_ARGS`, `UNKNOWN_COMMAND`, `UNKNOWN_PARAM`, `INTERNAL`, `NOT_FOUND`, `STALE_REVISION`, `CONFLICT`, `OUT_OF_RANGE`, `INVARIANT`, `PLANNED`,
`WRONG_KIND`, `OUT_OF_SOURCE`, `TOO_SHORT`, `LANE_CONFLICT`, `TRANSCRIPT_CHANGED`,
`LANE_NOT_EMPTY`, `OVERLAP`, `REPLAY_MINTS`, `REPLAY_MEDIA`, `BUSY`, `PERMISSION_DENIED` (project access `off`/`read`, set by the human), `NO_SESSION`, `LOCKED`,
`MEDIA_OFFLINE`, `BAD_MEDIA`, `FORMAT_TOO_NEW`, `FORMAT_UNKNOWN_KEY`, `JOURNAL_CORRUPT`, `EXISTS`, `EXPORT_FAILED`, `FRAME_FAILED`,
`CAPTURE_FAILED`, `CAPTURE_SOURCE`, `CAPTURE_CANCELLED` (CLI), `DISK_FULL`,
0.4: `UNKNOWN_PARAM` (with `didYouMean`), `NOT_KEYABLE` (keys on a parameter the evaluator does not
animate; the fix lists the keyable ones, also marked `keys:true` in `Splice.Effects`),
`NO_ACTIVITY` (an activity basis or `intent.steps` with no recorded input on the timeline or
inside the clip), `NO_TYPING` (`intent.speedTyping` found no run of at least `minRun`).
0.3 edits: `NO_HANDLES` (roll, slip, slide; a transition whose clips lack the source it consumes),
`NO_NEIGHBOR`, `NOT_ADJACENT`, `NOT_JOINABLE`,
`NOT_JOINED`, `NOTHING_TO_RESTORE`, `NOTHING_TO_UNDO`, `NO_TRANSITION`, `TOO_LONG`, `FIT_IMPOSSIBLE`, `NEEDS_TEXT`,
`NO_CLICKS`, `NO_FACES`, `NOT_TRACKED`, `NO_CAPTIONS`, `NO_TRANSCRIPT`, `MARKER_EXISTS`,
`MEDIA_MISMATCH`, `LOUDNESS_UNKNOWN`, `TOO_DEEP`, `RETIMED`,
`UNSUPPORTED_PLATFORM` (Linux: export, frames, recording), `TCC_DENIED` (recording without a grant;
the fix names the System Settings pane and `splice permissions --request`; `needsHuman:true` when only
a person can click), `IDENTITY_MISMATCH` (the running bundle is not the installed, Developer ID–signed
Splice.app, so grants would not apply).

## Tests

Run the fixtures in `fixtures/mcp/*.jsonl` with `cargo t -p splice_mcp transcript`.
A `> ` line sends one wire request. Its following `< ` line specifies the expected reply.
Notifications have only a request line. Blank lines start a new connection. Tool fixtures use
the shared initialization prelude, and the conformance fixture supplies its own handshakes.

Expectations can name a subset of object fields. Arrays match in order and in full. The string
`"…"` matches any present value. Failures report the fixture line and the JSON field that differs.

The tool tests bind `$sample` and `$new` in request paths to disposable packages. A project-bound
friction repro can name a sibling `<fixture-stem>.splice` seed, whose copy becomes `$project`.
`tools/list` is checked against its own golden. `fixtures/mcp/code/code.run.jsonl` runs against the code
profile; its `$sample` is replaced as text because the path sits inside the JavaScript.
