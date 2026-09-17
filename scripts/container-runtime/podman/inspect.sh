#!/usr/bin/env bash
# Official Podman adapter for the Quecto container-runtime contract: `inspect`.
#   inspect.sh --state-dir <dir>
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Reports the container's truth post-mortem. Bounded by Quecto's 5s
# inspect timeout, so only cheap Podman inspect calls happen here.
set -euo pipefail

log() { printf 'container-runtime-podman inspect: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the inspect result"
# Runtime CLI: rootless Podman is mandatory. The explicit override is accepted
# only as a spelling of Podman, preventing accidental runtime substitution.
[ "$(uname -s)" = "Linux" ] || die "Podman runtime requires Linux"
cli=podman
[ -z "${QUECTO_CONTAINER_CLI:-}" ] || [ "${QUECTO_CONTAINER_CLI}" = podman ] || die "Podman-only adapter rejects QUECTO_CONTAINER_CLI"
[ -n "$cli" ] && command -v "$cli" >/dev/null 2>&1 || die "podman is required"
# Inspection is a lifecycle operation: require the same local rootless engine
# before reading any retained environment metadata.
rootless="$($cli info --format '{{.Host.Security.Rootless}}' 2>/dev/null)" || die "Podman is not usable for this user"
[ "$rootless" = "true" ] || die "standard runtime requires rootless Podman"

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
