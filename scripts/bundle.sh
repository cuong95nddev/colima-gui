#!/bin/sh
# Build "Colima GUI.app" into target/bundle/.
#   UNIVERSAL=1 scripts/bundle.sh   → arm64 + x86_64 universal binary (used by CI)
set -eu
cd "$(dirname "$0")/.."

NAME="Colima GUI"
OUT="target/bundle"
APP="$OUT/$NAME.app"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

if [ "${UNIVERSAL:-0}" = 1 ]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/colima-gui \
    target/aarch64-apple-darwin/release/colima-gui \
    target/x86_64-apple-darwin/release/colima-gui
  BIN=target/universal/colima-gui
else
  cargo build --release
  BIN=target/release/colima-gui
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/colima-gui"

# App icon: render PNGs from src/icon.rs and pack them into an .icns
ICONSET="target/AppIcon.iconset"
rm -rf "$ICONSET"
cargo run --release --quiet --example export_icon -- "$ICONSET"
rm -f "$ICONSET/tray.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>dev.cuong95nddev.colima-gui</string>
  <key>CFBundleExecutable</key><string>colima-gui</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Ad-hoc signature (no Apple Developer ID).
codesign --force --sign - "$APP"
echo "$APP"
