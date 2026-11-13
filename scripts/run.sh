#!/usr/bin/env bash
# Launch the game (debug build by default: FASTER=1 for release).
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ "${FASTER:-0}" == "1" ]]; then
  exec cargo run --release -p tetris-app -- "$@"
fi
exec cargo run -p tetris-app -- "$@"
