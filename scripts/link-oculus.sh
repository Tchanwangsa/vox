#!/bin/sh
# Point Vox's local avenue at the helpers and models Oculus already has, by
# symlink, so nothing is downloaded or copied twice.
#
#   scripts/link-oculus.sh [path to the oculus checkout]
set -eu

OCULUS="${1:-$HOME/Documents/gh/oculus}"
OCULUS_DATA="$HOME/Library/Application Support/com.tchan.oculus"
VOX_DATA="$HOME/Library/Application Support/com.tchan.vox"
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"

link() {
  if [ -e "$1" ]; then
    mkdir -p "$(dirname "$2")"
    ln -sfn "$1" "$2"
    echo "linked  $2"
  else
    echo "missing $1"
  fi
}

link "$OCULUS/app/src-tauri/binaries/whisper-cli-$TRIPLE" "$VOX_DATA/bin/whisper-cli"
link "$OCULUS/app/src-tauri/binaries/llama-server-$TRIPLE" "$VOX_DATA/bin/llama-server"
for model in "$OCULUS_DATA"/models/whisper/ggml-*.bin; do
  link "$model" "$VOX_DATA/models/whisper/$(basename "$model")"
done
link "$OCULUS_DATA/models/s1-mini/s1-mini-q4_k_m.gguf" "$VOX_DATA/models/s1-mini/s1-mini-q4_k_m.gguf"
