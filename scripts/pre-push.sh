#!/usr/bin/env bash
# pre-push.sh — Fast local lint-only gate. Comprehensive validation belongs to CI.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

start=$SECONDS
step() { printf '\n[%s/4] %s\n' "$1" "$2"; }

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

run_logged_gate 1 "Repository quality rules" \
    bash -c '"$1/scripts/check-quality.sh" && "$1/scripts/check-golden-manifest.sh" && bash "$1/scripts/check-layout-ratchet-tests.sh" && "$1/scripts/check-layout-ratchet.sh"' _ "$ROOT" &
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
if (( FAILED == 0 )); then
    echo "All local lint-only gates passed."
else
    echo "Pre-push quality gates failed." >&2
    exit 1
fi

elapsed=$((SECONDS - start))
printf '\nPre-push passed in %ss. Strict workspace and standalone Clippy, architecture, contracts, docs, full tests, BDD and dependency policy run when the merge-requested label is applied; coverage runs nightly on master.\n' "$elapsed"
if (( elapsed > 20 )); then
    echo "WARNING: pre-push exceeded its 20-second target (${elapsed}s)." >&2
fi
