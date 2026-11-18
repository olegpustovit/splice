#!/bin/sh
# Reject signed bundles with bad metadata or a different designated requirement.
set -eu
source=${1:?usage: test-macos-bundle.sh STAGED_APP}
scratch=$(mktemp -d "${TMPDIR:-/tmp}/splice-bundle-test.XXXXXXXX")
trap 'rm -rf "$scratch"' EXIT
app="$scratch/Splice.app"
/usr/bin/ditto "$source" "$app"

reject() {
    if tools/check-macos-bundle.sh "$app" --static-only >"$scratch/log" 2>&1; then
        printf 'Bundle verifier missed: %s\n' "$1" >&2
        exit 1
    fi
    if ! grep -Fq "$1" "$scratch/log"; then
        cat "$scratch/log" >&2
        exit 1
    fi
}

/usr/libexec/PlistBuddy -c 'Delete :NSMicrophoneUsageDescription' "$app/Contents/Info.plist"
/usr/bin/codesign --force --sign - --entitlements packaging/splice.entitlements "$app" >/dev/null 2>&1
reject 'Info.plist requires NSMicrophoneUsageDescription'

cp "$source/Contents/Info.plist" "$app/Contents/Info.plist"
/usr/bin/codesign --force --sign - --entitlements packaging/splice.entitlements "$app" >/dev/null 2>&1
reject 'designated requirement differs'
