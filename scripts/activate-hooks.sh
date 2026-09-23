#!/usr/bin/env bash
# activate-hooks.sh — Activates the git wrapper that bans --no-verify.
#
# Usage: source scripts/activate-hooks.sh
#
# This creates a temporary bin directory with a symlink to git-wrapper.sh
# and prepends it to PATH so `git` resolves to the wrapper.
#
# The wrapper intercepts `git commit` and `git push` to reject --no-verify.
# All other git commands pass through to the real binary.
#
# To deactivate: start a new shell, or remove the temp dir from PATH.

# Resolve from the script's own checkout, not the caller's directory: a shell
# rc file sources this from $HOME. The wrapper lives in the repository's shared
# git directory, so one activation covers every worktree.
_QUECTO_SCRIPTS="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_QUECTO_COMMON_DIR="$(git -C "$_QUECTO_SCRIPTS" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"

if [[ "$_QUECTO_COMMON_DIR" == /* ]]; then
    _QUECTO_WRAPPER_DIR="$_QUECTO_COMMON_DIR/wrapper-bin"
    mkdir -p "$_QUECTO_WRAPPER_DIR"
    cp "$_QUECTO_SCRIPTS/git-wrapper.sh" "$_QUECTO_WRAPPER_DIR/git"
    chmod +x "$_QUECTO_WRAPPER_DIR/git"

    # Prepend to PATH (only if not already there).
    case ":$PATH:" in
        *":$_QUECTO_WRAPPER_DIR:"*)
            ;;
        *)
            export PATH="$_QUECTO_WRAPPER_DIR:$PATH"
            echo "Git wrapper activated. --no-verify is now banned for commit/push."
            ;;
    esac
    unset _QUECTO_SCRIPTS _QUECTO_COMMON_DIR _QUECTO_WRAPPER_DIR
else
    echo "activate-hooks.sh: $_QUECTO_SCRIPTS is not inside a git checkout; the --no-verify wrapper was not activated." >&2
    unset _QUECTO_SCRIPTS _QUECTO_COMMON_DIR
    return 1 2>/dev/null || exit 1
fi
