#!/bin/sh
# Build the app and install it into /Applications (replacing any previous copy).
set -eu
cd "$(dirname "$0")/.."
scripts/bundle.sh
APP="/Applications/Colima GUI.app"
pkill -x colima-gui 2>/dev/null || true
rm -rf "$APP"
cp -R "target/bundle/Colima GUI.app" "$APP"
touch "$APP"  # make Finder/Dock pick up the new icon
echo "Installed: $APP"
