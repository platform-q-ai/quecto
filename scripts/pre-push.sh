#!/usr/bin/env bash
# pre-push.sh — Fast local structural gate. Comprehensive validation belongs to CI.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

start=$SECONDS
step() { printf '\n[%s/6] %s\n' "$1" "$2"; }

step 1 "Repository quality rules"
"$ROOT/scripts/check-quality.sh"

step 2 "BDD quality rules"
"$ROOT/scripts/check-bdd-quality.sh"

step 3 "BDD status-tag rules"
"$ROOT/scripts/check-bdd-tags.sh"

step 4 "Formatting"
cargo fmt --all -- --check

step 5 "Strict Clippy for changed packages"
BASE_REF="${QUECTO_PREPUSH_BASE:-origin/master}"
if ! git rev-parse --verify "$BASE_REF" >/dev/null 2>&1; then
    BASE_REF="master"
fi
mapfile -t CHANGED_FILES < <(git diff --name-only "${BASE_REF}...HEAD")

WORKSPACE_CLIPPY=0
declare -A PACKAGES=()
for file in "${CHANGED_FILES[@]}"; do
    case "$file" in
        Cargo.toml|Cargo.lock|rust-toolchain|rust-toolchain.toml)
            WORKSPACE_CLIPPY=1
            ;;
        quecto-agentic-harness/*) PACKAGES[quecto-agentic-harness]=1 ;;
        quecto-tui/*) PACKAGES[quecto-tui]=1 ;;
        quecto-api/*) PACKAGES[quecto-api]=1 ;;
        quecto-mcp/*) PACKAGES[quecto-mcp]=1 ;;
        quecto-runtime-manager/*) PACKAGES[quecto-runtime-manager]=1 ;;
        quecto-line-io/*) PACKAGES[quecto-line-io]=1 ;;
    esac
done

CLIPPY_ARGS=(--all-targets -- -D warnings
    -W clippy::cognitive_complexity
    -W clippy::too_many_arguments
    -W clippy::too_many_lines)
if (( WORKSPACE_CLIPPY == 1 )); then
    cargo clippy --workspace --features quecto-agentic-harness/test-support "${CLIPPY_ARGS[@]}"
elif (( ${#PACKAGES[@]} > 0 )); then
    PACKAGE_ARGS=()
    while IFS= read -r package; do PACKAGE_ARGS+=(-p "$package"); done < <(printf '%s\n' "${!PACKAGES[@]}" | sort)
    echo "  Changed packages: ${!PACKAGES[*]}"
    cargo clippy "${PACKAGE_ARGS[@]}" "${CLIPPY_ARGS[@]}"
else
    echo "  No Rust workspace package changed; skipped."
fi

step 6 "Architecture and repository invariants"
cargo test -p quecto-agentic-harness --no-fail-fast \
    --test architecture \
    --test contracts \
    --test repo_docs \
    --test workflow_docs \
    --test workflow_config_template \
    --test workflow_config_refactor_template

elapsed=$((SECONDS - start))
printf '\nPre-push passed in %ss. Full workspace Clippy, tests, BDD, coverage and dependency policy run in authoritative merge-queue CI.\n' "$elapsed"
if (( elapsed > 20 )); then
    echo "WARNING: pre-push exceeded its 20-second target (${elapsed}s)." >&2
fi
