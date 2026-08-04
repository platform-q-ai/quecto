#!/usr/bin/env bash
# check-quality.sh — Static analysis pass that blocks work markers, lint bypasses,
# unsafe blocks, and ignored tests. Runs on all Rust source files under workspace src/ trees.
set -euo pipefail

RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
NC='\033[0m'

FAILED=0

rust_source_files() {
    find . -path ./.git -prune -o -path ./target -prune -o -name '*.rs' -path '*/src/*' -print
}

check_pattern() {
    local pattern="$1"
    local description="$2"
    local matches

    matches=$(rust_source_files | xargs -r grep -n "$pattern" || true)
    if [ -n "$matches" ]; then
        echo -e "${RED}FAIL${NC}: Found $description"
        echo "$matches"
        echo ""
        FAILED=1
    fi
}

echo "=== Quality Gate ==="

check_pattern 'todo!()' 'todo!() macros (remove or implement before committing)'
check_pattern 'unimplemented!()' 'unimplemented!() macros (implement before committing)'
check_pattern 'FIXME' 'FIXME comments (resolve before committing)'
check_pattern 'HACK' 'HACK comments (resolve before committing)'
check_pattern 'XXX' 'XXX comments (resolve before committing)'
check_pattern '#\[allow(' '#[allow()] attributes (remove or justify)'
unsafe_matches=$(rust_source_files | xargs -r awk '
    /unsafe \{/ {
        if (prev !~ /SAFETY:/ && $0 !~ /SAFETY:/) {
            printf "%s:%d:%s\n", FILENAME, FNR, $0
        }
    }
    { prev = $0 }
' || true)
if [ -n "$unsafe_matches" ]; then
    echo -e "${RED}FAIL${NC}: Found unsafe blocks without // SAFETY: justification"
    echo "$unsafe_matches"
    echo ""
    FAILED=1
fi
check_pattern '#\[ignore' '#[ignore] on tests (all tests must run)'

# ── File size check: no new source file may exceed MAX_LINES lines ──
MAX_LINES=750
# Temporary ratchet baselines for issue #1369 container-spawn wiring. New files
# remain capped at MAX_LINES; these touched legacy seams must not grow further.
declare -A OVERSIZED_BASELINE=(
    ["quecto-tui/src/protocol/client.rs"]=772
    ["quecto-tui/src/agents/app_subagent_first_tests.rs"]=754
    ["quecto-tui/src/agents/controller_subagent_panel.rs"]=811
    ["quecto-agentic-harness/src/infrastructure/config_tests.rs"]=783
    ["quecto-agentic-harness/src/interface/cli/protocol_tests.rs"]=770
)

oversized=""
baseline_warnings=""
while IFS= read -r -d '' file; do
    file="${file#./}"
    lines=$(wc -l <"$file")
    baseline="${OVERSIZED_BASELINE[$file]:-}"

    if (( lines <= MAX_LINES )); then
        if [[ -n "$baseline" ]]; then
            oversized+="  $file: $lines lines (remove obsolete baseline $baseline; max $MAX_LINES)"$'\n'
        fi
        continue
    fi

    if [[ -n "$baseline" ]]; then
        if (( lines < baseline )); then
            oversized+="  $file: $lines lines (ratchet baseline down from $baseline; target max $MAX_LINES)"$'\n'
        elif (( lines == baseline )); then
            baseline_warnings+="  $file: $lines lines (baseline $baseline; target max $MAX_LINES)"$'\n'
        else
            oversized+="  $file: $lines lines (baseline $baseline; target max $MAX_LINES)"$'\n'
        fi
    else
        oversized+="  $file: $lines lines (max $MAX_LINES)"$'\n'
    fi
done < <(find . -path ./.git -prune -o -path ./target -prune -o -name '*.rs' -path '*/src/*' -print0)

if [ -n "$oversized" ]; then
    echo -e "${RED}FAIL${NC}: Source files exceed allowed line-count baseline"
    printf "%s" "$oversized"
    echo ""
    FAILED=1
fi

if [ -n "$baseline_warnings" ]; then
    echo -e "${YELLOW}WARN${NC}: Existing oversized source files remain grandfathered"
    printf "%s" "$baseline_warnings"
    echo ""
fi

# Warn (not block) if git wrapper is not active.
WRAPPER_DIR="$(git rev-parse --show-toplevel)/.git/wrapper-bin"
if ! echo "$PATH" | tr ':' '\n' | grep -qF "$WRAPPER_DIR"; then
    echo -e "${YELLOW}WARN${NC}: Git --no-verify wrapper is not active."
    echo "  Run: source scripts/activate-hooks.sh"
    echo ""
fi

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}PASS${NC}: Quality gate passed"
else
    echo -e "${RED}BLOCKED${NC}: Fix the issues above before committing"
    exit 1
fi
