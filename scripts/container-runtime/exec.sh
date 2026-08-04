#!/usr/bin/env bash
set -euo pipefail
# Emits a machine-readable JSON result describing environment_id, workspace_path, container_ref, status/metadata.
env_id="${QUECTO_ENVIRONMENT_UUID:-${QUECTO_CONTAINER_REF:-}}"
socket_path="${QUECTO_SOCKET_PATH:-}"
read_only=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --environment-id) env_id="$2"; shift 2 ;;
    --socket-path) socket_path="$2"; shift 2 ;;
    --read-only) read_only="$2"; shift 2 ;;
    --script-name|--container-ref|--container-name) shift 2 ;;
    --) shift; break ;;
    *) echo "unknown exec flag: $1" >&2; exit 64 ;;
  esac
done
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
pid_file="$root/$env_id/child.pid"; mkdir -p "$(dirname "$pid_file")" "$(dirname "$socket_path")"
exec_started=false
if [[ $# -gt 0 ]]; then (cd "$workspace" && "$@") & echo "$!" > "$pid_file"; exec_started=true; fi
python3 - <<PY
import json
print(json.dumps({"environment_id":"$env_id","socket_path":"$socket_path","workspace_path":"$workspace","container_ref":"${QUECTO_CONTAINER_REF:-$env_id}","metadata":{"runtime":"docker-reference","exec_started":$exec_started}}))
PY
