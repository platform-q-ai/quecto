#!/usr/bin/env bash
# Official Docker adapter for the Quecto container-runtime contract: `exec` (join).
#   exec.sh --state-dir <dir> -- <child-binary> <child-args...>
# Environment: QUECTO_CONTAINER_CONFIG, QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Starts a joining child inside the environment's existing container via
# `exec`, in the environment's checkout. The joiner's socket lives
# in the same parent socket dir that create identity-mounted, so it is
# reachable from the host without extra mounts.
set -euo pipefail

log() { printf 'container-runtime-docker exec: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the exec result"
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
[ -n "$container" ] || die "environment $id has no recorded container"
running="$("$cli" inspect --format '{{.State.Running}}' "$container" 2>/dev/null || echo false)"
[ "$running" = "true" ] || die "container $container for environment $id is not running"

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

workspace_path="$env_dir/workspace"
workdir="$workspace_path/repo"
[ -d "$workdir" ] || workdir="$workspace_path"

envs=(-e "HOME=$HOME")
# Joiners get the same environment contract as the creator: git identity +
# gh credential helper as non-secret GIT_CONFIG_* entries, and the 0600
# provider-env file (API keys + GH token) sourced by a bootstrap so secrets
# never enter the docker-side exec config.
gcfg_i=0
add_git_cfg() {
  envs+=(-e "GIT_CONFIG_KEY_${gcfg_i}=$1" -e "GIT_CONFIG_VALUE_${gcfg_i}=$2")
  gcfg_i=$((gcfg_i + 1))
}
git_name="$(git config --global --get user.name 2>/dev/null || true)"
git_email="$(git config --global --get user.email 2>/dev/null || true)"
[ -n "$git_name" ] && add_git_cfg user.name "$git_name"
[ -n "$git_email" ] && add_git_cfg user.email "$git_email"
secret_env_file="$env_dir/provider-env"
if [ -f "$secret_env_file" ] && grep -q '^export GH_TOKEN=' "$secret_env_file"; then
  add_git_cfg credential.https://github.com.helper "!gh auth git-credential"
fi
[ "$gcfg_i" -gt 0 ] && envs+=(-e "GIT_CONFIG_COUNT=$gcfg_i")
# stdout is the strict JSON contract; `podman exec -d` prints the exec
# session id (docker prints nothing), so both branches discard stdout.
if [ -f "$secret_env_file" ]; then
  "$cli" exec -d -w "$workdir" "${envs[@]}" \
    "$container" /bin/sh -c '. "$0" && exec "$@"' "$secret_env_file" "$@" >/dev/null
else
  "$cli" exec -d -w "$workdir" "${envs[@]}" \
    "$container" "$@" >/dev/null
fi

jq -cn --arg container "$container" --arg socket "$socket_path" \
  '{container: $container, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$id" >>"$state_dir/execs.log"

jq -cn --arg cli "$cli" --arg socket "$socket_path" --arg container "$container" \
  '{metadata: {runtime: $cli, container: $container}, socket_path: $socket}'
