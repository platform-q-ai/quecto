#!/usr/bin/env bash
set -euo pipefail
# JSON result: stdout emits exactly one machine-readable JSON object for Quecto.

env_id="${QUECTO_ENVIRONMENT_UUID:-${QUECTO_CONTAINER_UUID:-${1:-}}}"
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
case "$env_id" in ""|*/*|*..*|*$'\n'*|*$'\r'*) echo "unsafe environment id" >&2; exit 64 ;; esac
python3 - "$root" "$workspace" <<'PY'
import pathlib, sys
root=pathlib.Path(sys.argv[1]).resolve(); ws=pathlib.Path(sys.argv[2])
ws = ws.resolve() if ws.exists() else ws.absolute()
if root != ws and root not in ws.parents:
    raise SystemExit(f"refusing to clean workspace outside managed root: {ws}")
PY
root_real="$(mkdir -p "$root" && cd "$root" && pwd -P)"
pid_file="$root_real/$env_id/child.pid"
if [[ -f "$pid_file" ]]; then
  pid="$(cat "$pid_file")"
  kill "$pid" 2>/dev/null || true
  rm -f -- "$pid_file"
fi
rm -rf -- "$root_real/$env_id"
export env_id workspace
python3 - <<'PY'
import json, os
print(json.dumps({"environment_id":os.environ["env_id"],"status":"stopped","workspace_path":os.environ["workspace"],"container_ref":os.environ.get("QUECTO_CONTAINER_REF",os.environ["env_id"]),"metadata":{"runtime":"host-local-reference","cleaned":True}}))
PY