#!/usr/bin/env bash
# install-hooks.sh — Installs git hooks and the --no-verify wrapper.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
# Hooks and the wrapper live in the repository's shared git directory, so one
# install serves the main checkout and every linked worktree (where `.git` is
# a file, not a directory).
COMMON_DIR="$(git rev-parse --path-format=absolute --git-common-dir)"
HOOKS_DIR="$COMMON_DIR/hooks"
WRAPPER_DIR="$COMMON_DIR/wrapper-bin"

# A core.hooksPath elsewhere makes git ignore $HOOKS_DIR, and that directory
# may be shared with other repositories: never write into it.
ACTIVE_HOOKS="$(realpath -m "$(git rev-parse --path-format=absolute --git-path hooks)")"
if [[ "$ACTIVE_HOOKS" != "$(realpath -m "$HOOKS_DIR")" ]]; then
    echo "install-hooks.sh: core.hooksPath makes git use $ACTIVE_HOOKS, so it would ignore quecto's hooks in $HOOKS_DIR." >&2
    echo "Unset it (git config --unset core.hooksPath, or with --global) and re-run." >&2
    exit 1
fi
mkdir -p "$HOOKS_DIR"

install_hook() {
    local hook_name="$1"
    local hook_path="$HOOKS_DIR/$hook_name"

    cat > "$hook_path" << EOF
#!/usr/bin/env bash
exec "\$(git rev-parse --show-toplevel)/scripts/${hook_name}.sh"
EOF
    chmod +x "$hook_path"
    echo "Installed $hook_name hook"
}

install_hook "pre-commit"
install_hook "pre-push"

# Self-heal clones that still have the removed pre-merge-commit hook: its shim
# would exec the now-deleted scripts/pre-merge-commit.sh and abort `git merge`.
rm -f "$HOOKS_DIR/pre-merge-commit"
git config --unset merge.ff 2>/dev/null || true

# Install git wrapper that bans --no-verify.
mkdir -p "$WRAPPER_DIR"
cp "$ROOT/scripts/git-wrapper.sh" "$WRAPPER_DIR/git"
chmod +x "$WRAPPER_DIR/git"
echo "Installed git wrapper to $WRAPPER_DIR/git"

echo ""
echo "Done. Hooks installed to $HOOKS_DIR"
echo ""
echo "To activate the --no-verify ban in your current shell:"
echo "  source scripts/activate-hooks.sh"
echo ""
echo "To activate automatically, add to your .bashrc or .zshrc:"
# The main checkout outlives any linked worktree, so point the shell there.
# (A bare repository lists itself first and has no scripts: use this checkout.)
MAIN_ROOT="$(git worktree list --porcelain | sed -n '1s/^worktree //p')"
if [[ -f "$MAIN_ROOT/scripts/activate-hooks.sh" ]]; then
    ACTIVATE="$MAIN_ROOT/scripts/activate-hooks.sh"
else
    ACTIVATE="$ROOT/scripts/activate-hooks.sh"
fi
echo "  source $(realpath "$ACTIVATE")"
