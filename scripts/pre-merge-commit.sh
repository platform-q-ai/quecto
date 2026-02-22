#!/usr/bin/env bash
# pre-merge-commit.sh — Runs locally before a merge commit is created.
# Contains expensive checks that don't belong in pre-push:
#   real-LLM end-to-end tests (sharded), machete, deny.
# This fires on `git merge <branch>` into master (Git 2.24+).
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

source "$ROOT/scripts/load-dotenv.sh"

REAL_LLM_TIMEOUT="${QUECTO_REAL_LLM_TIMEOUT:-12m}"
REAL_LLM_SHARDS="${QUECTO_REAL_LLM_SHARDS:-25}"
REAL_LLM_TAG="${QUECTO_REAL_LLM_TAG:-real-llm}"
FORCE_RUN="${QUECTO_PREMERGE_FORCE:-0}"

HEAD_SHA="$(git rev-parse HEAD)"
SCRIPT_HASH="$(sha256sum "$ROOT/scripts/pre-merge-commit.sh" | awk '{print $1}')"
CACHE_FILE="$ROOT/.git/pre-merge-commit.passed.${HEAD_SHA}.${SCRIPT_HASH}"
LOG_FILE="$ROOT/.git/pre-merge-commit.last.log"

if [[ "$FORCE_RUN" != "1" && -f "$CACHE_FILE" ]]; then
    echo "Pre-merge-commit checks already passed for commit $HEAD_SHA."
    echo "Use QUECTO_PREMERGE_FORCE=1 to force a full rerun."
    exit 0
fi

exec > >(tee "$LOG_FILE") 2>&1

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

step "1/3" "Real-LLM end-to-end tests (${REAL_LLM_SHARDS} shards, timeout ${REAL_LLM_TIMEOUT})"
if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    bash "$ROOT/scripts/run-bdd-shards.sh" \
        --suite "real-llm-bdd" \
        --shards "$REAL_LLM_SHARDS" \
        --timeout "$REAL_LLM_TIMEOUT" \
        --tag "$REAL_LLM_TAG" \
        --real-llm
else
    echo "  OPENAI_API_KEY is not set — skipping real-LLM suite"
    echo "  Set OPENAI_API_KEY to run the full real-LLM end-to-end tests before merge"
fi

step "2/3" "cargo machete (unused dependencies)"
if command -v cargo-machete &>/dev/null; then
    cargo machete
else
    echo "  cargo-machete not installed, skipping unused dep check"
    echo "  Install with: cargo install cargo-machete --locked"
fi

step "3/3" "cargo deny check (licenses, advisories, bans)"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "  cargo-deny not installed, skipping license/advisory check"
    echo "  Install with: cargo install cargo-deny --locked"
fi

echo -e "\n${GREEN}Pre-merge-commit passed.${NC}"
rm -f "$ROOT"/.git/pre-merge-commit.passed.*
touch "$CACHE_FILE"
echo "Cached pass marker: $CACHE_FILE"
echo "Full log: $LOG_FILE"
