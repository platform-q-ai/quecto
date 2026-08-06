#!/usr/bin/env bash
# Official Docker adapter for the Quecto container-runtime contract: `inspect`.
#   inspect.sh --state-dir <dir>
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Reports the container's truth post-mortem. Bounded by Quecto's 5s
# inspect timeout, so only cheap docker inspect calls happen here.
set -euo pipefail

log() { printf 'container-runtime-docker inspect: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the inspect result"
command -v docker >/dev/null 2>&1 || die "docker is required"

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
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
[ -n "$id" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"
case "$id" in
*/* | *..*) die "invalid environment id: $id" ;;
esac
env_dir="$state_dir/$id"
[ -d "$env_dir" ] || die "unknown environment: $id"
resolved="$(cd "$env_dir" && pwd -P)"
root="$(cd "$state_dir" && pwd -P)"
case "$resolved" in
"$root"/*) ;;
*) die "environment $id escapes the state root" ;;
esac
container="$(cat "$env_dir/container")"

if ! state="$(docker inspect --format '{{.State.Running}} {{.State.ExitCode}} {{.State.OOMKilled}}' "$container" 2>/dev/null)"; then
  jq -cn --arg container "$container" \
    '{status: "dead", metadata: {runtime: "docker", container: $container, cause: "container-removed"}}'
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
jq -cn --arg status "$status" --arg container "$container" --arg cause "$cause" \
  '{status: $status, metadata: {runtime: "docker", container: $container, cause: $cause}}'
