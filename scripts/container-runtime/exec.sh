#!/usr/bin/env bash
set -euo pipefail
# JSON result: stdout emits exactly one machine-readable JSON object for Quecto.
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
python3 - "$root" "$workspace" <<'PY'
import pathlib, sys
root=pathlib.Path(sys.argv[1]).resolve(); ws=pathlib.Path(sys.argv[2])
ws = ws.resolve() if ws.exists() else ws.absolute()
if root != ws and root not in ws.parents:
    raise SystemExit(f"workspace {ws} is not contained by runtime root {root}")
PY
pid_file="$root/$env_id/child.pid"; mkdir -p "$(dirname "$pid_file")" "$(dirname "$socket_path")"
exec_started=false
if [[ $# -gt 0 ]]; then (cd "$workspace" && "$@") & echo "$!" > "$pid_file"; exec_started=true; fi
export env_id socket_path workspace exec_started
python3 - <<'PY'
import json, os
print(json.dumps({"environment_id":os.environ["env_id"],"socket_path":os.environ["socket_path"],"workspace_path":os.environ["workspace"],"container_ref":os.environ.get("QUECTO_CONTAINER_REF",os.environ["env_id"]),"status":"running","metadata":{"runtime":"host-local-reference","exec_started":os.environ.get("exec_started") == "true"}}))
PY