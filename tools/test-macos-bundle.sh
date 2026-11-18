#!/bin/sh
# A valid signature alone must not pass a bundle missing a privacy usage string.
set -eu
source=${1:?usage: test-macos-bundle.sh STAGED_APP}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/splice-bundle-test.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
app="$scratch/Splice.app"
/usr/bin/ditto "$source" "$app"
/usr/libexec/PlistBuddy -c 'Delete :NSMicrophoneUsageDescription' "$app/Contents/Info.plist"
/usr/bin/codesign --force --sign - --entitlements packaging/splice.entitlements "$app" >/dev/null 2>&1
if tools/check-macos-bundle.sh "$app" --static-only >"$scratch/log" 2>&1; then
    printf '%s\n' 'Accepted a signed bundle without its microphone usage string.' >&2
    exit 1
fi
if ! grep -Fq 'Info.plist requires NSMicrophoneUsageDescription' "$scratch/log"; then
    cat "$scratch/log" >&2
    exit 1
fi
