#!/bin/sh
# Sign Vox.app or a standalone vox binary with the local Apple Development
# identity and bundle identifier `com.tchan.vox`, so macOS Keychain and
# Accessibility grants carry over across rebuilds and `cargo run`.
#
#   scripts/sign.sh [--quiet] <path>
set -eu

QUIET=0
if [ "${1:-}" = "--quiet" ]; then
  QUIET=1
  shift
fi

TARGET="${1:?usage: scripts/sign.sh [--quiet] <path>}"
IDENTITY="${VOX_SIGN_IDENTITY:-$(security find-identity -v -p codesigning 2>/dev/null | awk 'NR==1 && $2 ~ /^[0-9A-F]+$/ {print $2}')}"

case "$TARGET" in
  *.app|*.app/)
    if [ -n "$IDENTITY" ] && codesign --force --sign "$IDENTITY" "$TARGET" 2>/dev/null; then
      [ "$QUIET" -eq 0 ] && echo "signed with $IDENTITY"
    else
      codesign --force --sign - "$TARGET"
      [ "$QUIET" -eq 0 ] && echo "signed ad hoc"
    fi
    ;;
  *)
    if [ -n "$IDENTITY" ] && codesign --force --sign "$IDENTITY" --identifier com.tchan.vox "$TARGET" 2>/dev/null; then
      [ "$QUIET" -eq 0 ] && echo "signed with $IDENTITY"
    elif [ "$QUIET" -eq 0 ]; then
      echo "no signing identity found; left $TARGET ad hoc" >&2
    fi
    ;;
esac
