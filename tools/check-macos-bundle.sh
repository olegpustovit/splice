#!/bin/sh
# Adapted from LocalWave's rust-port bundle verifier for Splice's native payload.
set -eu
app=${1:-target/bundle/Splice.app}
mode=${2:-}
if [ "$#" -gt 2 ] || { [ -n "$mode" ] && [ "$mode" != --static-only ]; }; then
    printf 'Usage: %s [Splice.app [--static-only]]\n' "$0" >&2
    exit 2
fi
app=$(cd "$app" && pwd -P)
root=$(CDPATH= cd "$(dirname "$0")/.." && pwd -P)

python3 -I - "$app" "$root/packaging/designated-requirement.txt" <<'PY'
import plistlib
import re
import subprocess
import sys
from pathlib import Path


def require(condition, message):
    if not condition:
        sys.exit(f"splice bundle: {message}")


contents = Path(sys.argv[1]) / "Contents"
executable = contents / "MacOS/splice"
frameworks = contents / "Frameworks"
library_names = {"libpxui.1.dylib", "libSDL3.0.dylib"}
project_type = "com.olegpustovit.splice.project"

with (contents / "Info.plist").open("rb") as file:
    info = plistlib.load(file)
require(isinstance(info, dict), "Info.plist must be a dictionary")
for key, value in {
    "CFBundleIdentifier": "com.olegpustovit.splice",
    "CFBundleExecutable": "splice",
    "CFBundleName": "Splice",
    "CFBundleDisplayName": "Splice",
    "CFBundlePackageType": "APPL",
    "LSMinimumSystemVersion": "15.0",
}.items():
    require(info.get(key) == value, f"Info.plist {key} must be {value}")
for key in (
    "CFBundleShortVersionString", "CFBundleVersion",
    "NSCameraUsageDescription", "NSMicrophoneUsageDescription",
):
    value = info.get(key)
    require(isinstance(value, str) and value.strip(), f"Info.plist requires {key}")

documents = info.get("CFBundleDocumentTypes")
require(isinstance(documents, list) and len(documents) == 1 and
        isinstance(documents[0], dict), "Info.plist requires one project document declaration")
document = documents[0]
require(document.get("CFBundleTypeRole") == "Editor" and
        document.get("LSHandlerRank") == "Owner" and
        document.get("LSTypeIsPackage") is True and
        document.get("LSItemContentTypes") == [project_type],
        "Info.plist CFBundleDocumentTypes must register Splice as the .splice package editor")
exports = info.get("UTExportedTypeDeclarations")
require(isinstance(exports, list) and len(exports) == 1 and
        isinstance(exports[0], dict), "Info.plist requires one exported project type")
export = exports[0]
require(export.get("UTTypeIdentifier") == project_type and
        export.get("UTTypeConformsTo") == ["com.apple.package", "public.content"] and
        export.get("UTTypeTagSpecification") == {"public.filename-extension": ["splice"]},
        "Info.plist UTExportedTypeDeclarations must declare the .splice package")

require(frameworks.is_dir() and not frameworks.is_symlink() and
        {file.name for file in frameworks.iterdir()} == library_names,
        "Contents/Frameworks must contain the bundled PXUI and SDL libraries")
for binary in (executable, *(frameworks / name for name in sorted(library_names))):
    require(binary.is_file() and not binary.is_symlink(), f"missing regular binary {binary}")
    architectures = subprocess.check_output(["/usr/bin/lipo", "-archs", binary], text=True).split()
    require(architectures, f"no Mach-O architectures in {binary}")
    if binary == executable:
        app_architectures = set(architectures)
    else:
        require(app_architectures.issubset(architectures), f"{binary} is missing an app architecture")

    for architecture in architectures:
        otool = ["/usr/bin/otool", "-arch", architecture]
        commands = subprocess.check_output([*otool, "-l", binary], text=True)
        minimums = re.findall(
            r"cmd LC_BUILD_VERSION\s+cmdsize \d+\s+platform (?:1|MACOS)\s+minos ([\d.]+)",
            commands,
        ) + re.findall(r"cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version ([\d.]+)", commands)
        require(len(minimums) == 1 and
                (tuple(map(int, minimums[0].split("."))) + (0, 0))[:3] <= (15, 0, 0),
                f"{binary} ({architecture}) must target macOS 15.0 or earlier")

        rpaths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.*) \(offset \d+\)", commands)
        expected = ["@executable_path/../Frameworks"] if binary == executable else []
        require(rpaths == expected, f"unexpected rpaths in {binary} ({architecture}): {rpaths}")

        links = subprocess.check_output([*otool, "-L", binary], text=True)
        dependencies = set(re.findall(r"^\s+(.+?) \(compatibility version ", links, re.M))
        bundled = {f"@rpath/{name}" for name in library_names}
        require(dependencies, f"no Mach-O dependencies in {binary} ({architecture})")
        for dependency in sorted(dependencies - bundled):
            require(dependency.startswith(("/usr/lib/", "/System/Library/")),
                    f"unbundled dependency {dependency} in {binary} ({architecture})")
        if binary == executable:
            require(bundled <= dependencies, f"{binary} must link both bundled libraries")
        else:
            identity = subprocess.check_output([*otool, "-D", binary], text=True).splitlines()[1:]
            require(identity == [f"@rpath/{binary.name}"], f"invalid dylib install name in {binary}")

app = contents.parent
subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", app], check=True)
expected_requirement = Path(sys.argv[2]).read_text().strip()
display = subprocess.check_output(
    ["/usr/bin/codesign", "--display", "-r-", app], stderr=subprocess.STDOUT, text=True,
)
require(f"designated => {expected_requirement}" in display.splitlines(),
        "designated requirement differs from packaging/designated-requirement.txt")
subprocess.run(
    ["/usr/bin/codesign", "--verify", "--strict", "--test-requirement", f"={expected_requirement}", app],
    check=True,
)
signature = subprocess.check_output(
    ["/usr/bin/codesign", "--display", "--xml", "--entitlements", "-", app],
    stderr=subprocess.DEVNULL,
)
entitlements = plistlib.loads(signature) if signature else {}
expected = {"com.apple.security.device.audio-input", "com.apple.security.device.camera"}
require(isinstance(entitlements, dict) and set(entitlements) == expected and
        all(entitlements[key] is True for key in expected),
        "signed entitlements must enable camera and audio input only")
print(f"Verified {app}")
PY

if [ "$mode" != --static-only ]; then
    scratch=$(mktemp -d "${TMPDIR:-/tmp}/splice-bundle.XXXXXXXX")
    trap 'rm -rf "$scratch"' EXIT
    ln -s "$app/Contents/MacOS/splice" "$scratch/splice"
    (cd "$scratch" && HOME="$scratch" ./splice --smoke-test)
fi
