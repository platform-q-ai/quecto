#!/usr/bin/env bash
# Canonical Quecto container-runtime reference script: `inspect` (#1369).
#
# Post-mortem diagnostics (see docs/container-runtimes.md): Quecto runs this
# EXACTLY once per pushed member death (EOF/reset on the monitor connection),
# with QUECTO_CONTAINER_ENVIRONMENT_ID set to the runtime environment_id.
# It must print exactly one JSON object: {"status": ..., "metadata": {...}}.
#
# Host-local mode reports whether any recorded child process is still alive.
# A real adapter replaces only the marked section (e.g. `docker inspect`).
set -euo pipefail

log() { printf 'container-runtime inspect: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the inspect result"

state_dir=""
list=0
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || die "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  --list)
    list=1
    shift
    ;;
  *) die "unknown argument: $1" ;;
  esac
done
[ -n "$state_dir" ] || die "--state-dir is required"
if [ "$list" = 1 ]; then
  # `--list` (#2024 S4d, `quecto container gc`): one JSON object per
  # environment the runtime knows. The host-local reference knows exactly
  # the recorded environment directories; status is the recorded pids'
  # liveness.
  for dir in "$state_dir"/env-*; do
    [ -d "$dir" ] || continue
    id="$(basename "$dir")"
    status="dead"
    if [ -f "$dir/children.jsonl" ]; then
      while IFS= read -r pid; do
        if kill -0 "$pid" 2>/dev/null; then status="running"; break; fi
      done < <(jq -r '.pid' "$dir/children.jsonl" 2>/dev/null)
    fi
    jq -cn --arg id "$id" --arg status "$status" \
      '{environment_id: $id, container: $id, status: $status}'
  done
  exit 0
fi
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"

# Same trusted-root containment as kill.sh: reject path-shaped ids and prove
# the environment directory resolves under the trusted state root before use.
case "$QUECTO_CONTAINER_ENVIRONMENT_ID" in
*/* | *..*) die "invalid environment id: $QUECTO_CONTAINER_ENVIRONMENT_ID" ;;
esac
env_dir="$state_dir/$QUECTO_CONTAINER_ENVIRONMENT_ID"
if [ ! -d "$env_dir" ]; then
  # State gone (manual removal, a completed kill whose record outlived it):
  # a truthful "dead" so a registry restore (#2024 S4d) marks the record
  # stopped rather than keeping it unverified.
  jq -cn '{status: "dead", metadata: {cause: "environment-removed"}}'
  exit 0
fi
state_root="$(cd "$state_dir" && pwd -P)"
env_real="$(cd "$env_dir" && pwd -P)"
case "$env_real" in
"$state_root"/*) ;;
*) die "refusing to inspect $env_real outside trusted root $state_root" ;;
esac

printf 'inspect\n' >>"$env_dir/inspect.log"

# --- Runtime-specific section -------------------------------------------
# A real adapter queries the runtime here (e.g. `docker inspect`). The
# host-local reference checks the recorded child pids.
status="dead"
if [ -f "$env_dir/children.jsonl" ]; then
  # One jq pass over the whole record, not one fork per line. A jq parse
  # failure yields no pids, degrading to "dead" by design (truthful for a
  # corrupted record: nothing provably alive).
  while IFS= read -r pid; do
    if kill -0 "$pid" 2>/dev/null; then
      status="running"
      break
    fi
  done < <(jq -r '.pid' "$env_dir/children.jsonl")
fi
# ------------------------------------------------------------------------

# Exactly one JSON object on stdout — encoded with a real JSON encoder.
jq -cn --arg status "$status" \
  '{status: $status, metadata: {inspected_by: "scripts/container-runtime/inspect.sh"}}'
