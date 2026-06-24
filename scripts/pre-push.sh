#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Full local quality gate: static checks + parallel test wave (lib + architecture
# + contracts + repo docs + 24-way non-real BDD) + coverage + machete + deny +
# the real-LLM end-to-end suite (skippable with QUECTO_SKIP_REAL_LLM=1).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# NOTE: the deterministic wave must NOT load .env — a real provider key in the
# environment makes the agent see a configured provider, which breaks the many
# "no providers configured" / default-config tests. The real-LLM step below
# sources .env on its own, only after the deterministic wave has run.

E2E_TIMEOUT="${QUECTO_E2E_TIMEOUT:-12m}"
BDD_SHARDS="${QUECTO_BDD_SHARDS:-24}"
FORCE_RUN="${QUECTO_PREPUSH_FORCE:-0}"

HEAD_SHA="$(git rev-parse HEAD)"
SCRIPT_HASH="$(sha256sum "$ROOT/scripts/pre-push.sh" | awk '{print $1}')"
# Fold the real-LLM run-state into the cache key, so a cached pass with real-LLM
# skipped (no key / opt-out) doesn't suppress a later real-LLM run for the same
# SHA. Probe key presence in a SUBSHELL so .env never leaks into the
# deterministic wave below.
if [[ "${QUECTO_SKIP_REAL_LLM:-0}" == "1" ]]; then
    REAL_LLM_STATE="skip"
elif (
    source "$ROOT/scripts/load-dotenv.sh"
    [[ -n "${OPENAI_API_KEY:-}${QUECTO_PROVIDERS_OPENAI_API_KEY:-}" ]]
); then
    REAL_LLM_STATE="run"
else
    REAL_LLM_STATE="nokey"
fi
CACHE_FILE="$ROOT/.git/pre-push.passed.${HEAD_SHA}.${SCRIPT_HASH}.${REAL_LLM_STATE}"
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

step "1/10" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/10" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/10" "cargo fmt --check"
cargo fmt --all -- --check

step "4/10" "cargo clippy (strict, workspace)"
cargo clippy --workspace --all-targets --features test-support -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

COV_THRESHOLD="${QUECTO_COV_THRESHOLD:-87}"

step "5/10" "Parallel test wave: unit + every integration target + non-real BDD shards"

# Enumerate EVERY top-level integration test target dynamically rather than a
# hand-maintained allowlist. A static `--test architecture --test contracts ...`
# list silently drops any newly-added target (this is exactly how a broken
# tests/workflow_docs.rs reached master: it was never named, so the gate never
# ran it). Each `tests/*.rs` file is its own test binary; `tests/bdd/` is a
# directory (the sharded harness run separately below) and `tests/common/`,
# `tests/contracts/`, `tests/features/` are module dirs — none are top-level
# `.rs` files, so none are double-run here.
mapfile -t TEST_TARGETS < <(find "$ROOT/tests" -maxdepth 1 -name '*.rs' -printf '%f\n' | sed 's/\.rs$//' | sort)
if [[ "${#TEST_TARGETS[@]}" -eq 0 ]]; then
    echo -e "${RED}FAIL${NC}: no top-level integration test targets found under tests/ — gate enumeration is broken"
    exit 1
fi
TEST_TARGET_ARGS=()
for t in "${TEST_TARGETS[@]}"; do TEST_TARGET_ARGS+=(--test "$t"); done
echo "  Integration targets: ${TEST_TARGETS[*]}"

(
    cargo test --no-fail-fast --lib "${TEST_TARGET_ARGS[@]}" 2>&1 | "$ROOT/scripts/test-filter.sh"
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
    echo -e "${RED}FAIL${NC}: cargo test --lib ${TEST_TARGET_ARGS[*]}"
    FAIL=1
fi
if ! wait "$PID_BDD"; then
    echo -e "${RED}FAIL${NC}: non-real BDD shards"
    FAIL=1
fi

if [[ "$FAIL" -ne 0 ]]; then
    exit 1
fi

step "6/10" "Code coverage (cargo llvm-cov, threshold ${COV_THRESHOLD}%)"

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

step "7/10" "cargo machete (unused dependencies)"
if command -v cargo-machete &>/dev/null; then
    cargo machete
else
    echo "  cargo-machete not installed, skipping unused dep check"
    echo "  Install with: cargo install cargo-machete --locked"
fi

step "8/10" "cargo deny check (licenses, advisories, bans)"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "  cargo-deny not installed, skipping license/advisory check"
    echo "  Install with: cargo install cargo-deny --locked"
fi

step "9/10" "Real-LLM end-to-end tests"

# .env carries provider API keys; source it ONLY here. The deterministic wave
# above must run WITHOUT provider keys, or the "no providers configured" tests
# break. Skip cleanly when no key is present (CI, fresh checkout) or on opt-out.
source "$ROOT/scripts/load-dotenv.sh"

REAL_LLM_SHARDS="${QUECTO_REAL_LLM_SHARDS:-24}"
REAL_LLM_TIMEOUT="${QUECTO_REAL_LLM_TIMEOUT:-12m}"
if [[ "${QUECTO_SKIP_REAL_LLM:-0}" == "1" ]]; then
    echo "  SKIP: QUECTO_SKIP_REAL_LLM=1"
elif [[ -z "${OPENAI_API_KEY:-}${QUECTO_PROVIDERS_OPENAI_API_KEY:-}" ]]; then
    echo "  SKIP: no provider API key present (add OPENAI_API_KEY to .env to enable)"
else
    if ! bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "real-llm-bdd" \
        --shards "$REAL_LLM_SHARDS" \
        --timeout "$REAL_LLM_TIMEOUT" \
        --tag "real-llm" \
        --real-llm; then
        echo -e "${RED}FAIL${NC}: real-LLM end-to-end tests"
        exit 1
    fi
fi

step "10/10" "Pre-push summary"
echo "All local push gates passed."
echo "BDD shards: ${BDD_SHARDS}, timeout per shard: ${E2E_TIMEOUT}"
echo "Coverage threshold: ${COV_THRESHOLD}%"

echo -e "\n${GREEN}Pre-push passed.${NC}"
rm -f "$ROOT"/.git/pre-push.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
