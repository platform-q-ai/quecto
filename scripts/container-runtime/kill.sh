#!/usr/bin/env bash
# Canonical Quecto container-runtime reference script: `kill` / `cleanup`
# (#1369).
#
# One script serves both operations, distinguished by --op so the recorded
# state can prove WHICH operation ran (see docs/container-runtimes.md):
#   kill.sh --state-dir <dir> --op kill      # retained `kill` argv
#   kill.sh --state-dir <dir> --op cleanup   # retained `cleanup` argv
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID (the runtime environment_id
# reported by create.sh — NOT the session `C1` ref).
#
# Trusted-root containment: destructive removal only happens after proving
# the environment directory really resolves under the trusted --state-dir.
# A real adapter replaces only the marked section (e.g. `docker rm -f`).
set -euo pipefail
export LC_ALL=C

log() { printf 'container-runtime kill: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to read the recorded children"

state_dir=""
op="kill"
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || die "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  --op)
    [ "$#" -ge 2 ] || die "--op needs a value"
    op="$2"
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
case "$op" in kill | cleanup) ;; *) die "unknown --op: $op" ;; esac
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

# Trusted-root containment before anything destructive: the resolved
# environment directory must live under the resolved trusted state root.
state_root="$(cd "$state_dir" && pwd -P)"
env_real="$(cd "$env_dir" && pwd -P)"
case "$env_real" in
"$state_root"/*) ;;
*) die "refusing to destroy $env_real outside trusted root $state_root" ;;
esac

printf '%s\n' "$op" >>"$env_dir/kill.log"

# --- Runtime-specific section -------------------------------------------
# A real adapter tears the runtime environment down here (e.g.
# `docker rm -f`). The host-local reference terminates every recorded
# child process and removes the checked-out workspace.
if [ -f "$env_dir/children.jsonl" ]; then
  # One jq pass over the whole record, not one fork per line. A jq parse
  # failure yields no pids and the loop is a no-op by design. Reference
  # caveat: recorded pids can be reused by unrelated same-user processes on
  # long-lived environments; a real adapter kills the runtime container (a
  # stable handle) instead of raw pids.
  while IFS= read -r pid; do
    # Affirmative PID allowlist: malformed and privileged process IDs never
    # reach kill(2), even when state metadata is corrupted or attacker-written.
    case "$pid" in
      ''|*[!0-9]*) continue ;;
      0|1) continue ;;
    esac
    kill -9 -- "$pid" 2>/dev/null || true
  done < <(jq -r 'select(type == "object" and (.pid | type == "number") and (.pid | floor == .) and .pid > 1) | .pid' "$env_dir/children.jsonl")
fi
if [ "$op" = "cleanup" ]; then
  # cleanup is terminal: remove the ENTIRE per-environment state directory so
  # the state root does not accrete one directory per environment forever.
  # `kill` keeps the metadata (ref/kill.log/children.jsonl) for the cleanup
  # that follows it and for post-mortem inspection.
  rm -rf "$env_real"
else
  rm -rf "$env_real/workspace"
fi
# ------------------------------------------------------------------------
log "$op completed for $id"
