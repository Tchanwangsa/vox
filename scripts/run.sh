#!/bin/sh
# Cargo runner on macOS: sign the `vox` binary with the same identity and
# identifier as Vox.app before launching so macOS Keychain doesn't prompt on
# every rebuild. Test binaries (`deps/vox-*`) run straight through.
set -eu

BIN="$1"
case "$(basename "$BIN")" in
  vox)
    "$(dirname "$0")/sign.sh" --quiet "$BIN" || true
    ;;
esac
exec "$@"
