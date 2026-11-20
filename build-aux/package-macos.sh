#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CONFIG="${1:-release}"

case "$CONFIG" in
  debug)
    PROFILE=dev
    TARGET_DIR=debug
    ;;
  release)
    PROFILE=release
    TARGET_DIR=release
    ;;
  *)
    echo "usage: $0 [debug|release]" >&2
    exit 1
    ;;
esac

OUT="$ROOT/dist/obs-plugin_voice_changer.plugin"
LIB="$ROOT/target/$TARGET_DIR/libobs_filter.dylib"

(
  cd "$ROOT"
  cargo build -p obs-filter --profile "$PROFILE"
)

rm -rf "$OUT"
mkdir -p "$OUT/Contents/MacOS" "$OUT/Contents/Resources"
cp "$LIB" "$OUT/Contents/MacOS/obs-plugin_voice-changer"

cat > "$OUT/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Voice Changer</string>
  <key>CFBundleExecutable</key>
  <string>obs-plugin_voice-changer</string>
  <key>CFBundleIdentifier</key>
  <string>com.example.obs-plugin-voice-changer</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>obs-plugin_voice-changer</string>
  <key>CFBundlePackageType</key>
  <string>BNDL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CFBundleSupportedPlatforms</key>
  <array>
    <string>MacOSX</string>
  </array>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$OUT/Contents/MacOS/obs-plugin_voice-changer"
codesign --force --sign - "$OUT"
echo "created $OUT"
