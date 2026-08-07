#!/usr/bin/env bash
# Canonical Quecto container-runtime reference script: `exec` (#1369).
#
# Adds another agent to an EXISTING environment (see docs/container-runtimes.md):
#   exec-argv... -- <child-binary> <child-args...>
# Environment: QUECTO_CONTAINER_CONFIG (retained container-config name) and
# QUECTO_CONTAINER_ENVIRONMENT_ID (the runtime environment_id reported by
# create.sh — NOT the session `C1` ref).
#
# Host-local mode: the joining child runs directly on the host and shares the
# environment's workspace. A real adapter replaces only the marked section
# (e.g. `docker exec`) — the argv/JSON contract stays identical.
set -euo pipefail

log() { printf 'container-runtime exec: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the exec result"

state_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || die "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  --)
    shift
    break
    ;;
  *) die "unknown argument: $1" ;;
  esac
done
[ -n "$state_dir" ] || die "--state-dir is required"
[ "$#" -gt 0 ] || die "missing child command after --"
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"

# Same trusted-root containment as kill.sh: reject path-shaped ids and prove
# the environment directory resolves under the trusted state root before use.
case "$QUECTO_CONTAINER_ENVIRONMENT_ID" in
*/* | *..*) die "invalid environment id: $QUECTO_CONTAINER_ENVIRONMENT_ID" ;;
esac
env_dir="$state_dir/$QUECTO_CONTAINER_ENVIRONMENT_ID"
[ -d "$env_dir" ] || die "unknown environment: $QUECTO_CONTAINER_ENVIRONMENT_ID"
state_root="$(cd "$state_dir" && pwd -P)"
env_real="$(cd "$env_dir" && pwd -P)"
case "$env_real" in
"$state_root"/*) ;;
*) die "refusing to use $env_real outside trusted root $state_root" ;;
esac

socket_path=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then
    socket_path="$arg"
    break
  fi
  prev="$arg"
done
[ -n "$socket_path" ] || die "child command has no --socket argument"

# The joining child runs inside the environment's checkout (workspace/repo
# when create.sh cloned a repository, else the bare workspace).
workdir="$env_dir/workspace/repo"
[ -d "$workdir" ] || workdir="$env_dir/workspace"
[ -d "$workdir" ] || die "environment has no workspace: $QUECTO_CONTAINER_ENVIRONMENT_ID"

# --- Runtime-specific section -------------------------------------------
# A real adapter starts the child INSIDE the existing environment here
# (e.g. `docker exec`) exactly once. The host-local reference starts it
# directly, in the environment's shared checkout.
(cd "$workdir" && exec "$@") >/dev/null 2>&1 &
child_pid=$!
# ------------------------------------------------------------------------

jq -cn --argjson pid "$child_pid" --arg socket "$socket_path" \
  '{pid: $pid, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$QUECTO_CONTAINER_ENVIRONMENT_ID" >>"$state_dir/execs.log"

# Exactly one JSON object on stdout — encoded with a real JSON encoder.
jq -cn --arg socket "$socket_path" \
  '{metadata: {runtime: "host-local"}, socket_path: $socket}'
