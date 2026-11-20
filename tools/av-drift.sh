#!/bin/bash
# AC-EXP-2 on the Mac: A/V drift at the end of a long export through the installed splice.
#   bash tools/av-drift.sh [seconds=600] [work dir]
# Generates a 60 fps movie with a white frame and an audio click at every whole second
# (tools/av-drift.swift), inserts it into a new 60 fps project, exports it with social-h264 and
# social-hevc, and measures click onset − flash frame for every second in the last minute (and,
# for reference, the source movie). One JSON line per movie.
set -eu
seconds=${1:-600}
dir=${2:-$(mktemp -d "${TMPDIR:-/tmp}/splice-av-drift.XXXXXX")}
splice=${SPLICE:-$HOME/.local/bin/splice}
here=$(cd "$(dirname "$0")" && pwd)
from=$((seconds > 60 ? seconds - 60 : 0))
mkdir -p "$dir"
swiftc -O -suppress-warnings "$here/av-drift.swift" -o "$dir/av-drift"
"$dir/av-drift" make "$dir/clicks.mov" "$seconds"
rm -rf "$dir/drift.splice"
"$splice" new "$dir/drift.splice" >/dev/null
printf '[{"cmd":"project.set","fps":60,"aspect":"16:9"},{"cmd":"clip.insert","path":"%s"}]' "$dir/clicks.mov" >"$dir/ops.json"
"$splice" apply "$dir/ops.json" "$dir/drift.splice" >/dev/null
"$dir/av-drift" measure "$dir/clicks.mov" "$from"
for preset in social-h264 social-hevc; do
    start=$(date +%s)
    "$splice" export "$dir/drift.splice" --preset "$preset" --output "$dir/$preset.mp4" --overwrite >/dev/null
    echo "{\"preset\":\"$preset\",\"exportSeconds\":$(($(date +%s) - start))}"
    "$dir/av-drift" measure "$dir/$preset.mp4" "$from"
done
