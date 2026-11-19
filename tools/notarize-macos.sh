#!/bin/sh
# Package the signed Splice.app (make bundle) as a DMG; notarize and staple both when
# NOTARY_PROFILE names a notarytool keychain profile (xcrun notarytool store-credentials).
# Without it the DMG is Developer ID signed and verified but not notarized, and says so.
set -eu
app=${1:?usage: notarize-macos.sh APP SIGNING_KEYCHAIN [DIST]}
keychain=${2:?usage: notarize-macos.sh APP SIGNING_KEYCHAIN [DIST]}
dist=${3:-target/dist}
cd "$(dirname "$0")/.."

fail() {
    printf 'splice: %s\n' "$1" >&2
    exit 1
}

[ -d "$app" ] || fail "Build $app with make bundle first."
team=7ZH3FYV964
identity=$(/usr/bin/security find-identity -v -p codesigning "$keychain" 2>/dev/null | /usr/bin/awk -v team="$team" '
    length($2) == 40 && $2 ~ /^[A-F0-9]+$/ && index($0, "\"Developer ID Application: ") &&
    $0 ~ " \\(" team "\\)\"$" { print $2; exit }
')
[ -n "$identity" ] || fail "Import the Developer ID Application identity for team $team into $keychain."
/usr/bin/codesign --verify --deep --strict "$app"
version=$("$app/Contents/MacOS/splice" version | /usr/bin/awk '{ print $2 }')
printf '%s\n' "$version" | /usr/bin/grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "Unexpected version: $version"

work=$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/splice-dmg.XXXXXX")
trap '/bin/rm -rf "$work"' EXIT HUP INT TERM

notarize() {
    /usr/bin/xcrun notarytool submit "$1" --keychain-profile "$NOTARY_PROFILE" --wait
}

# Stapling fetches the ticket from Apple's CDN, which intermittently drops the connection.
retry() {
    for attempt in 1 2 3 4 5; do
        "$@" && return 0
        printf 'splice: retry %s/5: %s\n' "$attempt" "$*" >&2
        sleep 30
    done
    return 1
}

stage=$work/dmg-root
/bin/mkdir -p "$stage"
/usr/bin/ditto "$app" "$stage/Splice.app"
if [ -n "${NOTARY_PROFILE:-}" ]; then
    /usr/bin/ditto -c -k --keepParent "$stage/Splice.app" "$work/Splice.zip"
    notarize "$work/Splice.zip"
    retry /usr/bin/xcrun stapler staple "$stage/Splice.app"
    retry /usr/bin/xcrun stapler validate "$stage/Splice.app"
    /usr/sbin/spctl --assess --type execute --verbose=2 "$stage/Splice.app"
fi
/bin/ln -s /Applications "$stage/Applications"

/bin/mkdir -p "$dist"
dmg=$dist/Splice-$version.dmg
/bin/rm -f "$dmg"
/usr/bin/hdiutil create -quiet -volname Splice -srcfolder "$stage" -ov -format UDZO "$dmg"
/usr/bin/hdiutil verify -quiet "$dmg"
/usr/bin/codesign --force --keychain "$keychain" --sign "$identity" --timestamp "$dmg"
/usr/bin/codesign --verify --strict "$dmg"
if [ -n "${NOTARY_PROFILE:-}" ]; then
    notarize "$dmg"
    retry /usr/bin/xcrun stapler staple "$dmg"
    retry /usr/bin/xcrun stapler validate "$dmg"
    /usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
else
    printf '%s\n' 'splice: signed, not notarized: set NOTARY_PROFILE (xcrun notarytool store-credentials) to notarize.' >&2
fi
printf '%s\n' "$dmg"
