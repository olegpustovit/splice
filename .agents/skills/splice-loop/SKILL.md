---
name: splice-loop
description: One iteration of the Splice build loop. Picks the next task from PLAN.md or the friction backlog, implements it test-first under rays rules, gates it, commits, and logs it. Use when running `make loop` or when asked to "do the next task" in the Splice repo.
---

# One build-loop iteration

You are building Splice: an agent-first, headless-core video editor in Rust. Do exactly **one** unit
of work, leave the tree green and committed, then stop.

## 0 · Orient (read, don't skim)

- `AGENTS.md`: the rules. They are not optional: `cargo t` only, zero-alloc checks, `unsafe` only in
  `*_sys`, std + cc only, no async, macros or generics in product crates.
- `PLAN.md`: the milestone, cut lines and the board. `docs/LOOP.md`: this loop and the Mac rules.
- `docs/FORMAT.md`, `docs/MCP.md`, `docs/KIT.md`: the contracts you implement.
- `docs/reference/core.js` and `agent.js`: the prototype. Port its *behavior* (commands, apply, Store,
  tools); write idiomatic Rust under our rules, not a transliteration.
- `git log --oneline -15` and the last lines of PLAN.md's log: what just happened.

## 1 · Pick

In order:
1. `make check` is red → fix that.
2. Open friction (`splice friction list --open`, once T3.3 exists): any unrecovered entry, or a cluster
   with ≥ 2 hits → fix it as in docs/LOOP.md §2.
3. The first unchecked PLAN.md task in the current cut whose dependencies are done. Do not
   start a later cut while the current one has actionable tasks.

[M] tasks need the Mac mini. Do all the Linux-side work first, then use the Mac once, as docs/LOOP.md §3
says. The Mini is shared: no polling, `nice`, 2 jobs.

## 2 · Build

1. Write the acceptance test first and see it fail (`cargo t -p <crate> <filter>`).
2. Implement the smallest correct change. Inner loop: `cargo check -p <crate>`.
3. Add a zero-allocation check where the task touches a steady-state path.
4. Mark any shortcut with `ponytail:` and its upgrade trigger.

If you are blocked after two honest attempts, write `blocked: <why, what you tried, what would unblock
it>` under the task in PLAN.md, commit that note, and stop.

## 3 · Gate and commit

1. `make check` (gate + fmt + clippy + `cargo t`). [M] tasks also need `make mac-check`.
2. `git diff --check`. Never commit secrets, signing material or `target/`.
3. Commit: `T2.3: item.trim with ripple` (the task ID first). Tick the task `[x] … (abc1234)`.
4. Append to PLAN.md's log: `- 2026-11-18 · T2.3 · done · abc1234 · make check 41 s (slowest: …)`.
   Amend that into the same commit.
5. If the task changed a contract (format, MCP, kit), update the doc in the same commit.

## 4 · Stop

Report in five lines or fewer: the task, the result, the commit, the check times, and what's next. Then end the iteration.
