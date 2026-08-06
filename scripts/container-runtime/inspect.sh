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
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || die "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  *) die "unknown argument: $1" ;;
  esac
done
[ -n "$state_dir" ] || die "--state-dir is required"
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"

env_dir="$state_dir/$QUECTO_CONTAINER_ENVIRONMENT_ID"
[ -d "$env_dir" ] || die "unknown environment: $QUECTO_CONTAINER_ENVIRONMENT_ID"

printf 'inspect\n' >>"$env_dir/inspect.log"

# --- Runtime-specific section -------------------------------------------
# A real adapter queries the runtime here (e.g. `docker inspect`). The
# host-local reference checks the recorded child pids.
status="dead"
if [ -f "$env_dir/children.jsonl" ]; then
  # One jq pass over the whole record, not one fork per line.
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
