#!/bin/sh
# Build PXUI's pinned SDL for the oldest macOS Splice supports.
set -eu
prefix=${1:?usage: build-sdl-macos.sh ABSOLUTE_PREFIX}
cd "$(dirname "$0")/.."

lockfile=pxui/packaging/sdl3.lock
version=$(sed -n 's/.*((sdl3 .*[(]floor \([^)]*\)[)].*/\1/p' "$lockfile")
sha=$(sed -n 's/.*((sdl3 .*[(]sha256 \([^)]*\)[)].*/\1/p' "$lockfile")
[ -n "$version" ] && [ -n "$sha" ]
floor=15.0
arch=$(uname -m)
stamp="$version $sha $floor $arch"
if [ -f "$prefix/lib/libSDL3.0.dylib" ] && [ -f "$prefix/include/SDL3/SDL.h" ] \
    && [ -f "$prefix/share/licenses/SDL3/LICENSE.txt" ] && [ -f "$prefix/.splice-sdl" ] \
    && [ "$(cat "$prefix/.splice-sdl")" = "$stamp" ]; then
    exit 0
fi

work=$PWD/target/sdl-source
mkdir -p "$work" "$prefix"
lock=$work/.build-lock
mkdir "$lock" 2>/dev/null || { printf 'splice: SDL is already being built in %s\n' "$work" >&2; exit 1; }
archive=$work/SDL3-$version.tar.gz
partial=$archive.part.$$
trap 'rm -f "$partial"; rmdir "$lock"' EXIT
trap 'exit 1' HUP INT TERM
if [ ! -f "$archive" ]; then
    /usr/bin/curl --fail --location --retry 2 "https://www.libsdl.org/release/SDL3-$version.tar.gz" -o "$partial"
    printf '%s  %s\n' "$sha" "$partial" | /usr/bin/shasum -a 256 --check
    mv "$partial" "$archive"
fi
printf '%s  %s\n' "$sha" "$archive" | /usr/bin/shasum -a 256 --check
tar -xzf "$archive" -C "$work"
source=$work/SDL3-$version
build=$work/build-$version-$arch
cmake -S "$source" -B "$build" \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_SYSROOT=macosx \
    -DCMAKE_OSX_ARCHITECTURES="$arch" -DCMAKE_OSX_DEPLOYMENT_TARGET="$floor" \
    -DCMAKE_INSTALL_PREFIX="$prefix" -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_INSTALL_NAME_DIR=@rpath \
    -DSDL_SHARED=ON -DSDL_STATIC=OFF -DSDL_FRAMEWORK=OFF \
    -DSDL_TESTS=OFF -DSDL_TEST_LIBRARY=OFF -DSDL_EXAMPLES=OFF
cmake --build "$build" --parallel 2
cmake --install "$build"
mkdir -p "$prefix/share/licenses/SDL3"
cp "$source/LICENSE.txt" "$prefix/share/licenses/SDL3/LICENSE.txt"
printf '%s\n' "$stamp" > "$prefix/.splice-sdl"
