#!/usr/bin/env bash
set -euo pipefail
# Emits a machine-readable JSON result describing environment_id, workspace_path, container_ref, status/metadata.
repo="${QUECTO_REPO_URL:-}"
socket_path="${QUECTO_SOCKET_PATH:-}"
read_only="false"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) repo="$2"; shift 2 ;;
    --socket-path) socket_path="$2"; shift 2 ;;
    --read-only) read_only="$2"; shift 2 ;;
    --script-name) shift 2 ;;
    --) shift; break ;;
    *) echo "unknown create flag: $1" >&2; exit 64 ;;
  esac
done
: "${socket_path:=/tmp/quecto-${QUECTO_AGENT_UUID:-agent}.sock}"
env_id="${QUECTO_ENVIRONMENT_UUID:-quecto-${QUECTO_AGENT_UUID:-$(date +%s)}}"
container_ref="${QUECTO_CONTAINER_REF:-$env_id}"
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
if [[ -n "$repo" ]]; then
  case "$repo" in -*) echo "unsafe repository: leading dash is not allowed" >&2; exit 64 ;; *$'\n'*|*$'\r'*|*$'\t'*) echo "unsafe repository: control characters are not allowed" >&2; exit 64 ;; esac
fi
mkdir -p "$workspace" "$(dirname "$socket_path")"
if [[ -n "$repo" ]]; then
  if [[ -d "$workspace/.git" ]]; then git -C "$workspace" fetch --all --prune >&2 || true; git -C "$workspace" pull --ff-only >&2 || true
  elif [[ ! -e "$workspace/.git" ]]; then rm -rf -- "$workspace"; git clone -- "$repo" "$workspace" >&2; fi
fi
pid_file="$root/$env_id/child.pid"; mkdir -p "$(dirname "$pid_file")"
child_started=false
if [[ $# -gt 0 ]]; then (cd "$workspace" && "$@") & echo "$!" > "$pid_file"; child_started=true; fi
python3 - <<PY
import json
print(json.dumps({"environment_id":"$env_id","socket_path":"$socket_path","workspace_path":"$workspace","container_ref":"$container_ref","metadata":{"repo":"$repo","runtime":"docker-reference","uid":$(id -u),"gid":$(id -g),"read_only":$read_only,"child_started":$child_started,"pid_file":"$pid_file"}}))
PY
