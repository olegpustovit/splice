# Building on a loop, and getting better from agent friction

Two loops run on Splice:

1. **The build loop.** A coding agent (Codex by default) works through PLAN.md one task per
   iteration until the milestone exits.
2. **The friction loop.** Agents that *use* Splice over MCP hit problems. Every problem is
   recorded, and the build loop turns it into a test plus a fix in the core. Splice improves from
   its own use.

## 1 · The build loop

Run it with `make loop` (`tools/loop.sh`). Each iteration is a fresh, non-interactive
`codex exec` with the prompt in `skills/splice-loop/SKILL.md`; `AGENT=claude` runs `claude -p`
instead. One iteration does the following:

1. Read AGENTS.md, PLAN.md and the friction backlog (`splice friction list --open`, once T3.3 exists).
2. Pick work. Order:
   1. A red `make check`.
   2. Open friction clusters with ≥ 2 hits, or any entry an agent never recovered from.
   3. The first unchecked PLAN.md task whose dependencies are done and whose machine is available.
3. Write the failing test first (a unit test, a golden, or an MCP transcript in `fixtures/mcp/`).
4. Implement the smallest correct change. Inner loop: `cargo check -p`, then `cargo t -p <crate> [filter]`.
5. Gate: `make check` (Linux). If the task is tagged [M], also run `make mac-check` (see the Mac rules below).
6. Commit with the task ID in the subject (`T2.3: item.trim with ripple`). Tick the task in PLAN.md
   with the short SHA, and append a line to PLAN.md's log: the task, the result and the check wall times.
7. Stop. The next iteration starts clean.

Iterations never skip a red check. If something is still blocked after two honest attempts, the
iteration writes the blocker under the task in PLAN.md (`blocked: …`), leaves the task unticked and
moves on. The loop stops when PLAN.md has no actionable task, or when a `STOP` file exists in the repo root.

## 2 · The friction loop

### At runtime, inside an agent session (self-correction)

These rules live in `skills/splice/SKILL.md`, which every agent using Splice loads:

- Errors are results, not exceptions: `{code, message, fix, current?, valid?}`. The agent follows `fix`
  literally. On `STALE_REVISION` it re-reads, rebases its intent and retries.
- Before a risky batch the agent calls `dryRun:true`. After a batch it looks at the result through
  `view.frames` and `timeline.snapshot`.
- **The agent owns its undo.** No drafts and no approvals. If a result is wrong, it calls
  `history.undo {scope:"last"}`, `{scope:"session"}`, or `{scope:"to", revision}` back to a revision it
  trusted. Undo never destroys anything, so being bold is safe.
- When something was harder than it should be, the agent attaches `friction:"…"` to its next call.

### Recorded automatically, by the server

Every `isError` result is appended to `<project>.splice/friction.jsonl` and to the global state dir
(`~/Library/Application Support/Splice/friction.jsonl`, or `$XDG_STATE_HOME/splice/` on Linux):

```json
{"id":"fr_7k","time":…,"client":"codex 0.x","tool":"edit.apply","args":{…},"code":"BAD_ARGS",
 "message":"…","fix":"…","revision":41,"session":"ses_2x",
 "next":[{"tool":"edit.apply","args":{…},"ok":true}],"recovered":true,"note":null}
```

- `next` holds the following ≤ 3 calls of the same session. `recovered:true` means a later call
  succeeded at the same intent. The diff between the failing args and the args that worked is the
  most valuable signal: it shows what the agent expected the contract to be.
- Agent undos are logged too (`code:"AGENT_UNDO"`): an agent that reverted its own batch either
  misunderstood a command or got a surprising result.
- `friction:"…"` notes are logged with `code:"NOTE"`.

### Turned into fixes, by the build loop

`splice friction list` clusters entries by (tool, command, code) and shows hits, the recovery rate and
the latest example. Each cluster gets exactly one of these outcomes:

| Cause | Fix | Test |
| --- | --- | --- |
| The contract is fine but the fix hint was vague | Rewrite the hint so it names the call and the arguments | A transcript asserting the new `fix` |
| The agent's expectation is reasonable | Accept it: an alias, a default, a coercion, or a new anchor form | A transcript of the failing call, now passing |
| A capability is missing | A registry command or parameter (never a new tool past 19) | A command test + a transcript |
| A core bug | Fix it | A regression test |
| A usage pattern agents keep missing | Add one line to the Pitfalls section of `skills/splice/SKILL.md`, citing the friction ID | An eval task (T3.6) |

`splice friction repro <id>` writes the failing call as a transcript fixture. `splice friction resolve
<id> <sha>` closes the entry. Skill pitfalls are deleted once a core change makes them unnecessary, so
the skill shrinks as the core improves.

### Real projects from recordings

`splice record` and `record.start` turn any screen activity into an editable project. The loop uses
this to make eval seed projects from real recordings (an eval task may name a `record:` seed recipe),
and to capture a repro of a viewer bug as a project an agent can inspect frame by frame.

### Measured by evals (0.2)

`make eval` runs `evals/tasks/*.toml` (a natural-language task, a seed project, and end-state
assertions checked by `splice check`) through `codex exec` / `claude -p`, with the Splice skill and
`splice mcp` headless on Linux. It appends a row per task to `evals/results.tsv`: pass, calls, errors,
recoveries, undos. A drop from the baseline is a red check for the loop.

## 3 · The Mac mini, used sparingly

`snowbear@olegs-mac-mini` is shared. It is building another Rust project in parallel. Rules:

- Use it **only** for [M] tasks and `make mac-check` before committing them. All portable work, and
  every test that can run on Linux, stays on Linux.
- Batch the work: do all the Linux-side work for a mac task, then sync and check once. No watch
  loops, no polling, no repeated full builds to "see if it works".
- Builds there run `nice -n 10` with `CARGO_BUILD_JOBS=2` (the Makefile sets both). Only one
  Splice ssh session at a time. Never `cargo clean` there.
- `make mac-sync` rsyncs the Linux tree (without `target/` and `.git/`) to `~/projects/editor` on the
  Mini. The Mini is never edited by hand, and nothing on it outlives a sync.
- `make mac-check` = sync + `cargo t` of the mac crates + bundle check + `splice --smoke-test`.
  `make mac-run CMD='splice frame --at 30 /tmp/f.png fixtures/projects/sample.splice'` runs one command.
  `make mac-pull F=/tmp/f.png` copies a result back into `target/mac/` to look at.
- GUI checks use `--snapshot` (offscreen). Never take over the Mini's screen.
- Signing over ssh: unlock the passwordless `splice-signing` keychain (T7.2) and never print keychain contents.
- Recording checks are 3 s captures. Never record for longer, and never leave a capture agent running.
- Privacy grants are a **HUMAN** step (`splice permissions --request` on the Mini, then a person
  clicks). The loop marks such a task `- [~] … HUMAN: <what to click>` and moves on; it never tries
  to script System Settings or edit the TCC database.
