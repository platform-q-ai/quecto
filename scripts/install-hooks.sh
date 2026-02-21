#!/usr/bin/env bash
# install-hooks.sh — Installs git hooks and the --no-verify wrapper.
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
HOOKS_DIR="$ROOT/.git/hooks"

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
install_hook "pre-merge-commit"

# Install git wrapper that bans --no-verify.
WRAPPER_DIR="$ROOT/.git/wrapper-bin"
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
echo "  source $(realpath "$ROOT/scripts/activate-hooks.sh")"
