#!/usr/bin/env bash
set -euo pipefail
# JSON result: stdout emits exactly one machine-readable JSON object for Quecto.
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
python3 - "$root" "$workspace" <<'PY'
import os, pathlib, sys
root=pathlib.Path(sys.argv[1]).resolve(); ws=pathlib.Path(sys.argv[2])
if ws.exists(): ws=ws.resolve()
else: ws=ws.absolute();
if root != ws and root not in ws.parents:
    raise SystemExit(f"workspace {ws} is not contained by runtime root {root}")
PY
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
export env_id socket_path workspace container_ref repo read_only child_started pid_file
python3 - <<'PY'
import json, os
print(json.dumps({"environment_id":os.environ["env_id"],"socket_path":os.environ["socket_path"],"workspace_path":os.environ["workspace"],"container_ref":os.environ["container_ref"],"status":"running","metadata":{"repo":os.environ.get("repo",""),"runtime":"host-local-reference","uid":os.getuid(),"gid":os.getgid(),"read_only":os.environ.get("read_only") == "true","child_started":os.environ.get("child_started") == "true","pid_file":os.environ["pid_file"]}}))
PY