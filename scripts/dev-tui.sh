#!/usr/bin/env bash
set -euo pipefail

# Resolve paths relative to this script so it works from any working directory.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPO_ROOT"

if [[ "${QUECTO_TUI_WATCH_RUN:-}" == "1" ]]; then
  cargo build -p quecto-agentic-harness -p quecto-tui
  export QUECTO_TUI_BIN_DIR="$REPO_ROOT/target/debug"
  exec "$SCRIPT_DIR/run-tui.sh" "$@"
fi

if ! command -v cargo-watch >/dev/null 2>&1; then
  printf '%s\n' "cargo-watch is required; install it with: cargo install cargo-watch" >&2
  exit 1
fi

export QUECTO_TUI_WATCH_RUN=1
exec cargo watch -- "$SCRIPT_DIR/dev-tui.sh" "$@"
