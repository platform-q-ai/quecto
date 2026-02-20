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

step "1/6" "Quality gate (work markers, lint bypasses, unsafe, ignored tests)"
"$ROOT/scripts/check-quality.sh"

step "2/6" "cargo fmt --check"
cargo fmt --all -- --check

step "3/6" "cargo clippy (strict: -D warnings + complexity lints)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "4/6" "cargo build --all-targets"
cargo build --all-targets

step "5/6" "cargo test --lib (unit tests)"
cargo test --lib

step "6/6" "cargo test --test architecture (boundary enforcement)"
cargo test --test architecture

echo -e "\n${GREEN}Pre-commit passed.${NC}"
