#!/bin/sh
# The Splice build loop (docs/LOOP.md §1). One fresh agent run per iteration; each does one task.
#   AGENT=codex|claude (default codex)   MAX=N iterations (default 40)   touch STOP to end after the current one
# Codex runs with full access because [M] tasks ssh to the Mac mini and rsync; it works in this repo only.
set -eu
cd "$(dirname "$0")/.."
agent=${AGENT:-codex}
max=${MAX:-40}
mkdir -p target/loop
prompt='Read and follow .agents/skills/splice-loop/SKILL.md exactly: one iteration of the Splice build loop in this repository. AGENTS.md rules apply.'
i=1
while [ "$i" -le "$max" ]; do
    [ -e STOP ] && { echo "loop: STOP file present, ending"; rm -f STOP; exit 0; }
    log="target/loop/$(date +%Y%m%d-%H%M%S)-$i.log"
    head=$(git rev-parse --short HEAD 2>/dev/null || echo none)
    echo "loop: iteration $i ($agent) at $head -> $log"
    case "$agent" in
        codex)  codex exec --dangerously-bypass-approvals-and-sandbox -C "$PWD" -o "$log.last" "$prompt" > "$log" 2>&1 || true ;;
        claude) claude -p --permission-mode bypassPermissions "$prompt" > "$log" 2>&1 || true ;;
        *) echo "loop: unknown AGENT=$agent" >&2; exit 2 ;;
    esac
    tail -n 8 "$log.last" 2>/dev/null || tail -n 8 "$log"
    if [ "$(git rev-parse --short HEAD 2>/dev/null || echo none)" = "$head" ]; then
        echo "loop: iteration $i made no commit; stopping so a human can look (see $log)"
        exit 1
    fi
    i=$((i + 1))
done
