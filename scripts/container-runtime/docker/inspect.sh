#!/usr/bin/env bash
# Legacy-path Podman adapter for the Quecto container-runtime contract: `inspect`.
#   inspect.sh --state-dir <dir>
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Reports the container's truth post-mortem. Bounded by Quecto's 5s
# inspect timeout, so only cheap Podman inspect calls happen here.
set -euo pipefail
export LC_ALL=C

log() { printf 'container-runtime-podman inspect: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the inspect result"
require_local_rootless_podman() {
  # SECURITY: the standard adapter is deliberately local-only.  Refuse every
  # environment selector that can redirect Podman to a service, socket, or
  # named connection; an empty value is the only local configuration.
  for selector in CONTAINER_HOST CONTAINER_CONNECTION PODMAN_HOST PODMAN_CONNECTION DOCKER_HOST; do
    [ -z "${!selector:-}" ] || die "remote Podman selector $selector is not permitted"
  done
  [ "$(uname -s)" = "Linux" ] || die "Podman runtime requires Linux"
  cli=podman
  command -v "$cli" >/dev/null 2>&1 || die "podman is required"
  rootless="$($cli info --format '{{.Host.Security.Rootless}}' 2>/dev/null)" || die "Podman is not usable for this user"
  [ "$rootless" = "true" ] || die "standard runtime requires rootless Podman"
}
[ -z "${QUECTO_CONTAINER_CLI:-}" ] || [ "${QUECTO_CONTAINER_CLI}" = podman ] || die "Podman-only adapter rejects QUECTO_CONTAINER_CLI"
require_local_rootless_podman
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
# Affirmative environment-id allowlist: only runtime-minted path-safe IDs are
# accepted before any retained state or runtime inspection is touched.
case "$id" in
  *[!A-Za-z0-9_.-]*|.*|*-|*.) die "invalid environment id: $id" ;;
esac
env_dir="$state_dir/$id"
[ -d "$env_dir" ] && [ ! -L "$env_dir" ] || die "environment must be a real directory: $id"
[ -O "$env_dir" ] || die "environment is not owned by the current user: $id"
[ "$(stat -c '%a' -- "$env_dir" 2>/dev/null)" = 700 ] || die "environment must have mode 700: $id"
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
