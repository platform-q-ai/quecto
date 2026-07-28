#!/usr/bin/env bash
# pre-push.sh — Fast local structural gate. Comprehensive validation belongs to CI.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

start=$SECONDS
step() { printf '\n[%s/5] %s\n' "$1" "$2"; }

step 1 "Repository quality rules"
"$ROOT/scripts/check-quality.sh"

step 2 "BDD quality rules"
"$ROOT/scripts/check-bdd-quality.sh"

step 3 "BDD status-tag rules"
"$ROOT/scripts/check-bdd-tags.sh"

step 4 "Formatting"
cargo fmt --all -- --check

step 5 "Architecture and repository invariants"
cargo test -p quecto-agentic-harness --no-fail-fast \
    --test architecture \
    --test contracts \
    --test repo_docs \
    --test workflow_docs \
    --test workflow_config_template \
    --test workflow_config_refactor_template

elapsed=$((SECONDS - start))
printf '\nPre-push passed in %ss. Full tests, clippy, BDD, coverage and dependency policy run in authoritative merge-queue CI.\n' "$elapsed"
if (( elapsed > 20 )); then
    echo "WARNING: pre-push exceeded its 20-second target (${elapsed}s)." >&2
fi
