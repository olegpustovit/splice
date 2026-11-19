# MCP contract (M1)

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
Splice. Window IDs use `win_<id>`.

## Stdio lifecycle

Clients on `2025-11-25` send `initialize` and then `notifications/initialized`. The compatibility
handshake accepts `2026-07-28` explicitly too. Modern clients can call `server/discover` or send their
first tool request directly with the protocol version and client capabilities in `params._meta`.
See the [MCP stdio specification](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)
and [versioning rules](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).

Send one UTF-8 JSON-RPC message per newline, up to 16 MiB. The server flushes each response, ignores
one-way notifications, and exits at EOF. Unknown methods and malformed RPC requests use JSON-RPC
errors. Invalid tool arguments and editor failures return `isError:true`.

## Tools (19, fixed order)

| # | Tool | Hint | Purpose |
| --- | --- | --- | --- |
| 1 | `project.info` | read | Open by `path` (`create:true` makes a new project) or by `projectId`: fps, size, duration, lanes, offline media, revision |
| 2 | `timeline.snapshot` | read | Whole timeline with resolved frames and labels |
| 3 | `items.inspect` | read | Full properties of IDs, including effects and their resolved values |
| 4 | `timeline.find` | read | Query by kind, text, lane, range, word |
| 5 | `takes.events` | read | Cursor, clicks and keys of a take, in timeline frames (keys redacted by policy) |
| 6 | `transcript.get` | read | Words with IDs and timing; `captions: json\|srt\|vtt` |
| 7 | `audio.summary` | read | Loudness, silences and pauses per clip |
| 8 | `view.frames` | read | PNG frames at frames N from the export composition (what the agent sees = what export makes) |
| 9 | `commands.describe` | read | The command registry: params, examples, status (built or planned) |
| 10 | `session.start` | edit | A session id and a checkpoint revision |
| 11 | `edit.apply` | edit | One atomic batch of registry commands; `dryRun`, `ifRevision`, `idempotencyKey` |
| 12 | `history.list` | read | Revisions with labels, sources, sessions, and what each changed |
| 13 | `history.undo` | edit | `scope: "last"` (the session's last batch) · `"session"` (all of it) · `"to"` + `revision` (restore that exact state). Always a new revision; undo an undo to redo |
| 14 | `ui.show` | view | Playhead, selection, framing in a viewer if one is open; headless: `{applied:false, reason:"headless"}` |
| 15 | `export.start` | edit | Export job, **no approval**. Default path `<project>.splice/exports/<name>-r<rev>.mp4`; refuses to overwrite unless `overwrite:true` |
| 16 | `jobs.status` | read | Progress and result of export, transcription and recording jobs |
| 17 | `jobs.cancel` | edit | Cancel a job |
| 18 | `record.start` | edit | Headless capture of a display, window or area, with mic, system audio, camera and input events, into a project (`create:true` makes one). `dryRun:true` lists sources and permission states without recording; `duration` auto-stops |
| 19 | `record.stop` | edit | Finish recording: the take lands in `takes/` and on the spine as one undoable agent batch (`insert:false` keeps it in media only) |

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
- Business errors are results with `isError:true`: `{code, message, fix, current?, valid?, changedSince?}`.
  The `fix` says exactly what to call or change. If an agent had to guess, the fix hint was wrong;
  that is a friction entry and a bug.

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

Use `at` for a placement calculated from a frame or anchor. Use `anchor` to retain an explicit
item, marker, or transcript relation. Overlapping items can move to a free lane during settle.
Passing an explicit lane turns an overlap into `LANE_CONFLICT`.

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
`audio.summary` returns track settings and labels transcript-derived pauses. Loudness and peaks
are null when the current build cannot measure the audio.

`view.frames` accepts 1–16 exact frame numbers in `at` or a `count` from 1–16 for a filmstrip.
A filmstrip of two or more samples spans the first through last timeline frames. Each result
includes the full-size PNG as a local `file:` resource under the package cache, plus an inline
PNG with a longest edge of up to 640 pixels. Out-of-range frame numbers return `OUT_OF_RANGE`.

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
A window is a `win_<id>` from preflight or an object with `app` and an optional `title` that
identifies one shareable window. An area has a display
and a `rect` with `x`, `y`, `width` and `height` in display-local points. The default is the main
display. Enable `mic`, `systemAudio` and `camera` to add those tracks. Set `events:false` to omit
cursor and input events. `countdown` and optional `duration` are in seconds.

`jobs.status {jobId}` reports elapsed seconds, bytes, dropped frames, and whether the capture is
pending, recording, ready, done, or cancelled. A timed capture stops automatically and becomes
ready. Call `record.stop {jobId}` to stop a manual capture or publish a ready capture. It returns
the new revision and `mediaId` and, when inserted, a `clipId`. The media entry and clip form one
undoable edit. Set `insert:false` at start or stop to save the media without a clip. `jobs.cancel`
discards a take before publication. `still:true` captures a PNG with a three-frame default span
if inserted. A successful still stop also returns its package-relative `output` path.

## Error codes

`BAD_ARGS`, `UNKNOWN_COMMAND`, `NOT_FOUND`, `STALE_REVISION`, `CONFLICT`, `OUT_OF_RANGE`, `INVARIANT`, `PLANNED`,
`WRONG_KIND`, `OUT_OF_SOURCE`, `TOO_SHORT`, `LANE_CONFLICT`, `TRANSCRIPT_CHANGED`,
`LANE_NOT_EMPTY`, `OVERLAP`, `REPLAY_MINTS`, `REPLAY_MEDIA`, `BUSY`, `PERMISSION_DENIED` (project access `off`/`read`, set by the human), `NO_SESSION`, `LOCKED`,
`MEDIA_OFFLINE`, `BAD_MEDIA`, `FORMAT_TOO_NEW`, `FORMAT_UNKNOWN_KEY`, `JOURNAL_CORRUPT`, `EXISTS`, `EXPORT_FAILED`, `FRAME_FAILED`,
`CAPTURE_FAILED`, `CAPTURE_SOURCE`, `CAPTURE_CANCELLED` (CLI),
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
`tools/list` is checked against its own golden.
