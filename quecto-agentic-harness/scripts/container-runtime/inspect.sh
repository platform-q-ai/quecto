#!/usr/bin/env bash
set -euo pipefail
# JSON result contract: inspect emits one machine-readable JSON result on stdout.
printf '{"environment_id":"%s","status":"%s","health":"%s","workspace_path":"%s","container_ref":"%s","metadata":{"runtime":"docker-reference"}}\n' "${QUECTO_ENVIRONMENT_UUID:-${1:-unknown}}" "${QUECTO_STATUS:-running}" "${QUECTO_HEALTH:-healthy}" "${QUECTO_WORKSPACE_PATH:-/workspace/quecto}" "${QUECTO_CONTAINER_REF:-}"
