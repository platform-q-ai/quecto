#!/usr/bin/env bash
set -euo pipefail
# Emits a machine-readable JSON result describing environment_id, workspace_path, container_ref, status/metadata.
env_id="${QUECTO_ENVIRONMENT_UUID:-${1:-}}"
workspace="${QUECTO_WORKSPACE_PATH:-${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}/$env_id/workspace}"
status="running"
health="healthy"
[[ -d "$workspace" ]] || { status="stopped"; health="missing"; }
printf '{"environment_id":"%s","status":"%s","health":"%s","workspace_path":"%s","container_ref":"%s","metadata":{"runtime":"docker-reference"}}\n' "$env_id" "$status" "$health" "$workspace" "${QUECTO_CONTAINER_REF:-$env_id}"
