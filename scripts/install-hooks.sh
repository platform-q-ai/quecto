#!/usr/bin/env bash
# install-hooks.sh — Installs git hooks that delegate to scripts/.
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

echo "Done. Hooks installed to $HOOKS_DIR"
