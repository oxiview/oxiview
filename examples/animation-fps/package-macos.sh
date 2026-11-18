#!/bin/sh
# Assemble the local demo without installing it or requiring a signing identity.
set -eu
cd "$(dirname "$0")/../.."
cargo build --locked --release -p animation-fps
bundle="target/xtask/animation-fps/Oxiview Motion.app"
mkdir -p "$bundle/Contents/MacOS"
install -m 755 target/release/animation-fps "$bundle/Contents/MacOS/animation-fps"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.oxiview.animation-fps</string>
<key>CFBundleName</key><string>Oxiview Motion</string>
<key>CFBundleExecutable</key><string>animation-fps</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.0.0</string>
<key>CFBundleVersion</key><string>0.0.0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "$bundle"
