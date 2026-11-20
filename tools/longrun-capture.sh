#!/bin/bash
# AC-REC-1 on the Mac: one long display recording through the installed splice while a
# full-screen animation changes every frame, sampling CPU every 5 s.
#   bash tools/longrun-capture.sh [seconds=600] [work dir] [extra `splice record` flags…]
# CPU is the delta of cumulative CPU time (ps -o time) per 5 s window, in % of one core, for
# the splice processes (the `splice record` CLI + the capture agent); the system helpers that
# ScreenCaptureKit and the encoder run in (replayd, VTEncoderXPCService, WindowServer) are
# reported separately. Prints one JSON line.
set -eu
seconds=${1:-600}
dir=${2:-$(mktemp -d "${TMPDIR:-/tmp}/splice-longrun.XXXXXX")}
splice=${SPLICE:-$HOME/.local/bin/splice}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$dir"
swiftc -O -suppress-warnings "$here/longrun-capture.swift" -o "$dir/longrun-capture"
"$dir/longrun-capture" animate >"$dir/animate.json" &
animator=$!
caffeinate -dimsu -t $((seconds + 120)) &
awake=$!
trap 'kill $animator $awake 2>/dev/null || true' EXIT
sleep 3
rm -rf "$dir/long.splice"
"$splice" record "$dir/long.splice" --display main --duration "$seconds" --system-audio "${@:3}" \
    >"$dir/record.json" 2>"$dir/record.err" &
recorder=$!

# Cumulative CPU seconds of the newest process whose command matches $1.
cpu_of() {
    pid=$(pgrep -n -f -- "$1" || true)
    [ -n "$pid" ] || { echo 0; return; }
    ps -o time= -p "$pid" | awk -F: '{ s = 0; for (i = 1; i <= NF; i++) s = s * 60 + $i; print s }'
}
sample() {
    printf '%s %s %s %s %s %s\n' "$(python3 -c 'import time; print(time.time())')" \
        "$(ps -o time= -p $recorder 2>/dev/null | awk -F: '{ s = 0; for (i = 1; i <= NF; i++) s = s * 60 + $i; print s }')" \
        "$(cpu_of 'Splice.app/Contents/MacOS/splice --capture-agent')" \
        "$(cpu_of replayd)" "$(cpu_of VTEncoderXPCService)" "$(cpu_of WindowServer)"
}
: >"$dir/cpu.txt"
while kill -0 $recorder 2>/dev/null; do
    sample >>"$dir/cpu.txt"
    sleep 5
done
wait $recorder || { cat "$dir/record.err" >&2; exit 1; }
kill $animator && wait $animator || true
movie=$(ls "$dir"/long.splice/takes/*/screen.mov | head -1)
gaps=$("$dir/longrun-capture" gaps "$movie")
# Windows while recording: skip the first 10 s (agent launch, countdown) and partial samples.
cpu=$(awk -v skip=10 '
    NR == 1 { t0 = $1 }
    { t[NR] = $1; for (c = 2; c <= 6; c++) v[NR, c] = ($c == "" ? 0 : $c) }
    END {
        first = 0
        for (i = 1; i <= NR; i++) if (t[i] - t0 >= skip) { first = i; break }
        for (i = first + 1; i < NR; i++) {   # the last sample may follow the agent exiting
            dt = t[i] - t[i - 1]
            app = (v[i, 2] - v[i - 1, 2] + v[i, 3] - v[i - 1, 3]) / dt * 100
            if (v[i, 3] < v[i - 1, 3]) continue
            if (app > peak) peak = app
            n++
        }
        last = NR - 1
        span = t[last] - t[first]
        for (c = 2; c <= 6; c++) avg[c] = (v[last, c] - v[first, c]) / span * 100
        printf "{\"windows\":%d,\"appAvgPct\":%.1f,\"appPeakPct\":%.1f,\"cliAvgPct\":%.1f,\"agentAvgPct\":%.1f,\"replaydAvgPct\":%.1f,\"vtEncoderAvgPct\":%.1f,\"windowServerAvgPct\":%.1f}",
            n, avg[2] + avg[3], peak, avg[2], avg[3], avg[4], avg[5], avg[6]
    }' "$dir/cpu.txt")
take=$(python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); t=r.get("take",r); print(json.dumps({k:t.get(k) for k in ("seconds","frames","captureFps","videoFrames","droppedFrames","size","tracks")}))' "$dir/record.json")
printf '{"dir":"%s","take":%s,"movie":%s,"animator":%s,"cpu":%s}\n' "$dir" "$take" "$gaps" "$(cat "$dir/animate.json")" "$cpu"
