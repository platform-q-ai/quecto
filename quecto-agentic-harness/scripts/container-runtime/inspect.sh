#!/usr/bin/env bash
set -euo pipefail
# JSON result: stdout emits exactly one machine-readable JSON object for Quecto.
env_id="${QUECTO_ENVIRONMENT_UUID:-${QUECTO_CONTAINER_UUID:-${1:-}}}"
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
pid_file="$root/$env_id/child.pid"
status="stopped"; health="exited"
if [[ -f "$pid_file" ]] && kill -0 "$(cat "$pid_file")" 2>/dev/null; then status="running"; health="healthy"; fi
export env_id status health workspace
python3 - <<'PY'
import json, os
print(json.dumps({"environment_id":os.environ["env_id"],"status":os.environ["status"],"health":os.environ["health"],"workspace_path":os.environ["workspace"],"container_ref":os.environ.get("QUECTO_CONTAINER_REF",os.environ["env_id"]),"metadata":{"runtime":"host-local-reference"}}))
PY