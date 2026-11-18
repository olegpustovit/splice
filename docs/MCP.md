# MCP contract (M1)

`splice mcp` speaks MCP over stdio: JSON-RPC 2.0, newline-delimited and hand-rolled (no SDK). It never
opens UI. When a viewer has the target project open, the server forwards calls to the viewer over
the unix socket in the state dir, so the viewer stays the single writer and the human sees the edit
live. The main spec's "MCP surface" section is the design; `docs/reference/agent.js` is the behavioral
reference implementation.

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
| 18 | `record.start` | edit | (0.2) Headless screen, window, mic, camera and event capture into the project |
| 19 | `record.stop` | edit | (0.2) Finish recording; the take lands as a clip with components |

Changes from the prototype's 17 tools: no export approval (`NEEDS_APPROVAL` and `DENIED` are gone),
`history.undo` gains `scope:"to"`, `project.info` opens by path, and the recording pair is added.
Stay under 20 tools: new capabilities become `edit.apply` commands or parameters, not tools.

## Every call

- `projectId` (or `path` on `project.info`); writes also need `sessionId` and `ifRevision`.
- Any tool accepts an optional `friction` string: a note from the agent about what was hard. It is
  logged and has no effect (docs/LOOP.md).
- Results: `structuredContent` matching the tool's `outputSchema`, plus a short text mirror.
- Business errors are results with `isError:true`: `{code, message, fix, current?, valid?, changedSince?}`.
  The `fix` says exactly what to call or change. If an agent had to guess, the fix hint was wrong;
  that is a friction entry and a bug.

## Error codes

`BAD_ARGS`, `NOT_FOUND`, `STALE_REVISION`, `CONFLICT`, `OUT_OF_RANGE`, `INVARIANT`, `PLANNED`,
`BUSY`, `PERMISSION_DENIED` (project access `off`/`read`, set by the human), `NO_SESSION`, `LOCKED`,
`MEDIA_OFFLINE`, `FORMAT_TOO_NEW`, `FORMAT_UNKNOWN_KEY`, `JOURNAL_CORRUPT`, `EXISTS`,
`UNSUPPORTED_PLATFORM` (Linux: export, frames, recording), `TCC_DENIED` (recording without a grant;
the fix names the System Settings pane).

## Tests

`fixtures/mcp/*.jsonl` transcripts: alternating `>` request and `<` expected result lines, `"…"` as
a wildcard. Each tool has one; every fixed friction entry adds one. `tools/list` is a golden.
