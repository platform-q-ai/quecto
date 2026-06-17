#!/usr/bin/env bash
# git-wrapper.sh — Wraps the real git binary to enforce hook execution.
#
# Intercepts `git commit` and `git push` to reject --no-verify / -n flags.
# All other commands and flags pass through transparently.
#
# Usage:
#   source scripts/activate-hooks.sh   (adds this wrapper to PATH)
#   git commit -m "msg"                (works)
#   git commit --no-verify -m "msg"    (blocked)
#   git push --no-verify               (blocked)
#
# To bypass in genuine emergencies, use the real binary directly:
#   command git commit --no-verify -m "emergency"

set -euo pipefail

RED='\033[0;31m'
NC='\033[0m'

# Find the real git binary (skip this wrapper, which should be first in PATH).
SELF="$(realpath "$0")"
REAL_GIT=""
while IFS= read -r candidate; do
    if [[ "$(realpath "$candidate")" != "$SELF" ]]; then
        REAL_GIT="$candidate"
        break
    fi
done < <(type -aP git)

if [[ -z "$REAL_GIT" ]]; then
    echo "ERROR: could not locate real git binary behind wrapper" >&2
    exit 127
fi

# Commands where --no-verify is dangerous.
HOOK_COMMANDS="commit push"

ban_no_verify() {
    local cmd="$1"
    shift
    for arg in "$@"; do
        case "$arg" in
            --no-verify|-n)
                echo -e "${RED}ERROR: --no-verify is banned in this repository.${NC}" >&2
                echo "Hooks enforce quality gates (fmt, clippy, tests, security checks)." >&2
                echo "Skipping them risks breaking master and is not allowed." >&2
                echo "" >&2
                echo "If you have a genuine emergency, use: command git $cmd $*" >&2
                return 1
                ;;
            --)
                # Stop parsing flags after --
                break
                ;;
        esac
    done
    return 0
}

# Check if the first argument is a command we care about.
if [[ $# -ge 1 ]]; then
    case "$1" in
        commit|push)
            ban_no_verify "$@" || exit 1
            ;;
    esac
fi

exec "$REAL_GIT" "$@"
