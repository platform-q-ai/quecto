#!/usr/bin/env bash
# check-quality.sh — Static analysis pass that blocks work markers, lint bypasses,
# unsafe blocks, and ignored tests. Runs on all .rs files under src/.
set -euo pipefail

RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
NC='\033[0m'

FAILED=0

check_pattern() {
    local pattern="$1"
    local description="$2"
    local matches

    matches=$(grep -rn "$pattern" src/ --include='*.rs' || true)
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
check_pattern 'unsafe {' 'unsafe blocks (require // SAFETY: justification)'
check_pattern '#\[ignore' '#[ignore] on tests (all tests must run)'

# ── File size check: no new source file may exceed MAX_LINES lines ──
MAX_LINES=750
# Existing oversized files are grandfathered at their current line counts so the
# gate can pass on master while still blocking new oversized files or growth in
# known hotspots. The baseline is a ratchet: if a file shrinks, update/remove
# its entry in this table in the same change.
declare -A OVERSIZED_BASELINE=(
    ["quecto-mcp/src/lib.rs"]=1119
    ["quecto-runtime-manager/src/infrastructure.rs"]=1559
    ["quecto-tui/src/interface/app.rs"]=2604
    ["quecto-tui/src/infrastructure/client.rs"]=775
    ["quecto-tui/src/interface/components/chat.rs"]=1604
    ["quecto-tui/src/interface/components/editor.rs"]=941
    ["quecto-tui/src/interface/components/markdown.rs"]=960
    ["src/application/agent_loop.rs"]=809
    ["src/application/agent_loop_tests.rs"]=1080
    ["src/domain/workflow/engine.rs"]=829
    ["src/infrastructure/tools/agent_cmd.rs"]=844
    ["src/infrastructure/tools/agent_cmd_tests.rs"]=902
    ["src/infrastructure/tools/spawn.rs"]=1051
    ["src/interface/cli/agent_tests.rs"]=856
    ["src/interface/cli/uds.rs"]=762
    ["src/interface/cli/uds_ext_protocol.rs"]=1073
    ["src/interface/cli/uds_multi.rs"]=910
    ["src/interface/cli/uds_tests.rs"]=762
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
