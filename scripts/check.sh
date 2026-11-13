#!/usr/bin/env bash
# The gate a CI job would run (see docs/adr/0010-testing-strategy.md).
#   scripts/check.sh            → fmt, clippy, tests
#   scripts/check.sh --full     → also release build + windowed smoke test
set -euo pipefail
cd "$(dirname "$0")/.."

echo "── format ────────────────────────────────────────────"
cargo fmt --all --check

echo "── clippy ────────────────────────────────────────────"
cargo clippy --workspace --all-targets -- -D warnings

echo "── tests ─────────────────────────────────────────────"
cargo test --workspace

if [[ "${1:-}" == "--full" ]]; then
  echo "── release build ─────────────────────────────────────"
  cargo build --release -p tetris-app

  echo "── smoke test (Wayland) ──────────────────────────────"
  cargo run --release -p tetris-app -- --smoke-test --frames 90

  echo "── smoke test (X11 only) ─────────────────────────────"
  env -u WAYLAND_DISPLAY cargo run --release -p tetris-app -- --smoke-test --frames 90

  echo "── snapshot ──────────────────────────────────────────"
  cargo run --release -p tetris-app -- --snapshot /tmp/pi-tetris-snapshot.png
fi

echo "── all good ──────────────────────────────────────────"
