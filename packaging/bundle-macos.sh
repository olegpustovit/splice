#!/bin/sh
# Assemble and verify a signed bundle before replacing the staged build.
set -eu
products=${1:?usage: bundle-macos.sh PRODUCTS SIGNING_KEYCHAIN SDL_PREFIX}
keychain=${2:?usage: bundle-macos.sh PRODUCTS SIGNING_KEYCHAIN SDL_PREFIX}
sdl_prefix=${3:?usage: bundle-macos.sh PRODUCTS SIGNING_KEYCHAIN SDL_PREFIX}
root=${4:-target/bundle}
mode=${5:-}
cd "$(dirname "$0")/.."
# Distribution needs Apple's secure timestamp (notarization); check bundles skip the network
# round trip with SPLICE_TIMESTAMP=--timestamp=none.
timestamp=${SPLICE_TIMESTAMP:---timestamp}

fail() {
    printf 'splice: %s\n' "$1" >&2
    exit 1
}

team=7ZH3FYV964
identity=$(/usr/bin/security find-identity -v -p codesigning "$keychain" 2>/dev/null | /usr/bin/awk -v team="$team" '
    length($2) == 40 && $2 ~ /^[A-F0-9]+$/ && index($0, "\"Developer ID Application: ") &&
    $0 ~ " \\(" team "\\)\"$" { print $2; exit }
')
[ -n "$identity" ] || fail "Import a valid Developer ID Application identity for team $team into $keychain and run make bundle again."

[ -x "$products/splice" ] || fail "Build the splice executable in $products first."
[ -f "$products/libpxui.2.dylib" ] || fail "Build libpxui.2.dylib in $products first."
[ -f "$sdl_prefix/lib/libSDL3.0.dylib" ] || fail 'Build the pinned macOS SDL with make sdl first.'

output=$root/Splice.app
lock=$root/.splice-publish-lock
/bin/mkdir -p "$root"
stage=$(/usr/bin/mktemp -d "$root/.stage.XXXXXXXX")
locked=no
cleanup() {
    if [ "$locked" = yes ]; then
        if [ -d "$stage/previous.app" ] && [ ! -e "$output" ] && [ ! -L "$output" ]; then
            /bin/mv "$stage/previous.app" "$output" || {
                printf 'splice: previous bundle is at %s/previous.app\n' "$stage" >&2
                /bin/rmdir "$lock"
                return
            }
        fi
        /bin/rmdir "$lock"
    fi
    /bin/rm -rf "$stage"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

app=$stage/Splice.app
binary=$app/Contents/MacOS/splice
library=$app/Contents/Frameworks/libpxui.2.dylib
sdl=$app/Contents/Frameworks/libSDL3.0.dylib
/bin/mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
/bin/cp packaging/Info.plist "$app/Contents/Info.plist"
/bin/cp "$products/splice" "$binary"
/bin/cp -L "$products/libpxui.2.dylib" "$library"
/bin/cp -L "$sdl_prefix/lib/libSDL3.0.dylib" "$sdl"
/bin/cp "$sdl_prefix/share/licenses/SDL3/LICENSE.txt" "$app/Contents/Resources/SDL3-LICENSE.txt"
/bin/cp crates/apple_sys/assets/barlow/OFL.txt "$app/Contents/Resources/Barlow-LICENSE.txt"
/bin/cp crates/apple_sys/assets/ibm-plex/OFL.txt "$app/Contents/Resources/IBM-Plex-LICENSE.txt"

clear_rpaths() {
    loads=$(/usr/bin/otool -l "$1")
    paths=$(printf '%s\n' "$loads" | /usr/bin/awk '
        /cmd LC_RPATH/ { path = 1; next }
        path && $1 == "path" {
            sub(/^[[:space:]]*path /, ""); sub(/ \(offset [0-9]+\)$/, ""); print; path = 0
        }
    ')
    while IFS= read -r path; do
        [ -z "$path" ] || /usr/bin/install_name_tool -delete_rpath "$path" "$1"
    done <<PATHS
$paths
PATHS
}

for file in "$binary" "$library" "$sdl"; do
    /usr/bin/codesign --remove-signature "$file" 2>/dev/null || true
    clear_rpaths "$file"
done
/usr/bin/install_name_tool -add_rpath '@executable_path/../Frameworks' "$binary"

requirement="anchor apple generic and certificate leaf[subject.OU] = \"$team\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
for file in "$library" "$sdl"; do
    /usr/bin/codesign --force --keychain "$keychain" --sign "$identity" "$timestamp" --options runtime "$file"
    /usr/bin/codesign --verify --strict --test-requirement "=$requirement" "$file"
done
/usr/bin/codesign --force --keychain "$keychain" --sign "$identity" "$timestamp" --options runtime \
    --entitlements packaging/splice.entitlements "$app"
/usr/bin/codesign --verify --deep --strict --test-requirement "=$requirement" "$app"
tools/check-macos-bundle.sh "$app" --static-only

/bin/mkdir "$lock" 2>/dev/null || fail "Another bundle is being published in $root."
locked=yes
[ ! -L "$output" ] || fail "Move the existing link at $output before bundling."
if [ -e "$output" ]; then
    bundle_id=$(/usr/bin/plutil -extract CFBundleIdentifier raw -o - "$output/Contents/Info.plist" 2>/dev/null) ||
        fail "Move the existing item at $output before bundling."
    [ "$bundle_id" = com.olegpustovit.splice ] || fail "Move the existing app at $output before bundling."
    if [ "$mode" = --install ]; then
        /usr/bin/swift packaging/quit-macos-app.swift "$output" "$bundle_id"
        # A terminal or MCP host owns its CLI process and must close it normally.
        if pids=$(/usr/sbin/lsof -t -a -d txt "$output/Contents/MacOS/splice" 2>"$stage/lsof.error"); then
            fail "Close the installed Splice CLI processes before reinstalling (PIDs: $(printf '%s' "$pids" | tr '\n' ' '))."
        elif [ -s "$stage/lsof.error" ]; then
            cat "$stage/lsof.error" >&2
            fail "Could not check which Splice processes are running."
        fi
        if [ "$root" -ef "$HOME/Applications" ]; then
            previous=$(/usr/bin/codesign --display -r- "$output" 2>&1 | /usr/bin/sed -n 's/^designated => //p')
            if [ "$previous" != "$(cat packaging/designated-requirement.txt)" ]; then
                printf 'Splice signing identity changed. Resetting its Screen Recording, Microphone, Camera and Input Monitoring grants.\n'
                for service in ScreenCapture Microphone Camera ListenEvent; do
                    /usr/bin/tccutil reset "$service" "$bundle_id" ||
                        fail "Could not reset $service for $bundle_id. Retry the install after resolving the error."
                done
            fi
        fi
    fi
    /bin/mv "$output" "$stage/previous.app"
fi
/bin/mv "$app" "$output"
printf 'Signed %s for team %s\n' "$output" "$team"
