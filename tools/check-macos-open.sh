#!/bin/sh
# Exercise Finder's document-open event without opening a visible viewer.
set -eu
app=$(cd "${1:?usage: check-macos-open.sh STAGED_APP}" && pwd -P)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/splice-open.XXXXXXXX")
lsregister=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
cleanup() {
    "$lsregister" -u "$app" >/dev/null 2>&1 || true
    rm -rf "$scratch"
}
trap cleanup EXIT
scratch=$(cd "$scratch" && pwd -P)
ln -s "$app/Contents/MacOS/splice" "$scratch/splice"
(cd "$scratch" && HOME="$scratch" ./splice --smoke-test)
project="$scratch/Finder launch.splice"
"$app/Contents/MacOS/splice" new "$project" >/dev/null
touch "$scratch/stdout" "$scratch/stderr"
/usr/bin/open -g -j -n -W -a "$app" "$project" --env "HOME=$scratch" \
    --stdout "$scratch/stdout" --stderr "$scratch/stderr" --args --smoke-test
cat "$scratch/stderr"
grep -Fxq '{"ok":true}' "$scratch/stdout"
test "$(cat "$scratch/Library/Application Support/Splice/last-project")" = "$project"
