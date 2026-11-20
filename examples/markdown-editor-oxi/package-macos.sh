#!/bin/sh
# Assemble the release executable and its welcome document for local use.
set -eu
editor_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$editor_dir/../.." && pwd)
cd "$repo_root"
cargo build --locked --release -p markdown-editor-oxi
bundle="$repo_root/target/xtask/markdown-editor-oxi/Oxiview Markdown.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/fixtures" "$bundle/Contents/Resources/licenses"
cp "$repo_root/target/release/markdown-editor-oxi" "$bundle/Contents/MacOS/markdown-editor-oxi"
cp "$editor_dir/fixtures/acceptance.md" "$editor_dir/fixtures/hill.png" "$bundle/Contents/Resources/fixtures/"
cp "$repo_root/crates/oxiview-text/tests/fonts/NotoSans-OFL.txt" "$repo_root/crates/oxiview-text/tests/fonts/NotoSansArabic-OFL.txt" "$editor_dir/licenses/KaTeX-OFL.txt" "$bundle/Contents/Resources/licenses/"
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.oxiview.markdown-editor</string>
<key>CFBundleName</key><string>Oxiview Markdown</string>
<key>CFBundleExecutable</key><string>markdown-editor-oxi</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf '%s\n' "$bundle"
