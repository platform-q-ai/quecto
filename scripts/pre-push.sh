#!/usr/bin/env bash
# pre-push.sh — Runs on every git push.
# Everything in pre-commit PLUS: full test suite, BDD tests, coverage, machete, deny.
set -euo pipefail

# Ensure ~/.cargo/bin is in PATH (needed for cargo-tarpaulin, cargo-machete, cargo-deny).
# Only prepend to PATH rather than sourcing full env to avoid overriding rustup overrides.
export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(git rev-parse --show-toplevel)"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

step() {
    echo -e "\n${BLUE}[$1]${NC} $2"
}

# --- Pre-commit checks ---

step "1/11" "Quality gate"
"$ROOT/scripts/check-quality.sh"

# Belt-and-suspenders: re-run BDD gate even though pre-commit also runs it,
# because commits can be created with --no-verify or cherry-picked from
# branches that never ran the pre-commit hook.
step "2/11" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/11" "cargo fmt --check"
cargo fmt --all -- --check

step "4/11" "cargo clippy (strict)"
cargo clippy --all-targets -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "5/11" "cargo build --all-targets"
cargo build --all-targets

step "6/11" "cargo test --lib (unit tests)"
cargo test --lib

step "7/11" "cargo test --test architecture (boundary enforcement)"
cargo test --test architecture

# --- Additional pre-push checks ---

step "8/11" "cargo test --test bdd (BDD integration tests)"
cargo test --test bdd

step "9/11" "cargo tarpaulin (code coverage)"
if command -v cargo-tarpaulin &>/dev/null; then
    cargo tarpaulin
else
    echo "  cargo-tarpaulin not installed, skipping coverage check"
    echo "  Install with: cargo install cargo-tarpaulin --locked"
fi

step "10/11" "cargo machete (unused dependencies)"
if command -v cargo-machete &>/dev/null; then
    cargo machete
else
    echo "  cargo-machete not installed, skipping unused dep check"
    echo "  Install with: cargo install cargo-machete --locked"
fi

step "11/11" "cargo deny check (licenses, advisories, bans)"
if command -v cargo-deny &>/dev/null; then
    cargo deny check
else
    echo "  cargo-deny not installed, skipping license/advisory check"
    echo "  Install with: cargo install cargo-deny --locked"
fi

echo -e "\n${GREEN}Pre-push passed.${NC}"
