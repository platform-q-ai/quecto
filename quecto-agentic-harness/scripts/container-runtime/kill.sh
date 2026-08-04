#!/usr/bin/env bash
set -euo pipefail

env_id="${QUECTO_ENVIRONMENT_UUID:-${QUECTO_CONTAINER_UUID:-${1:-}}}"
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
root_real="$(mkdir -p "$root" && cd "$root" && pwd -P)"
case "$env_id" in
  ""|*/*|*..*|*$'\n'*|*$'\r'*) echo "unsafe environment id" >&2; exit 64 ;;
esac
workspace_parent="$(dirname "$workspace")"
mkdir -p "$workspace_parent"
parent_real="$(cd "$workspace_parent" && pwd -P)"
workspace_real="$parent_real/$(basename "$workspace")"
case "$workspace_real" in
  "$root_real"/*/workspace) ;;
  *) echo "refusing to clean workspace outside managed root" >&2; exit 64 ;;
esac
pid_file="$root_real/$env_id/child.pid"
if [[ -f "$pid_file" ]]; then
  pid="$(cat "$pid_file")"
  kill "$pid" 2>/dev/null || true
  rm -f -- "$pid_file"
fi
rm -rf -- "$root_real/$env_id"
printf '{"environment_id":"%s","status":"stopped","workspace_path":"%s","container_ref":"%s","metadata":{"runtime":"docker-reference","cleaned":true}}\n' "$env_id" "$workspace" "${QUECTO_CONTAINER_REF:-$env_id}"
