#!/bin/sh
# Package a distributable macOS release (.dmg, .zip, and SHA-256 checksums)
# from the current Cargo.toml version. Uses ad-hoc signing by default so
# personal development certificates are not embedded in public artifacts.
#
#   scripts/release.sh
#   VOX_SIGN_IDENTITY="Developer ID Application: ..." scripts/release.sh
set -eu
cd "$(dirname "$0")/.."

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"
ARCH="$(uname -m)"
DIST="target/release-dist"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/vox-dmg.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

VOX_SIGN_IDENTITY="${VOX_SIGN_IDENTITY:--}" ./scripts/bundle.sh

rm -rf "$DIST"
mkdir -p "$DIST"

ZIP="$DIST/Vox-${VERSION}-macos-${ARCH}.zip"
DMG="$DIST/Vox-${VERSION}-macos-${ARCH}.dmg"

ditto -c -k --sequesterRsrc --keepParent target/Vox.app "$ZIP"

cp -R target/Vox.app "$STAGE/Vox.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create \
  -volname "Vox ${VERSION}" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  "$DMG" >/dev/null

(cd "$DIST" && shasum -a 256 "$(basename "$DMG")" "$(basename "$ZIP")" > SHA256SUMS.txt)

echo "packaged release artifacts in $DIST:"
ls -lh "$DIST"
