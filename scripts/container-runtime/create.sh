#!/usr/bin/env bash
set -euo pipefail
# JSON result contract: create emits one machine-readable JSON result on stdout.
repo=${QUECTO_REPO_URL:-${1:-}}
env_id=${QUECTO_ENVIRONMENT_UUID:-quecto-$(date +%s)}
workspace=${QUECTO_WORKSPACE_PATH:-/workspace/quecto}
printf '{"environment_id":"%s","socket_path":"%s","workspace_path":"%s","container_ref":"%s","metadata":{"repo":"%s","runtime":"docker-reference"}}\n' "$env_id" "${QUECTO_SOCKET_PATH:-/tmp/quecto.sock}" "$workspace" "${QUECTO_CONTAINER_REF:-}" "$repo"
