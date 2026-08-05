#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

SUITE_NAME="bdd"
PACKAGE="quecto-agentic-harness"
FEATURES="test-support"
SHARDS="24"
TIMEOUT_PER_SHARD="5m"
TAG=""
REAL_LLM="0"
COVERAGE="0"
COVERAGE_THRESHOLD=""
IGNORE_FILENAME_REGEX=""

resolve_llvm_tools() {
    # cargo-llvm-cov can use system LLVM tools when rustup llvm-tools-preview
    # is not installed. Preserve explicit caller settings, otherwise prefer PATH.
    if [[ -z "${LLVM_COV:-}" ]] && command -v llvm-cov &>/dev/null; then
        export LLVM_COV="$(command -v llvm-cov)"
    fi
    if [[ -z "${LLVM_PROFDATA:-}" ]] && command -v llvm-profdata &>/dev/null; then
        export LLVM_PROFDATA="$(command -v llvm-profdata)"
    fi
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --suite)
            SUITE_NAME="$2"
            shift 2
            ;;
        --package)
            PACKAGE="$2"
            shift 2
            ;;
        --features)
            FEATURES="$2"
            shift 2
            ;;
        --shards)
            SHARDS="$2"
            shift 2
            ;;
        --timeout)
            TIMEOUT_PER_SHARD="$2"
            shift 2
            ;;
        --tag)
            TAG="$2"
            shift 2
            ;;
        --real-llm)
            REAL_LLM="1"
            shift
            ;;
        --coverage)
            COVERAGE="1"
            shift
            ;;
        --coverage-threshold)
            COVERAGE_THRESHOLD="$2"
            shift 2
            ;;
        --ignore-filename-regex)
            IGNORE_FILENAME_REGEX="$2"
            shift 2
            ;;
        *)
            echo "Unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

if ! [[ "$SHARDS" =~ ^[0-9]+$ ]] || [[ "$SHARDS" -le 0 ]]; then
    echo "--shards must be a positive integer" >&2
    exit 2
fi

# Load repo-local .env (API keys) ONLY for real-LLM runs, so direct invocations
# pick it up without `set -a; . ./.env`. Deterministic (non-real) shards must
# NOT see provider keys — an ambient key makes "no providers" tests fail.
if [[ "$REAL_LLM" == "1" ]]; then
    source "$ROOT/scripts/load-dotenv.sh"
fi

# Use the resolved git dir (not "$ROOT/.git"): in a git worktree, .git is a
# FILE pointer, not a directory, so mktemp under it fails. --git-common-dir is
# always a real directory in both the main checkout and worktrees.
GIT_DIR_RESOLVED="$(git rev-parse --git-common-dir)"
[[ "$GIT_DIR_RESOLVED" = /* ]] || GIT_DIR_RESOLVED="$ROOT/$GIT_DIR_RESOLVED"

# Reclaim scratch dirs left by prior runs of THIS suite before creating a new
# one. A fully successful run self-cleans via the EXIT trap below; a failed or
# interrupted run's dir is deliberately retained so its shard logs/coverage stay
# inspectable, so this prune is what bounds how many such dirs survive (keeping
# the most recent $SHARD_DIR_KEEP). Without it every run's multi-GB llvm-cov
# target dir leaked forever (#1203: 114 GB across 62 dirs observed). Cleanup
# failures must never fail the test run, hence the trailing `|| true`.
SHARD_DIR_KEEP="${QUECTO_BDD_SHARD_KEEP:-3}"
find "$GIT_DIR_RESOLVED" -maxdepth 1 -type d -name "${SUITE_NAME}-shards.*" \
    -printf '%T@ %p\n' 2>/dev/null \
    | sort -rn \
    | awk -v keep="$SHARD_DIR_KEEP" 'NR>keep {print $2}' \
    | while IFS= read -r stale; do rm -rf -- "$stale"; done || true

TMP_DIR="$(mktemp -d "$GIT_DIR_RESOLVED/${SUITE_NAME}-shards.XXXXXX")"

# Remove the scratch dir on a fully successful run — its per-run llvm-cov
# target/ dir alone is several GB (#1203). Retain it on any failure so the
# failing shards' logs and coverage report remain inspectable; the startup
# prune above bounds how many retained dirs accumulate. Set
# QUECTO_BDD_KEEP_SCRATCH=1 to always keep it (e.g. to inspect a passing run).
RUN_STATUS="incomplete"
cleanup_shard_dir() {
    local rc=$?
    if [[ "$RUN_STATUS" == "success" && "${QUECTO_BDD_KEEP_SCRATCH:-0}" != "1" ]]; then
        rm -rf -- "$TMP_DIR"
    else
        echo "Scratch dir retained for inspection: ${TMP_DIR}" >&2
    fi
    return "$rc"
}
trap cleanup_shard_dir EXIT

echo "Running ${SUITE_NAME} in ${SHARDS} shard(s); package: ${PACKAGE}; features: ${FEATURES}; timeout per shard: ${TIMEOUT_PER_SHARD}"
[[ -n "$TAG" ]] && echo "Tag filter: ${TAG}"
[[ "$REAL_LLM" == "1" ]] && echo "QUECTO_REAL_LLM=1"
echo "Logs: ${TMP_DIR}"

if [[ "$COVERAGE" == "1" ]]; then
    resolve_llvm_tools
    export CARGO_TARGET_DIR="$TMP_DIR/target"
    # Source cargo-llvm-cov's build environment once. A per-run target dir keeps
    # concurrent sharded suites from cleaning or merging each other's profiles.
    eval "$(cargo llvm-cov show-env --sh)"
    echo "Coverage: enabled; target/profiles: ${CARGO_TARGET_DIR}"
    [[ -n "$COVERAGE_THRESHOLD" ]] && echo "Coverage function threshold: ${COVERAGE_THRESHOLD}%"
fi

declare -a PIDS=()
declare -a SHARD_IDS=()

for i in $(seq 0 $((SHARDS - 1))); do
    (
        start="$(date +%s)"
        env_args=(
            "QUECTO_BDD_SHARD_INDEX=${i}"
            "QUECTO_BDD_SHARD_TOTAL=${SHARDS}"
        )
        [[ -n "$TAG" ]] && env_args+=("QUECTO_TAG=${TAG}")
        [[ "$REAL_LLM" == "1" ]] && env_args+=("QUECTO_REAL_LLM=1")

        set +e
        timeout "$TIMEOUT_PER_SHARD" env "${env_args[@]}" cargo test -p "$PACKAGE" --no-fail-fast --features "$FEATURES" --test bdd 2>&1 | "$ROOT/scripts/test-filter.sh"
        code="${PIPESTATUS[0]}"
        set -e
        end="$(date +%s)"
        echo "exit=${code} elapsed=$((end - start))s" >"$TMP_DIR/shard-${i}.result"
        exit "$code"
    ) >"$TMP_DIR/shard-${i}.log" 2>&1 &
    PIDS+=("$!")
    SHARD_IDS+=("$i")
done

FAIL=0
for idx in "${!PIDS[@]}"; do
    pid="${PIDS[$idx]}"
    shard="${SHARD_IDS[$idx]}"
    if ! wait "$pid"; then
        FAIL=1
        echo "Shard ${shard} failed (see $TMP_DIR/shard-${shard}.log)"
        # Surface the failure inline — on CI the log files are unreachable.
        echo "--- shard-${shard}.log (last 400 lines) ---"
        tail -n 400 "$TMP_DIR/shard-${shard}.log" || true
        echo "--- end shard-${shard}.log ---"
    fi
done

max_elapsed=0
for i in $(seq 0 $((SHARDS - 1))); do
    if [[ -f "$TMP_DIR/shard-${i}.result" ]]; then
        line="$(cat "$TMP_DIR/shard-${i}.result")"
        elapsed="${line##*elapsed=}"
        elapsed="${elapsed%s}"
        if [[ "$elapsed" =~ ^[0-9]+$ ]] && ((elapsed > max_elapsed)); then
            max_elapsed="$elapsed"
        fi
        echo "shard-${i} ${line}"
    else
        FAIL=1
        echo "shard-${i} exit=missing elapsed=unknown"
    fi
done

echo "max-shard-elapsed=${max_elapsed}s"

if [[ "$FAIL" -ne 0 ]]; then
    echo "${SUITE_NAME} shards failed. Inspect logs in ${TMP_DIR}" >&2
    exit 1
fi

if [[ "$COVERAGE" == "1" ]]; then
    report_args=(report -p "$PACKAGE")
    if [[ -n "$IGNORE_FILENAME_REGEX" ]]; then
        report_args+=(--ignore-filename-regex "$IGNORE_FILENAME_REGEX")
    fi
    if [[ -n "$COVERAGE_THRESHOLD" ]]; then
        report_args+=(--fail-under-functions "$COVERAGE_THRESHOLD")
    fi
    echo "Generating merged coverage report..."
    if ! cargo llvm-cov "${report_args[@]}" 2>&1 | tee "$TMP_DIR/coverage.txt"; then
        echo "${SUITE_NAME} coverage gate failed. Report: ${TMP_DIR}/coverage.txt" >&2
        exit 1
    fi
    echo "Coverage report: ${TMP_DIR}/coverage.txt"
fi

RUN_STATUS="success"
echo "${SUITE_NAME} shards passed."
