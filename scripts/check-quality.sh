#!/usr/bin/env bash
# check-quality.sh — Static analysis pass that blocks work markers, lint bypasses,
# unsafe blocks, and ignored tests. Runs on all .rs files under src/.
set -euo pipefail

RED='\033[0;31m'
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

# Warn (not block) if git wrapper is not active.
YELLOW='\033[0;33m'
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
