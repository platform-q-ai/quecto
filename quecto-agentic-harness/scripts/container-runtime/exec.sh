#!/usr/bin/env bash
set -euo pipefail
env_id="${QUECTO_ENVIRONMENT_UUID:-}"
socket_path="${QUECTO_SOCKET_PATH:-}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --environment-id) env_id="$2"; shift 2 ;;
    --socket-path) socket_path="$2"; shift 2 ;;
    --container-ref|--container-name|--script-name|--read-only) shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
: "${env_id:=quecto-${QUECTO_AGENT_UUID:-agent}}"
: "${socket_path:=/tmp/quecto-${QUECTO_AGENT_UUID:-agent}.sock}"
workspace="${QUECTO_WORKSPACE_PATH:-${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}/$env_id/workspace}"
mkdir -p "$workspace" "$(dirname "$socket_path")"
if [[ $# -gt 0 ]]; then
  (cd "$workspace" && "$@") &
fi
printf '{"environment_id":"%s","socket_path":"%s","workspace_path":"%s","container_ref":"%s","metadata":{"runtime":"docker-reference","exec_started":%s}}\n' "$env_id" "$socket_path" "$workspace" "${QUECTO_CONTAINER_REF:-$env_id}" "$([[ $# -gt 0 ]] && echo true || echo false)"
