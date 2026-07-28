#!/usr/bin/env bash
# pre-commit.sh — Cheap commit-time hygiene only. Keep this gate below 5 seconds.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

# Fail before doing any work when committing on the protected branch.
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
if [[ "$CURRENT_BRANCH" == "master" || "$CURRENT_BRANCH" == "main" ]]; then
    echo -e "${RED}ERROR: Direct commits to ${CURRENT_BRANCH} are not allowed.${NC}"
    exit 1
fi

mapfile -d '' -t STAGED < <(git diff --cached --name-only --diff-filter=ACMR -z)
if (( ${#STAGED[@]} == 0 )); then
    echo -e "${GREEN}Pre-commit passed (nothing staged).${NC}"
    exit 0
fi

FAILED=0
for file in "${STAGED[@]}"; do
    [[ -f "$file" ]] || continue
    case "$file" in
        *.rs)
            if grep -nE 'todo!\(\)|unimplemented!\(\)|FIXME|HACK|XXX|#\[allow\(|#\[ignore' "$file"; then
                echo -e "${RED}FAIL${NC}: forbidden work marker or lint/test bypass in $file"
                FAILED=1
            fi
            if git diff --cached --name-only --diff-filter=A -- "$file" | grep -q . \
                && (( $(wc -l < "$file") > 750 )); then
                echo -e "${RED}FAIL${NC}: new file $file exceeds 750 lines"
                FAILED=1
            fi
            if awk '/unsafe \{/ && previous !~ /SAFETY:/ && $0 !~ /SAFETY:/ { exit 1 } { previous=$0 }' "$file"; then :; else
                echo -e "${RED}FAIL${NC}: unsafe block without adjacent SAFETY justification in $file"
                FAILED=1
            fi
            ;;
    esac
done

if printf '%s\0' "${STAGED[@]}" | grep -zEq '\.(rs|toml)$'; then
    cargo fmt --all -- --check || FAILED=1
fi

if printf '%s\0' "${STAGED[@]}" | grep -zEq '(^|/)(tests/bdd/.*\.rs|tests/features/.*\.feature)$'; then
    "$ROOT/scripts/check-bdd-quality.sh" || FAILED=1
    "$ROOT/scripts/check-bdd-tags.sh" || FAILED=1
fi

(( FAILED == 0 )) || exit 1
echo -e "${GREEN}Pre-commit passed.${NC}"
