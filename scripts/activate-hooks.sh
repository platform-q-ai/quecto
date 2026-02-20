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

_QUECTO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || echo "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)")"
_QUECTO_WRAPPER_DIR="$_QUECTO_ROOT/.git/wrapper-bin"

mkdir -p "$_QUECTO_WRAPPER_DIR"
cp "$_QUECTO_ROOT/scripts/git-wrapper.sh" "$_QUECTO_WRAPPER_DIR/git"
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

unset _QUECTO_ROOT _QUECTO_WRAPPER_DIR
