#!/usr/bin/env bash
set -euo pipefail

export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"

exec quecto-tui \
  --no-sandbox \
  --system "You are the lead developer of quecto and quecto-tui, adhere to YAGNI principles, BDD/TDD and Clean Architecture." \
  "$@"
