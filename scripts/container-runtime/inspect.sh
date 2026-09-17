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
export LC_ALL=C

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
# SECURITY: retained state is trusted only when the root is an owner-only,
# non-symlink directory. Never resolve an attacker-controlled root implicitly.
case "$state_dir" in /*) ;; *) die "--state-dir must be an absolute path" ;; esac
if [ -e "$state_dir" ] || [ -L "$state_dir" ]; then
  [ -d "$state_dir" ] && [ ! -L "$state_dir" ] || die "state dir must be a real directory"
  [ -O "$state_dir" ] || die "state dir is not owned by the current user"
  [ "$(stat -c '%a' -- "$state_dir" 2>/dev/null)" = 700 ] || die "state dir must have mode 700"
else
  die "state dir does not exist"
fi
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"

# Same trusted-root containment as kill.sh: reject path-shaped ids and prove
# the environment directory resolves under the trusted state root before use.
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
# SECURITY: affirmative ASCII allowlist; IDs are names, never paths/options.
if [[ "$id" =~ ^[A-Za-z0-9]([A-Za-z0-9_.-]*[A-Za-z0-9])?$ ]]; then
  : # affirmative strict ASCII identifier allowlist
else
  die "invalid environment id: $id"
fi
env_dir="$state_dir/$id"
[ -d "$env_dir" ] && [ ! -L "$env_dir" ] || die "environment must be a real directory: $id"
[ -O "$env_dir" ] || die "environment is not owned by the current user: $id"
[ "$(stat -c '%a' -- "$env_dir" 2>/dev/null)" = 700 ] || die "environment must have mode 700: $id"
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
