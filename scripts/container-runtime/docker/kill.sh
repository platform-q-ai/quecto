#!/usr/bin/env bash
# Official Docker adapter for the Quecto container-runtime contract: `kill`/`cleanup`.
#   kill.sh --state-dir <dir> --op kill      # retained `kill` argv
#   kill.sh --state-dir <dir> --op cleanup   # retained `cleanup` argv
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Removes the environment's container (force) and its state directory,
# after proving the directory resolves under the trusted state root.
set -euo pipefail

log() { printf 'container-runtime-docker kill: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

# Runtime CLI: rootless Podman by default. Membership of the `docker` group
# is root-equivalent on the host (the daemon runs as root and has no policy
# layer, so anything holding the socket can mount / and escalate), which is
# exactly what an autonomous agent spawner must not hand out. Rootless
# Podman runs the container as the invoking user with a user namespace, so
# an escape lands as that user, not root. QUECTO_CONTAINER_CLI overrides;
# Docker stays a fallback for hosts without Podman.
cli="${QUECTO_CONTAINER_CLI:-}"
if [ -z "$cli" ]; then
  if command -v podman >/dev/null 2>&1; then
    cli=podman
  elif command -v docker >/dev/null 2>&1; then
    cli=docker
  fi
fi
[ -n "$cli" ] && command -v "$cli" >/dev/null 2>&1 || die "podman (preferred) or docker is required"

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
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
[ -n "$id" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"
case "$id" in
*/* | *..*) die "invalid environment id: $id" ;;
esac
env_dir="$state_dir/$id"
if [ ! -d "$env_dir" ]; then
  # Already gone (e.g. cleanup after a kill): succeed idempotently.
  printf '%s %s\n' "$op" "$id" >>"$state_dir/kill.log" 2>/dev/null || true
  exit 0
fi
resolved="$(cd "$env_dir" && pwd -P)"
root="$(cd "$state_dir" && pwd -P)"
case "$resolved" in
"$root"/*) ;;
*) die "environment $id escapes the state root" ;;
esac

container="$(cat "$env_dir/container" 2>/dev/null || true)"
if [ -n "$container" ]; then
  # Docker's `rm -f` kills immediately; Podman's sends SIGTERM and waits the
  # container's stop timeout (10s) before SIGKILL. The parent runs this from
  # its own SIGTERM handling inside the TUI's two-second exit budget, so
  # bound the grace: one second for the child to cascade, then SIGKILL.
  grace=()
  [ "$cli" = podman ] && grace=(--time 1)
  "$cli" rm -f "${grace[@]}" "$container" >/dev/null 2>&1 || true
fi
rm -rf "$env_dir"
printf '%s %s\n' "$op" "$id" >>"$state_dir/kill.log"
