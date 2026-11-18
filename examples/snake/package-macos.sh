#!/bin/sh
# Build a local development application bundle; no signing identity or installation.
set -eu
snake_example_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
snake_package=${snake_example_dir##*/}
case "$snake_package" in
    snake) snake_name="Snake" ;;
    snake-oxi) snake_name="Snake Oxi" ;;
    *) exit 2 ;;
esac
snake_repo_root=$(CDPATH= cd -- "$snake_example_dir/../.." && pwd)
cd "$snake_repo_root"
cargo build --release -p "$snake_package"
snake_bundle="$snake_repo_root/target/xtask/$snake_package/Oxiview $snake_name.app"
mkdir -p "$snake_bundle/Contents/MacOS"
cp "$snake_repo_root/target/release/$snake_package" "$snake_bundle/Contents/MacOS/$snake_package"
cat > "$snake_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.oxiview.$snake_package</string>
<key>CFBundleName</key><string>$snake_name</string>
<key>CFBundleDisplayName</key><string>Oxiview $snake_name</string>
<key>CFBundleExecutable</key><string>$snake_package</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.0.0</string>
<key>CFBundleShortVersionString</key><string>0.0.0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSApplicationCategoryType</key><string>public.app-category.games</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "$snake_bundle"
