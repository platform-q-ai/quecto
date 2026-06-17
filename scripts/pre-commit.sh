#!/usr/bin/env bash
# pre-commit.sh — Runs on every git commit.
# Local static gates only; full tests run in pre-push.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

step "1/4" "Quality gate (work markers, file size, lint bypasses, unsafe, ignored tests)"
"$ROOT/scripts/check-quality.sh"

step "2/4" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/4" "cargo fmt --check"
cargo fmt --all -- --check

step "4/4" "cargo clippy (strict, workspace complexity gates)"
cargo clippy --workspace --all-targets --features test-support -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

# Block direct commits to master/main — force feature branches.
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")"
if [[ "$CURRENT_BRANCH" == "master" || "$CURRENT_BRANCH" == "main" ]]; then
    echo -e "\n${RED}ERROR: Direct commits to ${CURRENT_BRANCH} are not allowed.${NC}"
    echo -e "Create a feature branch first:\n"
    echo -e "  git checkout -b <branch-name>"
    echo -e "  git commit\n"
    exit 1
fi

echo -e "\n${GREEN}Pre-commit passed.${NC}"
