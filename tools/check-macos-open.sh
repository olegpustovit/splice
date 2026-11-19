#!/bin/sh
# Human-path matrix (T10.5): Finder's document-open event must reach a viewer in every state a
# person meets. Each case opens a hidden smoke viewer through Launch Services and expects it to
# present and exit cleanly. The capture-agent case needs the installed app and is checked by hand
# (PLAN.md T10.2).
set -eu
app=$(cd "${1:?usage: check-macos-open.sh STAGED_APP}" && pwd -P)
splice="$app/Contents/MacOS/splice"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/splice-open.XXXXXXXX")
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
mcp=
cleanup() {
    test -z "$mcp" || kill "$mcp" 2>/dev/null || true
    "$lsregister" -u "$app" >/dev/null 2>&1 || true
    rm -rf "$scratch"
}
trap cleanup EXIT
scratch=$(cd "$scratch" && pwd -P)

finder_open() {
    rm -f "$scratch/stdout" "$scratch/stderr"
    touch "$scratch/stdout" "$scratch/stderr"
    /usr/bin/open -g -j -n -W -a "$app" "$1" --env "HOME=$scratch" \
        --stdout "$scratch/stdout" --stderr "$scratch/stderr" --args --smoke-test
    if ! grep -q '"ok":true' "$scratch/stdout"; then
        printf 'check-macos-open: %s did not open\n' "$2" >&2
        cat "$scratch/stderr" >&2
        exit 1
    fi
}

# Clean: nothing else holds the project.
project="$scratch/Finder launch.splice"
"$splice" new "$project" >/dev/null
finder_open "$project" clean
test "$(cat "$scratch/Library/Application Support/Splice/last-project")" = "$project"

# Agent first: an agent's splice mcp still holds the writer (T10.1).
held="$scratch/Held.splice"
"$splice" new "$held" >/dev/null
mkfifo "$scratch/mcp.in"
HOME="$scratch" "$splice" mcp <"$scratch/mcp.in" >"$scratch/mcp.out" &
mcp=$!
exec 3>"$scratch/mcp.in"
printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"open-check","version":"1"}}}' \
    "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"project.info\",\"arguments\":{\"path\":\"$held\"}}}" >&3
tries=0
until [ "$(wc -l <"$scratch/mcp.out")" -ge 2 ]; do
    tries=$((tries + 1))
    [ "$tries" -lt 100 ] || { echo 'check-macos-open: splice mcp did not answer' >&2; exit 1; }
    sleep 0.05
done
grep -q '"projectId":"prj_1"' "$scratch/mcp.out"
finder_open "$held" 'a project held by splice mcp'
exec 3>&-
wait "$mcp"
mcp=

# Offline media: the sample fixture names movies that are not in the repository (T10.3).
cp -R "$(dirname "$0")/../fixtures/projects/sample.splice" "$scratch/Offline.splice"
finder_open "$scratch/Offline.splice" 'a project with offline media'
