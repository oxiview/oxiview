#!/bin/sh
# Package the local release build and the license for its embedded font.
set -eu
viewer_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$viewer_dir/../.." && pwd)
if [ "$#" -eq 0 ]; then
    cd "$repo_root"
    # Let Cargo resolve the target directory and triple, with diagnostics on stderr.
    executable=$(python3 - <<'PY'
import json
import os
import subprocess
import sys

command = [
    "cargo", "build", "--release", "--locked", "-p", "log-viewer-oxi",
    "--message-format=json-render-diagnostics",
]
executables = set()
with subprocess.Popen(command, stdout=subprocess.PIPE) as build:
    for line in build.stdout:
        try:
            message = json.loads(line)
        except (json.JSONDecodeError, UnicodeDecodeError):
            sys.stderr.buffer.write(line)
            continue
        if not isinstance(message, dict) or message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target")
        path = message.get("executable")
        if (
            isinstance(target, dict)
            and target.get("name") == "log-viewer-oxi"
            and target.get("kind") == ["bin"]
            and isinstance(path, str)
        ):
            executables.add(path)

if build.returncode:
    sys.exit(build.returncode if build.returncode > 0 else 128 - build.returncode)
if len(executables) != 1:
    sys.exit(f"Expected one log-viewer-oxi executable from Cargo, found {len(executables)}")
executable = executables.pop()
if not os.path.isabs(executable):
    sys.exit(f"Cargo reported a relative log-viewer-oxi executable path: {executable!r}")
sys.stdout.buffer.write(os.fsencode(executable) + b"\n")
PY
    )
    output="$repo_root/target/xtask/log-viewer-oxi"
elif [ "$#" -eq 3 ] && [ "$1" = "--from-build" ]; then
    executable=$2
    output=$3
else
    echo "Usage: $0 [--from-build RELEASE_EXECUTABLE OUTPUT_DIRECTORY]" >&2
    exit 2
fi
bundle="$output/Oxiview Logs.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/licenses"
install -S -m 755 "$executable" "$bundle/Contents/MacOS/log-viewer-oxi"
cp "$repo_root/crates/oxiview-text/tests/fonts/NotoSans-OFL.txt" "$bundle/Contents/Resources/licenses/"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.oxiview.log-viewer</string>
<key>CFBundleName</key><string>Oxiview Logs</string>
<key>CFBundleExecutable</key><string>log-viewer-oxi</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "$bundle"
