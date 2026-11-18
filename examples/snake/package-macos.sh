#!/bin/sh
# Build a local development application bundle; no signing identity or installation.
set -eu
snake_repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$snake_repo_root"
cargo build --release -p snake
snake_bundle="$snake_repo_root/target/xtask/snake/Oxiview Snake.app"
mkdir -p "$snake_bundle/Contents/MacOS"
cp "$snake_repo_root/target/release/snake" "$snake_bundle/Contents/MacOS/snake"
cat > "$snake_bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.oxiview.snake</string>
<key>CFBundleName</key><string>Snake</string>
<key>CFBundleDisplayName</key><string>Oxiview Snake</string>
<key>CFBundleExecutable</key><string>snake</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.0.0</string>
<key>CFBundleShortVersionString</key><string>0.0.0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.games</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "$snake_bundle"
