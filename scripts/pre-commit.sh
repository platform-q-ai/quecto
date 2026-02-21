#!/usr/bin/env bash
# pre-commit.sh — Runs on every git commit.
# Checks: quality gate, formatting, clippy (strict), build, unit tests, architecture tests.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

step "1/7" "Quality gate (work markers, lint bypasses, unsafe, ignored tests)"
"$ROOT/scripts/check-quality.sh"

step "2/7" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/7" "cargo fmt --check"
cargo fmt --all -- --check

step "4/7" "cargo clippy (strict: -D warnings + complexity lints)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "5/7" "cargo build --all-targets"
cargo build --all-targets

step "6/7" "cargo test --lib (unit tests)"
cargo test --lib

step "7/7" "cargo test --test architecture (boundary enforcement)"
cargo test --test architecture

# If committing directly to master (squash merge, ff commit, etc.),
# also run the expensive pre-merge-commit checks. The pre-merge-commit
# hook only fires for real merge commits, so this catches the other paths.
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")"
if [[ "$CURRENT_BRANCH" == "master" || "$CURRENT_BRANCH" == "main" ]]; then
    echo -e "\n${BLUE}[master]${NC} Detected commit to master — running pre-merge-commit checks"
    "$ROOT/scripts/pre-merge-commit.sh"
fi

echo -e "\n${GREEN}Pre-commit passed.${NC}"
