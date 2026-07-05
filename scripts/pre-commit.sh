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

step "1/6" "Quality gate (work markers, file size, lint bypasses, unsafe, ignored tests)"
"$ROOT/scripts/check-quality.sh"

step "2/6" "BDD quality gate (stubs, always-pass tests, reimplemented logic)"
"$ROOT/scripts/check-bdd-quality.sh"

step "3/6" "BDD status-tag gate (every scenario is @done / @wip / @pending)"
"$ROOT/scripts/check-bdd-tags.sh"

step "4/6" "cargo fmt --check"
cargo fmt --all -- --check

step "5/6" "cargo clippy (strict, workspace complexity gates)"
cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings \
    -W clippy::cognitive_complexity \
    -W clippy::too_many_arguments \
    -W clippy::too_many_lines

step "6/6" "Fast guard tests (docs, architecture, contracts, config)"
# Run the deterministic top-level integration guards at commit time — they are
# the doc/architecture/contract/config checks that catch drift like a config
# step reorder breaking quecto-agentic-harness/tests/workflow_docs.rs. They are fast and have no
# external deps. The heavy lib unit tests, BDD shards, coverage and real-LLM
# suites stay in pre-push. Targets are enumerated dynamically so a new
# harness tests/*.rs guard is picked up automatically and can never be silently
# dropped from the gate. `tests/bdd/` is a subdir (the sharded harness), so it
# is excluded by -maxdepth 1.
HARNESS_ROOT="$ROOT/quecto-agentic-harness"
mapfile -t GUARD_TARGETS < <(find "$HARNESS_ROOT/tests" -maxdepth 1 -name '*.rs' -printf '%f\n' | sed 's/\.rs$//' | sort)
if [[ "${#GUARD_TARGETS[@]}" -eq 0 ]]; then
    echo -e "\n${RED}ERROR: no top-level integration guard targets found under quecto-agentic-harness/tests/ - gate enumeration is broken.${NC}"
    exit 1
fi
GUARD_TARGET_ARGS=()
for t in "${GUARD_TARGETS[@]}"; do GUARD_TARGET_ARGS+=(--test "$t"); done
echo "  Guard targets: ${GUARD_TARGETS[*]}"
cargo test -p quecto-agentic-harness --no-fail-fast "${GUARD_TARGET_ARGS[@]}"

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
