#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Fast checks only: quality gates, fmt, clippy, unit tests, architecture, BDD.
# Expensive checks (real-LLM, tarpaulin, machete, deny) live in pre-merge-commit.sh.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

E2E_TIMEOUT="${QUECTO_E2E_TIMEOUT:-5m}"
FORCE_RUN="${QUECTO_PREPUSH_FORCE:-0}"

HEAD_SHA="$(git rev-parse HEAD)"
SCRIPT_HASH="$(sha256sum "$ROOT/scripts/pre-push.sh" | awk '{print $1}')"
CACHE_FILE="$ROOT/.git/pre-push.passed.${HEAD_SHA}.${SCRIPT_HASH}"
LOG_FILE="$ROOT/.git/pre-push.last.log"

if [[ "$FORCE_RUN" != "1" && -f "$CACHE_FILE" ]]; then
    echo "Pre-push checks already passed for commit $HEAD_SHA."
    echo "Use QUECTO_PREPUSH_FORCE=1 to force a full rerun."
    exit 0
fi

# Keep a full log for post-mortem while preserving console output.
exec > >(tee "$LOG_FILE") 2>&1

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

# --- Pre-commit checks (belt-and-suspenders) ---

step "1/8" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/8" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/8" "cargo fmt --check"
cargo fmt --all -- --check

step "4/8" "cargo clippy (strict)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "5/8" "cargo build --all-targets"
cargo build --all-targets

step "6/8" "cargo test --lib (unit tests)"
cargo test --lib

step "7/8" "cargo test --test architecture (boundary enforcement)"
cargo test --test architecture

step "8/8" "cargo test --test bdd (BDD integration tests, timeout ${E2E_TIMEOUT})"
timeout "${E2E_TIMEOUT}" cargo test --test bdd

echo -e "\n${GREEN}Pre-push passed.${NC}"
rm -f "$ROOT"/.git/pre-push.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
