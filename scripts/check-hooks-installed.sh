#!/usr/bin/env bash
# check-hooks-installed.sh — verifies local quality hooks and the --no-verify wrapper.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
FAIL=0

check_hook() {
    local hook_name="$1"
    local hook_path
    hook_path="$(git rev-parse --git-path "hooks/$hook_name")"
    local expected="exec \"\$(git rev-parse --show-toplevel)/scripts/${hook_name}.sh\""

    if [[ ! -x "$hook_path" ]]; then
        echo "FAIL: $hook_name hook is missing or not executable at $hook_path" >&2
        FAIL=1
        return
    fi

    if ! grep -Fqx "$expected" "$hook_path"; then
        echo "FAIL: $hook_name hook does not dispatch to scripts/${hook_name}.sh" >&2
        FAIL=1
        return
    fi

    echo "PASS: $hook_name hook installed"
}

check_hook pre-commit
check_hook pre-push

WRAPPER="$(git rev-parse --git-path wrapper-bin/git)"
if [[ ! -x "$WRAPPER" ]]; then
    echo "FAIL: git --no-verify wrapper missing at $WRAPPER" >&2
    FAIL=1
else
    echo "PASS: git wrapper installed"
fi

RESOLVED_GIT="$(command -v git || true)"
if [[ -z "$RESOLVED_GIT" ]]; then
    echo "FAIL: git is not available in PATH" >&2
    FAIL=1
elif [[ "$(realpath "$RESOLVED_GIT")" != "$(realpath "$WRAPPER")" ]]; then
    echo "FAIL: git wrapper is not first in PATH" >&2
    echo "Expected: $WRAPPER" >&2
    echo "Found:    $RESOLVED_GIT" >&2
    echo "Run: source scripts/activate-hooks.sh" >&2
    FAIL=1
else
    echo "PASS: git wrapper active in PATH"
fi

if (( FAIL != 0 )); then
    echo "Local quality hook installation check failed." >&2
    exit 1
fi

echo "All local quality hooks are installed and active."
