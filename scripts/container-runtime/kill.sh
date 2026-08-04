#!/usr/bin/env bash
set -euo pipefail
env_id="${QUECTO_ENVIRONMENT_UUID:-${QUECTO_CONTAINER_UUID:-${1:-}}}"
workspace="${QUECTO_WORKSPACE_PATH:-${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}/$env_id/workspace}"
rm -rf "${workspace%/workspace}"
printf '{"environment_id":"%s","status":"stopped","workspace_path":"%s","container_ref":"%s","metadata":{"runtime":"docker-reference","cleaned":true}}\n' "$env_id" "$workspace" "${QUECTO_CONTAINER_REF:-$env_id}"
