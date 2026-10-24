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

# Read zero or one NUL-delimited entry while retaining Git command success.
# Metadata and literal filename are parsed separately by callers.
one_git_entry() {
    local -a entries=()
    mapfile -d '' -t entries < <("$@" 2>/dev/null && printf 'OK\0')
    case ${#entries[@]} in
        1) if [[ "${entries[0]}" == OK ]]; then ENTRY=''; return 0; fi ;;
        2) if [[ "${entries[1]}" == OK ]]; then ENTRY="${entries[0]}"; return 0; fi ;;
    esac
    return 1
}

merge_import_matches_index() {
    local file="$1" merge_head head type index_oid metadata name ENTRY
    local hash_length byte_count text_count matched=0
    local -a parents=()
    if merge_head=$(git rev-parse --git-path MERGE_HEAD) && [[ -f "$merge_head" ]] \
        && hash_length=$(git hash-object --stdin </dev/null); then
        hash_length=${#hash_length}
    else
        return 1
    fi
    # Bash read drops NUL bytes: affirm byte integrity before parsing lines.
    if byte_count=$(wc -c < "$merge_head") \
        && text_count=$(LC_ALL=C tr -d '\000' < "$merge_head" | wc -c) \
        && (( byte_count == text_count )); then :; else
        return 1
    fi
    # Validate the entire parent set before accepting any content match.
    while IFS= read -r head || [[ -n "$head" ]]; do
        if [[ "$head" =~ ^[0-9a-fA-F]+$ ]] && (( ${#head} == hash_length )) \
            && type=$(git cat-file -t "$head" 2>/dev/null) && [[ "$type" == commit ]]; then
            parents+=("$head")
        else
            return 1
        fi
    done < "$merge_head" || return 1
    (( ${#parents[@]} > 0 )) || return 1
    if one_git_entry git ls-files --stage -z -- ":(literal)$file"; then
        metadata=${ENTRY%%$'\t'*}; name=${ENTRY#*$'\t'}
        if [[ "$name" == "$file" && "$metadata" =~ ^100(644|755)\ ([0-9a-f]+)\ 0$ ]]; then
            index_oid=${BASH_REMATCH[2]}
        else
            return 1
        fi
    else
        return 1
    fi
    if type=$(git cat-file -t "$index_oid" 2>/dev/null) && [[ "$type" == blob ]]; then
        for head in "${parents[@]}"; do
            if one_git_entry git ls-tree -z "$head" -- ":(literal)$file"; then
                metadata=${ENTRY%%$'\t'*}; name=${ENTRY#*$'\t'}
                if [[ "$name" == "$file" && "$metadata" =~ ^100(644|755)\ blob\ ([0-9a-f]+)$ ]] \
                    && [[ "${BASH_REMATCH[2]}" == "$index_oid" ]]; then
                    matched=1
                fi
            else
                return 1
            fi
        done
    fi
    (( matched == 1 ))
}

FAILED=0
for file in "${STAGED[@]}"; do
    [[ -f "$file" ]] || continue
    case "$file" in
        *.rs)
            if grep -nE 'todo!\(\)|unimplemented!\(\)|FIXME|HACK|XXX|#\[allow\(|#\[ignore' "$file"; then
                echo -e "${RED}FAIL${NC}: forbidden work marker or lint/test bypass in $file"
                FAILED=1
            fi
            if git diff --cached --name-only --diff-filter=A -- ":(literal)$file" | grep -q . \
                && (( $(wc -l < "$file") > 750 )); then
                if merge_import_matches_index "$file"; then :; else
                    echo -e "${RED}FAIL${NC}: new file $file exceeds 750 lines"
                    FAILED=1
                fi
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
