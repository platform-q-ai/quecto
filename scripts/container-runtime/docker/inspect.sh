#!/usr/bin/env bash
# Official Docker adapter for the Quecto container-runtime contract: `inspect`.
#   inspect.sh --state-dir <dir>          # one environment (QUECTO_CONTAINER_ENVIRONMENT_ID)
#   inspect.sh --state-dir <dir> --list   # every environment the runtime knows
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID (not needed with --list)
#
# Reports the container's truth post-mortem. Bounded by Quecto's 5s
# inspect timeout, so only cheap `podman`/`docker inspect` calls happen here.
# `--list` (#2024 S4d, `quecto container gc`) prints one JSON object per
# container carrying the `quecto.environment_id` label —
# `{"environment_id": ..., "container": ..., "status": "running"|"dead"}` —
# so the collector learns about exited containers whose state dir is gone
# without the harness knowing the runtime.
set -euo pipefail

log() { printf 'container-runtime-docker inspect: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the inspect result"
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
  # Every container the create script labelled, whatever its state.
  "$cli" ps -a --filter label=quecto.environment_id \
    --format '{{.Names}}\t{{.State}}\t{{.Label "quecto.environment_id"}}' 2>/dev/null |
    while IFS=$'\t' read -r name state env_id; do
      [ -n "$name" ] || continue
      case "$state" in running) status=running ;; *) status=dead ;; esac
      jq -cn --arg id "$env_id" --arg container "$name" --arg status "$status" \
        '{environment_id: $id, container: $container, status: $status}'
    done
  exit 0
fi
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
[ -n "$id" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"
case "$id" in
*/* | *..*) die "invalid environment id: $id" ;;
esac
env_dir="$state_dir/$id"
if [ ! -d "$env_dir" ]; then
  # The environment's state is gone (a manual `rm -rf`, a completed kill
  # whose record outlived it): that is a truthful "dead", not an error —
  # a restore (#2024 S4d) marks the record stopped instead of keeping it
  # unverified forever.
  jq -cn --arg cli "$cli" \
    '{status: "dead", metadata: {runtime: $cli, cause: "environment-removed"}}'
  exit 0
fi
resolved="$(cd "$env_dir" && pwd -P)"
root="$(cd "$state_dir" && pwd -P)"
case "$resolved" in
"$root"/*) ;;
*) die "environment $id escapes the state root" ;;
esac
container="$(cat "$env_dir/container")"

if ! state="$("$cli" inspect --format '{{.State.Running}} {{.State.ExitCode}} {{.State.OOMKilled}}' "$container" 2>/dev/null)"; then
  jq -cn --arg cli "$cli" --arg container "$container" \
    '{status: "dead", metadata: {runtime: $cli, container: $container, cause: "container-removed"}}'
  exit 0
fi
read -r running exit_code oom <<<"$state"
if [ "$running" = "true" ]; then
  status="running"
  cause="member-connection-lost"
else
  status="dead"
  if [ "$oom" = "true" ]; then
    cause="oom-killed"
  else
    cause="exit-code-$exit_code"
  fi
fi
jq -cn --arg cli "$cli" --arg status "$status" --arg container "$container" --arg cause "$cause" \
  '{status: $status, metadata: {runtime: $cli, container: $container, cause: $cause}}'
