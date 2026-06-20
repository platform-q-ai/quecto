#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Local quality gate: static checks + parallel test wave (lib + architecture + contracts + repo docs + 24-way non-real BDD).
# Expensive checks (real-LLM, machete, deny) live in pre-merge-commit.sh.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# NOTE: pre-push runs only deterministic checks (no real-LLM step — those live in
# pre-merge-commit.sh). It must NOT load .env: a real provider key in the
# environment makes the agent see a configured provider, which breaks the many
# "no providers configured" / default-config tests in the deterministic wave.

E2E_TIMEOUT="${QUECTO_E2E_TIMEOUT:-12m}"
BDD_SHARDS="${QUECTO_BDD_SHARDS:-24}"
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

step "1/7" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/7" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/7" "cargo fmt --check"
cargo fmt --all -- --check

step "4/7" "cargo clippy (strict, workspace)"
cargo clippy --workspace --all-targets --features test-support -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

COV_THRESHOLD="${QUECTO_COV_THRESHOLD:-85}"

step "5/7" "Parallel test wave: unit + architecture + contracts + non-real BDD shards"

(
    cargo test --no-fail-fast --lib --test architecture --test contracts --test repo_docs 2>&1 | "$ROOT/scripts/test-filter.sh"
) &
PID_CORE_GUARDS=$!

(
    bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "non-real-bdd" \
        --shards "$BDD_SHARDS" \
        --timeout "$E2E_TIMEOUT"
) &
PID_BDD=$!

FAIL=0
if ! wait "$PID_CORE_GUARDS"; then
    echo -e "${RED}FAIL${NC}: cargo test --lib --test architecture --test contracts --test repo_docs"
    FAIL=1
fi
if ! wait "$PID_BDD"; then
    echo -e "${RED}FAIL${NC}: non-real BDD shards"
    FAIL=1
fi

if [[ "$FAIL" -ne 0 ]]; then
    exit 1
fi

step "6/7" "Code coverage (cargo llvm-cov, threshold ${COV_THRESHOLD}%)"

# Resolve llvm tools — cargo-llvm-cov needs these when llvm-tools-preview
# isn't installed via rustup (e.g. system Rust on Arch Linux).
if [[ -z "${LLVM_COV:-}" ]] && command -v llvm-cov &>/dev/null; then
    export LLVM_COV="$(command -v llvm-cov)"
fi
if [[ -z "${LLVM_PROFDATA:-}" ]] && command -v llvm-profdata &>/dev/null; then
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
fi

COV_FAIL=0

echo "  quecto (core)..."
COV_OUT_QUECTO=$(cargo llvm-cov --lib -p quecto --fail-under-regions "$COV_THRESHOLD" 2>&1) || {
    echo -e "  ${RED}FAIL${NC}: quecto region coverage below ${COV_THRESHOLD}%"
    COV_FAIL=1
}
echo "$COV_OUT_QUECTO" | tail -3

echo "  quecto-tui..."
COV_OUT_TUI=$(cargo llvm-cov --lib -p quecto-tui --fail-under-regions "$COV_THRESHOLD" 2>&1) || {
    echo -e "  ${RED}FAIL${NC}: quecto-tui region coverage below ${COV_THRESHOLD}%"
    COV_FAIL=1
}
echo "$COV_OUT_TUI" | tail -3

if [[ "$COV_FAIL" -ne 0 ]]; then
    echo -e "\n${RED}FAIL${NC}: Code coverage below ${COV_THRESHOLD}% threshold."
    echo "  Run: cargo llvm-cov --workspace --lib   to see full report"
    echo "  Run: cargo llvm-cov --html --workspace --lib   for HTML report"
    exit 1
fi

step "7/7" "Pre-push summary"
echo "All local push gates passed."
echo "BDD shards: ${BDD_SHARDS}, timeout per shard: ${E2E_TIMEOUT}"
echo "Coverage threshold: ${COV_THRESHOLD}%"

echo -e "\n${GREEN}Pre-push passed.${NC}"
rm -f "$ROOT"/.git/pre-push.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
