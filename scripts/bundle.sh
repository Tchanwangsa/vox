#!/bin/sh
# Build Vox.app: the release binary in a bundle whose Info.plist lets macOS
# ask for the microphone, signed so its permissions survive rebuilds.
#
#   scripts/bundle.sh          build target/Vox.app
#   scripts/bundle.sh --open   build it, then launch it
set -eu
cd "$(dirname "$0")/.."

cargo build --release
APP=target/Vox.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/vox "$APP/Contents/MacOS/vox"
[ -f assets/Vox.icns ] && cp assets/Vox.icns "$APP/Contents/Resources/Vox.icns"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Vox</string>
  <key>CFBundleDisplayName</key><string>Vox</string>
  <key>CFBundleIdentifier</key><string>com.tchan.vox</string>
  <key>CFBundleExecutable</key><string>vox</string>
  <key>CFBundleIconFile</key><string>Vox</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key>
  <string>Vox listens while you hold the shortcut, to turn what you say into text.</string>
</dict>
</plist>
PLIST

# A real signing identity keeps Accessibility and keychain grants across
# rebuilds; ad-hoc signing works but macOS asks again after each build.
./scripts/sign.sh "$APP"
echo "built $APP"
[ "${1:-}" = "--open" ] && open "$APP"
exit 0
