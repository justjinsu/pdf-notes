#!/bin/sh
# Builds "PDF Notes.app" in app/dist — double-click it, or drag it into
# the Applications folder to keep it in Launchpad and the Dock.
set -e
cd "$(dirname "$0")"
# PDF_NOTES_BINARY=<file>: wrap an already built program (the release build on
# GitHub hands over one that runs on both Apple and Intel Macs).
BIN="${PDF_NOTES_BINARY:-}"
if [ -z "$BIN" ]; then
  if command -v cargo >/dev/null 2>&1; then CARGO=cargo; else CARGO="$HOME/.cargo/bin/cargo"; fi
  "$CARGO" build --release
  BIN=target/release/pdf-notes
fi
APP="dist/PDF Notes.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/PDF Notes"
iconutil -c icns assets/AppIcon.iconset -o "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>PDF Notes</string>
  <key>CFBundleDisplayName</key><string>PDF Notes</string>
  <key>CFBundleIdentifier</key><string>app.pdfnotes.desktop</string>
  <key>CFBundleExecutable</key><string>PDF Notes</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${PDF_NOTES_VERSION:-0.1.0}</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "Built: $(pwd)/$APP"
