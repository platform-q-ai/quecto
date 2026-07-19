#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Full local quality gate: static checks + parallel test wave (lib + architecture
# + contracts + repo docs + coverage-enabled non-real BDD shards) + coverage + machete + deny +
# the zero-cost mocked end-to-end suite (@mock-llm). The live, paid
# @manual-real-llm suite is NOT run by default; opt in on demand with
# QUECTO_RUN_REAL_LLM=1.
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
TUI_BDD_SHARDS="${QUECTO_TUI_BDD_SHARDS:-8}"
FORCE_RUN="${QUECTO_PREPUSH_FORCE:-0}"

HEAD_SHA="$(git rev-parse HEAD)"
SCRIPT_HASH="$(sha256sum "$ROOT/scripts/pre-push.sh" | awk '{print $1}')"
# Default lane runs the zero-cost mocked e2e suite — no provider key is probed
# and no .env is sourced before the deterministic wave, so a key in .env can
# NEVER auto-trigger paid provider calls. The live @manual-real-llm suite runs only on
# explicit opt-in. Fold that opt-in into the cache key so a cached mock-only pass
# doesn't suppress a later opted-in real-LLM run for the same SHA.
REAL_LLM_LANE="${QUECTO_RUN_REAL_LLM:-0}"
CACHE_FILE="$ROOT/.git/pre-push.passed.${HEAD_SHA}.${SCRIPT_HASH}.real${REAL_LLM_LANE}"
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

step "1/11" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/11" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/11" "BDD status-tag gate (every scenario is @done / @wip / @pending)"
"$ROOT/scripts/check-bdd-tags.sh"

step "4/11" "cargo fmt --check"
cargo fmt --all -- --check

step "5/11" "cargo clippy (strict, workspace)"
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

COV_THRESHOLD="${QUECTO_COV_THRESHOLD:-95}"
API_COV_THRESHOLD="${QUECTO_API_COV_THRESHOLD:-95}"
HARNESS_BDD_COV_THRESHOLD="${QUECTO_HARNESS_BDD_COV_THRESHOLD:-72}"
TUI_BDD_COV_THRESHOLD="${QUECTO_TUI_BDD_COV_THRESHOLD:-62}"

step "6/11" "Parallel test wave: unit + every integration target + coverage-enabled non-real BDD shards"

# Enumerate EVERY top-level integration test target dynamically rather than a
# hand-maintained allowlist. A static `--test architecture --test contracts ...`
# list silently drops any newly-added target (this is exactly how a broken
# quecto-agentic-harness/tests/workflow_docs.rs reached master: it was never named, so the gate never
# ran it). Each harness `tests/*.rs` file is its own test binary; `tests/bdd/` is a
# directory (the sharded harness run separately below) and `tests/common/`,
# `tests/contracts/`, `tests/features/` are module dirs - none are top-level
# `.rs` files, so none are double-run here.
HARNESS_ROOT="$ROOT/quecto-agentic-harness"
mapfile -t TEST_TARGETS < <(find "$HARNESS_ROOT/tests" -maxdepth 1 -name '*.rs' -printf '%f\n' | sed 's/\.rs$//' | sort)
if [[ "${#TEST_TARGETS[@]}" -eq 0 ]]; then
    echo -e "${RED}FAIL${NC}: no top-level integration test targets found under quecto-agentic-harness/tests/ - gate enumeration is broken"
    exit 1
fi
TEST_TARGET_ARGS=()
for t in "${TEST_TARGETS[@]}"; do TEST_TARGET_ARGS+=(--test "$t"); done
echo "  Integration targets: ${TEST_TARGETS[*]}"

(
    cargo test -p quecto-agentic-harness --no-fail-fast --lib "${TEST_TARGET_ARGS[@]}" 2>&1 | "$ROOT/scripts/test-filter.sh"
) &
PID_CORE_GUARDS=$!

# quecto-api unit + BDD (@done scenarios). The gateway package was previously
# compiled by clippy but never TESTED by any gate (#1061 review follow-up).
(
    cargo test -p quecto-api --no-fail-fast --all-targets 2>&1 | "$ROOT/scripts/test-filter.sh"
) &
PID_API=$!

(
    bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "non-real-bdd" \
        --shards "$BDD_SHARDS" \
        --timeout "$E2E_TIMEOUT" \
        --coverage \
        --coverage-threshold "$HARNESS_BDD_COV_THRESHOLD"
) &
PID_BDD=$!

(
    bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "tui-bdd" \
        --package "quecto-tui" \
        --features "test-harness" \
        --shards "$TUI_BDD_SHARDS" \
        --timeout "$E2E_TIMEOUT" \
        --coverage \
        --coverage-threshold "$TUI_BDD_COV_THRESHOLD"
) &
PID_TUI_BDD=$!

FAIL=0
if ! wait "$PID_CORE_GUARDS"; then
    echo -e "${RED}FAIL${NC}: cargo test -p quecto-agentic-harness --lib ${TEST_TARGET_ARGS[*]}"
    FAIL=1
fi
if ! wait "$PID_API"; then
    echo -e "${RED}FAIL${NC}: cargo test -p quecto-api --all-targets"
    FAIL=1
fi
if ! wait "$PID_BDD"; then
    echo -e "${RED}FAIL${NC}: non-real BDD shards"
    FAIL=1
fi
if ! wait "$PID_TUI_BDD"; then
    echo -e "${RED}FAIL${NC}: TUI BDD shards"
    FAIL=1
fi

if [[ "$FAIL" -ne 0 ]]; then
    exit 1
fi

step "7/11" "Code coverage (cargo llvm-cov, function coverage, threshold ${COV_THRESHOLD}%)"

# Resolve llvm tools — cargo-llvm-cov needs these when llvm-tools-preview
# isn't installed via rustup (e.g. system Rust on Arch Linux).
if [[ -z "${LLVM_COV:-}" ]] && command -v llvm-cov &>/dev/null; then
    export LLVM_COV="$(command -v llvm-cov)"
fi
if [[ -z "${LLVM_PROFDATA:-}" ]] && command -v llvm-profdata &>/dev/null; then
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
fi

COV_FAIL=0

# Test-only apparatus is excluded from coverage: it exists to *drive* tests
# (headless TUI harness, warn capture, provider/test fixtures), so counting its
# unexercised helpers against production coverage only inflates the denominator.
COV_IGNORE_REGEX='(tui_harness|test_support|warn_capture)'

echo "  quecto (core)..."
COV_OUT_QUECTO=$(cargo llvm-cov --lib -p quecto-agentic-harness --ignore-filename-regex "$COV_IGNORE_REGEX" --fail-under-functions "$COV_THRESHOLD" 2>&1) || {
    echo -e "  ${RED}FAIL${NC}: quecto function coverage below ${COV_THRESHOLD}%"
    COV_FAIL=1
}
echo "$COV_OUT_QUECTO" | tail -3

echo "  quecto-tui..."
COV_OUT_TUI=$(cargo llvm-cov --lib -p quecto-tui --ignore-filename-regex "$COV_IGNORE_REGEX" --fail-under-functions "$COV_THRESHOLD" 2>&1) || {
    echo -e "  ${RED}FAIL${NC}: quecto-tui function coverage below ${COV_THRESHOLD}%"
    COV_FAIL=1
}
echo "$COV_OUT_TUI" | tail -3

echo "  quecto-api..."
COV_OUT_API=$(cargo llvm-cov --lib -p quecto-api --ignore-filename-regex "$COV_IGNORE_REGEX" --fail-under-functions "$API_COV_THRESHOLD" 2>&1) || {
    echo -e "  ${RED}FAIL${NC}: quecto-api function coverage below ${API_COV_THRESHOLD}%"
    COV_FAIL=1
}
echo "$COV_OUT_API" | tail -3

if [[ "$COV_FAIL" -ne 0 ]]; then
    echo -e "\n${RED}FAIL${NC}: Function coverage below ${COV_THRESHOLD}% threshold."
    echo "  Run: cargo llvm-cov --workspace --lib   to see full report"
    echo "  Run: cargo llvm-cov --html --workspace --lib   for HTML report"
    exit 1
fi

step "8/11" "cargo machete (unused dependencies)"
if command -v cargo-machete &>/dev/null; then
    cargo machete
else
    echo "  cargo-machete not installed, skipping unused dep check"
    echo "  Install with: cargo install cargo-machete --locked"
fi

step "9/11" "cargo deny check (licenses, advisories, bans)"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "  cargo-deny not installed, skipping license/advisory check"
    echo "  Install with: cargo install cargo-deny --locked"
fi

step "10/11" "Mocked end-to-end tests (@mock-llm, zero-cost, default)"

# Default e2e lane: deterministic WireMock-backed copy of the @real-llm
# behaviours. Makes ZERO paid provider calls and needs no API key — .env is
# never sourced here, so a key in .env can't turn this into a paid run.
# The mocked suite mirrors the retired live behavioral suite, so shard it like
# the broader BDD lanes. Override with QUECTO_MOCK_LLM_SHARDS if needed locally.
MOCK_LLM_SHARDS="${QUECTO_MOCK_LLM_SHARDS:-24}"
MOCK_LLM_TIMEOUT="${QUECTO_MOCK_LLM_TIMEOUT:-12m}"
if ! bash "$ROOT/scripts/run-bdd-shards.sh" \
    --suite "mock-llm-bdd" \
    --shards "$MOCK_LLM_SHARDS" \
    --timeout "$MOCK_LLM_TIMEOUT" \
    --tag "mock-llm"; then
    echo -e "${RED}FAIL${NC}: mocked end-to-end tests"
    exit 1
fi

# Optional live lane: the paid @manual-real-llm suite is retained for occasional
# on-demand validation. It runs ONLY when explicitly opted in — a .env key alone
# never triggers it. Enable with: QUECTO_RUN_REAL_LLM=1 git push
if [[ "${QUECTO_RUN_REAL_LLM:-0}" == "1" ]]; then
    echo -e "\n${BLUE}[10b/11]${NC} Live real-LLM end-to-end tests (opt-in)"
    # run-bdd-shards.sh --real-llm sources .env (provider credentials) itself.
    # If no key is configured the real-LLM workspace step fails loudly, which is
    # the correct outcome for an explicit opt-in run.
    REAL_LLM_SHARDS="${QUECTO_REAL_LLM_SHARDS:-24}"
    REAL_LLM_TIMEOUT="${QUECTO_REAL_LLM_TIMEOUT:-12m}"
    if ! bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "real-llm-bdd" \
        --shards "$REAL_LLM_SHARDS" \
        --timeout "$REAL_LLM_TIMEOUT" \
        --tag "manual-real-llm" \
        --real-llm; then
        echo -e "${RED}FAIL${NC}: real-LLM end-to-end tests"
        exit 1
    fi
else
    echo "  Live @manual-real-llm suite skipped (opt in with QUECTO_RUN_REAL_LLM=1)."
fi

step "11/11" "Pre-push summary"
echo "All local push gates passed."
echo "BDD shards: ${BDD_SHARDS}; TUI BDD shards: ${TUI_BDD_SHARDS}; timeout per shard: ${E2E_TIMEOUT}"
echo "Lib function coverage threshold: ${COV_THRESHOLD}% (quecto-api ${API_COV_THRESHOLD}%)"
echo "BDD function coverage thresholds: harness ${HARNESS_BDD_COV_THRESHOLD}%; TUI ${TUI_BDD_COV_THRESHOLD}%"

echo -e "\n${GREEN}Pre-push passed.${NC}"
rm -f "$ROOT"/.git/pre-push.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
