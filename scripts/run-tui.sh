#!/usr/bin/env bash
set -euo pipefail

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

# Pre-warm the kernel page cache so the cold-binary cost of the first launch
# after `cargo install` is paid before the TUI's socket-readiness window (#808).
# POSIX-safe and must not fail the script if `quecto` is not yet on PATH.
quecto --version >/dev/null 2>&1 || true

exec quecto-tui \
  --no-sandbox \
  --system "You are the lead developer of quecto and quecto-tui, adhere to YAGNI principles, BDD/TDD and Clean Architecture." \
  "$@"
