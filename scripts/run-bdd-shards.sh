#!/usr/bin/env bash
# run-bdd-shards.sh — build one cucumber test binary, run it N times in parallel.
#
# Build once, run many: the test target is compiled with a single
# `cargo test ... --no-run --message-format=json` and the resulting executable
# is spawned directly once per shard with the same cwd and environment cargo
# would give it (package dir, CARGO_MANIFEST_DIR, CARGO_BIN_EXE_*, target
# dir, LD_LIBRARY_PATH). N concurrent `cargo test` processes used to contend
# on the target-dir lock and each re-ran the fingerprint walk; with
# `--coverage` they also started from a fresh instrumented target dir every
# run, which CI's rust-cache could never keep warm.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

SUITE_NAME="bdd"
# `--package` names the crate whose coverage is reported; the build itself is
# always `cargo test --workspace --features quecto-agentic-harness/test-support
# --bins --test <target>`, the one feature unification every other test
# invocation shares (a `-p` build, or a bare `--test <target>` that builds a
# single crate, resolves a different dependency feature set and rebuilds the
# harness library and shared deps; `--bins` are five empty bin test harnesses
# that keep every member's dev-dependencies in the resolution). Non-harness
# BDD runners are named `<crate>_bdd`, so a `--test` filter selects exactly
# one target.
PACKAGE="quecto-agentic-harness"
TEST_TARGET="bdd"
FEATURES="quecto-agentic-harness/test-support"
SHARDS="8"
TIMEOUT_PER_SHARD="5m"
TAG=""
REAL_LLM="0"
COVERAGE="0"
COVERAGE_THRESHOLD=""
BUILD_ONLY="0"

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
            # Workspace-qualified, e.g. `quecto-agentic-harness/test-support`.
            FEATURES="$2"
            shift 2
            ;;
        --test-target)
            TEST_TARGET="$2"
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
        --build-only)
            # Compile the test binary (instrumented with --coverage) and stop;
            # used to warm CI's target-dir cache without running scenarios.
            BUILD_ONLY="1"
            shift
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
# the most recent $SHARD_DIR_KEEP). Cleanup failures must never fail the test
# run, hence the trailing `|| true`. (The scratch dir holds logs and the
# coverage text only; the instrumented build lives in a stable target dir.)
SHARD_DIR_KEEP="${QUECTO_BDD_SHARD_KEEP:-3}"
find "$GIT_DIR_RESOLVED" -maxdepth 1 -type d -name "${SUITE_NAME}-shards.*" \
    -printf '%T@ %p\n' 2>/dev/null \
    | sort -rn \
    | awk -v keep="$SHARD_DIR_KEEP" 'NR>keep {print $2}' \
    | while IFS= read -r stale; do rm -rf -- "$stale"; done || true

TMP_DIR="$(mktemp -d "$GIT_DIR_RESOLVED/${SUITE_NAME}-shards.XXXXXX")"

# Remove the scratch dir on a fully successful run. Retain it on any failure so
# the failing shards' logs and coverage report remain inspectable; the startup
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

echo "Running ${SUITE_NAME} in ${SHARDS} shard(s); package: ${PACKAGE}; test target: ${TEST_TARGET}; features: ${FEATURES}; timeout per shard: ${TIMEOUT_PER_SHARD}"
[[ -n "$TAG" ]] && echo "Tag filter: ${TAG}"
[[ "$REAL_LLM" == "1" ]] && echo "QUECTO_REAL_LLM=1"
echo "Logs: ${TMP_DIR}"

if [[ "$COVERAGE" == "1" ]]; then
    resolve_llvm_tools
    # A STABLE, suite-specific target dir: the instrumented build is reused
    # across runs (and kept by CI's rust-cache), unlike the per-run temp dir
    # the runner used before (#1203 leaked 114 GB of those). Per-suite so
    # concurrently running suites never clean or merge each other's profiles;
    # only the profraw output is reset per run (`clean --profraw-only`).
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}/llvm-cov-${SUITE_NAME}"
    eval "$(cargo llvm-cov show-env --sh)"
    cargo llvm-cov clean --profraw-only
    echo "Coverage: enabled; instrumented target: ${CARGO_TARGET_DIR}"
    [[ -n "$COVERAGE_THRESHOLD" ]] && echo "Coverage function threshold: ${COVERAGE_THRESHOLD}%"
fi

# ---------------------------------------------------------------------------
# Build once.
# ---------------------------------------------------------------------------
BUILD_JSON="$TMP_DIR/build.json"
BUILD_LOG="$TMP_DIR/build.log"
build_start="$(date +%s)"
set +e
cargo test --workspace --no-fail-fast --features "$FEATURES" --bins --test "$TEST_TARGET" \
    --no-run --message-format=json-render-diagnostics >"$BUILD_JSON" 2>"$BUILD_LOG"
build_code=$?
set -e
echo "build elapsed=$(( $(date +%s) - build_start ))s exit=${build_code}"
if [[ "$build_code" -ne 0 ]]; then
    echo "--- build.log (last 400 lines) ---"
    tail -n 400 "$BUILD_LOG" || true
    echo "--- end build.log ---"
    echo "${SUITE_NAME}: build failed" >&2
    exit 1
fi

# Resolve, from cargo's artifact messages: the test executable, the package
# directory it must run from (cargo runs integration tests with the package
# root as cwd; the cucumber runners read `tests/features` relative to it) and
# the package's bin executables (cargo exports `CARGO_BIN_EXE_<name>` for
# them; the e2e steps spawn `quecto` through that variable).
#
# The resolver also removes executables this build did not produce from the
# deps dir. `cargo llvm-cov report` feeds every executable under the profile
# dir to llvm-cov with no age filter, so in the persistent coverage target a
# superseded `bdd-<old hash>` would report its functions at zero and drag the
# threshold down after every source change (cargo-llvm-cov normally avoids
# this by cleaning the workspace packages before each run).
RESOLVED_FILE="$TMP_DIR/resolved.txt"
python3 - "$BUILD_JSON" "$TEST_TARGET" "$COVERAGE" >"$RESOLVED_FILE" <<'PY'
import json, os, sys
build_json, test_target, coverage = sys.argv[1], sys.argv[2], sys.argv[3] == "1"
artifacts = []
for line in open(build_json, encoding="utf-8"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    msg = json.loads(line)
    if msg.get("reason") == "compiler-artifact" and msg.get("executable"):
        artifacts.append(msg)
tests = [m for m in artifacts if "test" in m["target"]["kind"] and m["target"]["name"] == test_target]
if not tests:
    sys.exit(f"no test executable named {test_target!r} in {build_json}")
test_exe, manifest = tests[0]["executable"], tests[0]["manifest_path"]
bins = [f"CARGO_BIN_EXE_{m['target']['name']}={m['executable']}"
        for m in artifacts if m["manifest_path"] == manifest and "bin" in m["target"]["kind"]]
if coverage:
    produced = {m["executable"] for m in artifacts}
    deps_dir = os.path.dirname(test_exe)
    for name in os.listdir(deps_dir):
        path = os.path.join(deps_dir, name)
        if "." not in name and os.path.isfile(path) and os.access(path, os.X_OK) and path not in produced:
            os.remove(path)
            print(f"removed stale instrumented executable {path}", file=sys.stderr)
print(test_exe)
print(os.path.dirname(manifest))
print("\n".join(bins))
PY
mapfile -t RESOLVED <"$RESOLVED_FILE"
TEST_EXE="${RESOLVED[0]}"
PACKAGE_DIR="${RESOLVED[1]}"
BIN_ENV=("${RESOLVED[@]:2}")
# `<build root>/debug/deps/<exe>`: the build root is what cargo would have
# used as the target dir for this build (under --coverage it is
# `<target>/llvm-cov-target`), and steps that spawn `<target>/debug/quecto`
# read it from CARGO_TARGET_DIR at runtime.
DEPS_DIR="$(dirname "$TEST_EXE")"
PROFILE_DIR="$(dirname "$DEPS_DIR")"
BUILD_ROOT="$(dirname "$PROFILE_DIR")"
PKG_NAME="$(basename "$PACKAGE_DIR")"
PKG_VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$PACKAGE_DIR/Cargo.toml" | head -n1)"
mkdir -p "$BUILD_ROOT/tmp"
echo "Test binary: ${TEST_EXE}"

if [[ "$BUILD_ONLY" == "1" ]]; then
    RUN_STATUS="success"
    echo "${SUITE_NAME}: build only, not running shards."
    exit 0
fi

# ---------------------------------------------------------------------------
# Run many.
# ---------------------------------------------------------------------------
declare -a PIDS=()
declare -a SHARD_IDS=()

for i in $(seq 0 $((SHARDS - 1))); do
    (
        start="$(date +%s)"
        env_args=(
            "QUECTO_BDD_SHARD_INDEX=${i}"
            "QUECTO_BDD_SHARD_TOTAL=${SHARDS}"
            "CARGO_MANIFEST_DIR=${PACKAGE_DIR}"
            "CARGO_PKG_NAME=${PKG_NAME}"
            "CARGO_PKG_VERSION=${PKG_VERSION}"
            "CARGO_TARGET_DIR=${BUILD_ROOT}"
            "CARGO_TARGET_TMPDIR=${BUILD_ROOT}/tmp"
            "LD_LIBRARY_PATH=${DEPS_DIR}:${PROFILE_DIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        )
        [[ ${#BIN_ENV[@]} -gt 0 ]] && env_args+=("${BIN_ENV[@]}")
        [[ -n "$TAG" ]] && env_args+=("QUECTO_TAG=${TAG}")
        [[ "$REAL_LLM" == "1" ]] && env_args+=("QUECTO_REAL_LLM=1")
        if [[ "$COVERAGE" == "1" ]]; then
            # One profile file per process, in the directory `cargo llvm-cov
            # report` merges from; the shard index keeps the names distinct.
            env_args+=("LLVM_PROFILE_FILE=$(dirname "$LLVM_PROFILE_FILE")/${SUITE_NAME}-shard${i}-%p-%32m.profraw")
        fi

        cd "$PACKAGE_DIR"
        set +e
        timeout "$TIMEOUT_PER_SHARD" env "${env_args[@]}" "$TEST_EXE" 2>&1 | "$ROOT/scripts/test-filter.sh"
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
    if [[ -n "$COVERAGE_THRESHOLD" ]]; then
        report_args+=(--fail-under-functions "$COVERAGE_THRESHOLD")
    fi
    echo "Generating merged coverage report..."
    if ! cargo llvm-cov "${report_args[@]}" 2>&1 | tee "$TMP_DIR/coverage.txt"; then
        echo "${SUITE_NAME} coverage gate failed. Report: ${TMP_DIR}/coverage.txt" >&2
        exit 1
    fi
    echo "Coverage report: ${TMP_DIR}/coverage.txt"
    cargo llvm-cov clean --profraw-only
fi

RUN_STATUS="success"
echo "${SUITE_NAME} shards passed."
