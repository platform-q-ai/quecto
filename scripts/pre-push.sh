#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Everything in pre-commit PLUS: full test suite, BDD tests, coverage, machete, deny.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

# --- Pre-commit checks ---

step "1/10" "Quality gate"
"$ROOT/scripts/check-quality.sh"

step "2/10" "cargo fmt --check"
cargo fmt --all -- --check

step "3/10" "cargo clippy (strict)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "4/10" "cargo build --all-targets"
cargo build --all-targets

step "5/10" "cargo test --lib (unit tests)"
cargo test --lib

step "6/10" "cargo test --test architecture (boundary enforcement)"
cargo test --test architecture

# --- Additional pre-push checks ---

step "7/10" "cargo test --test bdd (BDD integration tests)"
cargo test --test bdd

step "8/10" "cargo tarpaulin --fail-under 90 (code coverage)"
if command -v cargo-tarpaulin &>/dev/null; then
    cargo tarpaulin --fail-under 90 --skip-clean
else
    echo "  cargo-tarpaulin not installed, skipping coverage check"
    echo "  Install with: cargo install cargo-tarpaulin --locked"
fi

step "9/10" "cargo machete (unused dependencies)"
if command -v cargo-machete &>/dev/null; then
    cargo machete
else
    echo "  cargo-machete not installed, skipping unused dep check"
    echo "  Install with: cargo install cargo-machete --locked"
fi

step "10/10" "cargo deny check (licenses, advisories, bans)"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "  cargo-deny not installed, skipping license/advisory check"
    echo "  Install with: cargo install cargo-deny --locked"
fi

echo -e "\n${GREEN}Pre-push passed.${NC}"
