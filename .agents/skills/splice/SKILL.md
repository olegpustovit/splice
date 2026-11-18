---
name: splice
description: Edit, export and record video projects with Splice over its MCP server (`splice mcp`). Use whenever a task involves a .splice project, cutting or arranging video, titles, captions, exports or screen recordings through Splice. Covers the call pattern, self-correction, owning undo history, and reporting friction.
---

# Using Splice

Splice is an agent-first video editor. You have the whole editor: every edit a human can make is a
command you can send. There are no drafts and no approvals. Your edits land live, and **you own the
undo history**, so being bold is safe as long as you check your work.

Start the server with `splice mcp` (already configured in `.mcp.json`). It never opens UI. If a human
has the project open in the viewer, they see your edits as you make them.

## The pattern

1. `project.info {path}`: open the project (`create:true` makes one). Keep `projectId` and `revision`.
2. `session.start`: get `sessionId` and a checkpoint revision.
3. Read before writing: `timeline.snapshot`, then `items.inspect`, `transcript.get` and `timeline.find` as needed.
4. `commands.describe` when you are unsure about a command: every command has params and a working example.
5. `edit.apply {sessionId, ifRevision, ops:[…]}`: one batch per intent. Use `$ref` to point at items
   created earlier in the same batch. Send `dryRun:true` first when a batch deletes, ripples or touches
   more than a few items.
6. Check the result: `changes` and `sideEffects` in the response, then `view.frames {at:[…]}` to see it.
7. Wrong? `history.undo {scope:"last"}`, then try again. `{scope:"session"}` reverts all your work;
   `{scope:"to", revision}` restores any earlier state exactly. Undo is itself a revision, so you can
   undo an undo.
8. `export.start` when done. It writes `<project>.splice/exports/…`, refuses to overwrite unless
   `overwrite:true`, and reports through `jobs.status`.

## When a call fails

Errors come back as results: `{code, message, fix, current?, valid?}`.

- Do what `fix` says, literally. It names the call and arguments that work.
- `STALE_REVISION`: a human (or another agent) edited. Re-read with `timeline.snapshot`, re-check your
  intent against the new state, then retry with the new `ifRevision`. Never retry blindly.
- `BAD_ARGS` / `OUT_OF_RANGE`: the `valid` field lists accepted values or ranges.
- `PLANNED`: that command is in the registry but not built yet. Use the alternatives `commands.describe` lists.
- Pass the same `idempotencyKey` when you retry after a transport error.

## Report friction (this is how Splice improves)

Splice logs every error and every undo you make, and its developers fix the causes. Help them: if
something was confusing, surprising or needed several attempts, add `friction:"<one sentence: what you
expected vs what happened>"` to your next call (any tool accepts it). Be specific: name the command
and the argument.

## Time, IDs, anchors

- Time is integer frames at the project `fps` (`project.info`). `at`, `from`, `to`, `length` are frames.
- IDs are stable and prefixed: `clp_` clips, `ovl_` overlays, `lan_` lanes, `mrk_` markers, `med_` media, `w12` words.
- Anchor overlays to *what is said* (`{word, transcriptHash}`) or to items (`{item, edge, offset}`)
  rather than to absolute frames, so they follow later edits.

## Pitfalls

<!-- Maintained by the build loop (docs/LOOP.md). One line each, citing a friction ID; deleted once a
core change makes the line unnecessary. -->
- (none yet)
