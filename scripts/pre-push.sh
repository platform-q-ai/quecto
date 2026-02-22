#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Local quality gate: static checks + parallel test wave (lib + architecture + 25-way non-real BDD).
# Expensive checks (real-LLM, machete, deny) live in pre-merge-commit.sh.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

bash "$ROOT/scripts/load-dotenv.sh"

E2E_TIMEOUT="${QUECTO_E2E_TIMEOUT:-12m}"
BDD_SHARDS="${QUECTO_BDD_SHARDS:-25}"
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

step "1/6" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/6" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/6" "cargo fmt --check"
cargo fmt --all -- --check

step "4/6" "cargo clippy (strict)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "5/6" "Parallel test wave: unit + architecture + non-real BDD shards"

(
    cargo test --lib
) &
PID_LIB=$!

(
    cargo test --test architecture
) &
PID_ARCH=$!

(
    bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "non-real-bdd" \
        --shards "$BDD_SHARDS" \
        --timeout "$E2E_TIMEOUT"
) &
PID_BDD=$!

FAIL=0
if ! wait "$PID_LIB"; then
    echo -e "${RED}FAIL${NC}: cargo test --lib"
    FAIL=1
fi
if ! wait "$PID_ARCH"; then
    echo -e "${RED}FAIL${NC}: cargo test --test architecture"
    FAIL=1
fi
if ! wait "$PID_BDD"; then
    echo -e "${RED}FAIL${NC}: non-real BDD shards"
    FAIL=1
fi

if [[ "$FAIL" -ne 0 ]]; then
    exit 1
fi

step "6/6" "Pre-push summary"
echo "All local push gates passed."
echo "BDD shards: ${BDD_SHARDS}, timeout per shard: ${E2E_TIMEOUT}"

echo -e "\n${GREEN}Pre-push passed.${NC}"
rm -f "$ROOT"/.git/pre-push.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
