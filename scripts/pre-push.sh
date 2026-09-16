#!/usr/bin/env bash
# pre-push.sh — Fast local structural gate. Comprehensive validation belongs to CI.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

start=$SECONDS
step() { printf '\n[%s/6] %s\n' "$1" "$2"; }

# Cheap, read-only gates are independent. Capture their output separately so
# parallel execution stays readable, then print each result in step order.
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/quecto-pre-push.XXXXXX")"
trap 'rm -rf "$LOG_DIR"' EXIT

run_logged_gate() {
    local number="$1"
    local label="$2"
    shift 2
    (
        step "$number" "$label"
        "$@"
    ) >"$LOG_DIR/gate-${number}.log" 2>&1
}

run_logged_gate 1 "Repository quality rules" "$ROOT/scripts/check-quality.sh" &
QUALITY_PID=$!
run_logged_gate 2 "BDD quality rules" "$ROOT/scripts/check-bdd-quality.sh" &
BDD_QUALITY_PID=$!
run_logged_gate 3 "BDD status-tag rules" "$ROOT/scripts/check-bdd-tags.sh" &
BDD_TAG_PID=$!
run_logged_gate 4 "Formatting" cargo fmt --all -- --check &
FMT_PID=$!

FAILED=0
for gate in \
    "1:$QUALITY_PID" \
    "2:$BDD_QUALITY_PID" \
    "3:$BDD_TAG_PID" \
    "4:$FMT_PID"; do
    number="${gate%%:*}"
    pid="${gate##*:}"
    wait "$pid" || FAILED=1
    cat "$LOG_DIR/gate-${number}.log"
done
if (( FAILED != 0 )); then
    echo "Pre-push quality gates failed." >&2
    exit 1
fi

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

# One workspace feature unification everywhere: cargo resolves dependency
# features over the packages whose targets it builds, so a `-p <crate>` clippy
# or a bare `--test <harness target>` resolves a different feature set from the
# `--workspace --all-targets --features quecto-agentic-harness/test-support`
# shape CI uses, and every switch rebuilds the harness library and shared deps.
# `--bins` (five empty bin test harnesses) keeps every member's
# dev-dependencies in the resolution when only harness test targets run.
WORKSPACE_FEATURES=(--features quecto-agentic-harness/test-support)
TEST_SHAPE=(--workspace --no-fail-fast "${WORKSPACE_FEATURES[@]}" --bins)

# The two compilation-based gates are independent. Run them concurrently and
# preserve both statuses so either failure blocks the push.
(
    step 5 "Strict Clippy (workspace shape)"
    if (( WORKSPACE_CLIPPY == 1 || ${#PACKAGES[@]} > 0 )); then
        (( ${#PACKAGES[@]} > 0 )) && echo "  Changed packages: ${!PACKAGES[*]}"
        cargo clippy --workspace "${WORKSPACE_FEATURES[@]}" "${CLIPPY_ARGS[@]}"
    else
        echo "  No Rust workspace package changed; skipped."
    fi
    # Standalone shape: `cargo install --path` builds one package with default
    # features, without the workspace feature unification that enables
    # quecto-tui's `test-harness` via the harness's dev-dependency.
    for standalone in quecto-tui quecto-agentic-harness; do
        cargo clippy -p "$standalone" --lib --bins -- -D warnings
    done
) &
CLIPPY_PID=$!

(
    step 6 "Architecture and repository invariants"
    # cargo-nextest (one process per test, scheduled across the three
    # binaries; `ci` profile in .config/nextest.toml) when it is installed,
    # otherwise plain `cargo test` — same artifacts, same tests.
    if cargo nextest --version >/dev/null 2>&1; then
        cargo nextest run --workspace "${WORKSPACE_FEATURES[@]}" --bins \
            --test architecture \
            --test contracts \
            --test docs \
            --profile ci
    else
        echo "  cargo-nextest not installed (cargo install cargo-nextest --locked); using cargo test."
        cargo test "${TEST_SHAPE[@]}" \
            --test architecture \
            --test contracts \
            --test docs
    fi
) &
ARCH_PID=$!

FAILED=0
wait "$CLIPPY_PID" || FAILED=1
wait "$ARCH_PID" || FAILED=1
if (( FAILED != 0 )); then
    echo "Pre-push compilation gates failed." >&2
    exit 1
fi

elapsed=$((SECONDS - start))
printf '\nPre-push passed in %ss. Full workspace Clippy, tests, BDD, coverage and dependency policy run when the merge-requested label is applied.\n' "$elapsed"
if (( elapsed > 20 )); then
    echo "WARNING: pre-push exceeded its 20-second target (${elapsed}s)." >&2
fi
