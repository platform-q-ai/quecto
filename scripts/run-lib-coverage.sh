#!/usr/bin/env bash
# run-lib-coverage.sh — per-package unit-test function coverage from ONE build.
#
# Usage: run-lib-coverage.sh [--build-only] <package>:<function-threshold> ...
#   e.g. run-lib-coverage.sh quecto-agentic-harness:92 quecto-tui:95 quecto-api:95
#
# `cargo llvm-cov --lib -p <crate>` once per crate compiled three different
# instrumented feature unifications (each `-p` shape resolves its own
# dependency features; 110-141 crates rebuilt per switch even without
# `llvm-cov clean`). This script builds the workspace shape once
# (`--workspace --features quecto-agentic-harness/test-support --bins --lib`,
# the same unification every other test invocation uses), then for each
# package runs only that package's lib test binary and reports `-p <package>`
# over its profiles alone, so every number is still the package's own tests
# covering the package's own code, as before. The instrumented target dir is
# stable (`target/llvm-cov-lib`) so CI's rust-cache keeps it.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

# `cargo llvm-cov report` has no `--lib` selector: the bin roots built by
# `--bins` would otherwise enter the report (they never did under
# `cargo llvm-cov --lib`), so `src/main.rs` is excluded explicitly.
IGNORE_REGEX='(tui_harness|test_support|warn_capture|src/main\.rs$)'
FEATURES="quecto-agentic-harness/test-support"
BUILD_ONLY="0"
declare -a SPECS=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --build-only) BUILD_ONLY="1"; shift ;;
        --ignore-filename-regex) IGNORE_REGEX="$2"; shift 2 ;;
        --features) FEATURES="$2"; shift 2 ;;
        -*) echo "Unknown arg: $1" >&2; exit 2 ;;
        *) SPECS+=("$1"); shift ;;
    esac
done
if [[ "$BUILD_ONLY" == "0" && ${#SPECS[@]} -eq 0 ]]; then
    echo "usage: $0 [--build-only] <package>:<function-threshold> ..." >&2
    exit 2
fi

if [[ -z "${LLVM_COV:-}" ]] && command -v llvm-cov &>/dev/null; then
    export LLVM_COV="$(command -v llvm-cov)"
fi
if [[ -z "${LLVM_PROFDATA:-}" ]] && command -v llvm-profdata &>/dev/null; then
    export LLVM_PROFDATA="$(command -v llvm-profdata)"
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}/llvm-cov-lib"
eval "$(cargo llvm-cov show-env --sh)"
cargo llvm-cov clean --profraw-only

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/lib-coverage.XXXXXX")"
trap 'rm -rf -- "$SCRATCH"' EXIT

build_start="$(date +%s)"
if ! cargo test --workspace --no-fail-fast --features "$FEATURES" --bins --lib \
    --no-run --message-format=json-render-diagnostics >"$SCRATCH/build.json" 2>"$SCRATCH/build.log"; then
    tail -n 400 "$SCRATCH/build.log" || true
    echo "lib coverage: build failed" >&2
    exit 1
fi
echo "build elapsed=$(( $(date +%s) - build_start ))s target: ${CARGO_TARGET_DIR}"
if [[ "$BUILD_ONLY" == "1" ]]; then
    echo "lib coverage: build only."
    exit 0
fi

# One line per lib test binary: <package name>\t<manifest dir>\t<executable>
python3 - "$SCRATCH/build.json" <<'PY' >"$SCRATCH/libs.tsv"
import json, os, sys
for line in open(sys.argv[1], encoding="utf-8"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    msg = json.loads(line)
    if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
        continue
    kind = msg["target"]["kind"]
    if msg["profile"]["test"] and ("lib" in kind or "rlib" in kind):
        pkg = os.path.basename(os.path.dirname(msg["manifest_path"]))
        print(f"{pkg}\t{os.path.dirname(msg['manifest_path'])}\t{msg['executable']}")
PY

FAIL=0
for spec in "${SPECS[@]}"; do
    package="${spec%%:*}"
    threshold="${spec##*:}"
    line="$(awk -F'\t' -v p="$package" '$1 == p {print; exit}' "$SCRATCH/libs.tsv")"
    if [[ -z "$line" ]]; then
        echo "no lib test binary for package ${package}" >&2
        exit 1
    fi
    pkg_dir="$(cut -f2 <<<"$line")"
    exe="$(cut -f3 <<<"$line")"
    deps_dir="$(dirname "$exe")"
    profile_dir="$(dirname "$deps_dir")"
    cargo llvm-cov clean --profraw-only
    echo "== ${package}: running $(basename "$exe")"
    run_start="$(date +%s)"
    set +e
    (
        cd "$pkg_dir"
        env "CARGO_MANIFEST_DIR=${pkg_dir}" \
            "CARGO_TARGET_DIR=$(dirname "$profile_dir")" \
            "LD_LIBRARY_PATH=${deps_dir}:${profile_dir}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
            "LLVM_PROFILE_FILE=$(dirname "$LLVM_PROFILE_FILE")/${package}-%p-%32m.profraw" \
            "$exe" </dev/null 2>&1 | "$ROOT/scripts/test-filter.sh"
        exit "${PIPESTATUS[0]}"
    )
    code=$?
    set -e
    echo "${package} tests exit=${code} elapsed=$(( $(date +%s) - run_start ))s"
    if [[ "$code" -ne 0 ]]; then
        FAIL=1
    fi
    if ! cargo llvm-cov report -p "$package" --ignore-filename-regex "$IGNORE_REGEX" \
        --fail-under-functions "$threshold" --summary-only; then
        echo "${package}: function coverage below ${threshold}%" >&2
        FAIL=1
    fi
done
cargo llvm-cov clean --profraw-only
if [[ "$FAIL" -ne 0 ]]; then
    echo "lib coverage failed." >&2
    exit 1
fi
echo "lib coverage passed."
