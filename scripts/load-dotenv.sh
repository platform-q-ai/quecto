#!/usr/bin/env bash
# load-dotenv.sh — best-effort loader for repo-local .env in hook scripts.
# Exports variables defined in .env without printing values.

set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
DOTENV_FILE="$ROOT/.env"

if [[ -f "$DOTENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$DOTENV_FILE"
    set +a
fi
