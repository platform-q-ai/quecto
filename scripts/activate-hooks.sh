#!/usr/bin/env bash
# activate-hooks.sh — Activates the git wrapper that bans --no-verify.
#
# Usage: source scripts/activate-hooks.sh
#
# This copies git-wrapper.sh into the repository's shared git directory
# (<git-common-dir>/wrapper-bin/git) and prepends that directory to PATH so
# `git` resolves to the wrapper in every worktree.
#
# The wrapper intercepts `git commit` and `git push` to reject --no-verify.
# All other git commands pass through to the real binary.
#
# To deactivate: start a new shell, or remove the temp dir from PATH.

# Resolve from the script's own checkout, not the caller's directory: a shell
# rc file sources this from $HOME. The wrapper lives in the repository's shared
# git directory, so one activation covers every worktree.
# bash names the sourced file in BASH_SOURCE; zsh through the %x prompt escape.
if [[ -n "${BASH_SOURCE[0]:-}" ]]; then
    _QUECTO_SOURCE="${BASH_SOURCE[0]}"
elif [[ -n "${ZSH_VERSION:-}" ]]; then
    eval '_QUECTO_SOURCE="${(%):-%x}"'
else
    _QUECTO_SOURCE=""
fi
# CDPATH would make `cd` print the directory it found into the substitution.
_QUECTO_SCRIPTS="$(CDPATH='' cd -- "$(dirname -- "$_QUECTO_SOURCE")" >/dev/null 2>&1 && pwd)"
_QUECTO_COMMON_DIR="$(git -C "$_QUECTO_SCRIPTS" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)"

if [[ "$_QUECTO_COMMON_DIR" == /* && -f "$_QUECTO_SCRIPTS/git-wrapper.sh" ]]; then
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
    unset _QUECTO_SOURCE _QUECTO_SCRIPTS _QUECTO_COMMON_DIR _QUECTO_WRAPPER_DIR
else
    echo "activate-hooks.sh: could not find scripts/git-wrapper.sh inside a git checkout; the --no-verify wrapper was not activated." >&2
    unset _QUECTO_SOURCE _QUECTO_SCRIPTS _QUECTO_COMMON_DIR
    return 1 2>/dev/null || exit 1
fi
