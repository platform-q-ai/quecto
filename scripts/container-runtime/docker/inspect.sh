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
  # Every container the create script labelled with THIS state root,
  # whatever its state (a container of another root or base dir is not
  # this collector's). Containers created before the root label existed
  # are reached through their state directory instead. A runtime that
  # cannot answer is a failure with its own words, never an empty list.
  root_label="$(cd "$state_dir" 2>/dev/null && pwd -P || printf '%s' "$state_dir")"
  listing="$("$cli" ps -a --filter label=quecto.environment_id --filter "label=quecto.state_dir=$root_label" \
    --format '{{.Names}}\t{{.State}}\t{{.Label "quecto.environment_id"}}')" || die "$cli ps failed"
  printf '%s\n' "$listing" | while IFS=$'\t' read -r name state env_id; do
    [ -n "$name" ] || continue
    # Only a container the runtime calls finished is dead; created,
    # paused or restarting ones are still somebody's.
    case "$state" in exited | dead | stopped) status=dead ;; *) status=running ;; esac
    jq -cn --arg id "$env_id" --arg container "$name" --arg status "$status" \
      '{environment_id: $id, container: $container, status: $status}'
  done
  exit 0
fi
# Whether the runtime's own words say container $2 does not exist: a
# line of its error output $1 saying "no such container"/"no such object"
# (Podman and Docker alike) that names the container.
says_no_such() {
  printf '%s\n' "$1" | grep -iE 'no such (container|object)' | grep -qF -- "$2"
}

# Ask the runtime about one container. The answer is left in `answer`;
# returns 1 only when the runtime's own words say the container does not
# exist. Any other failure — a runtime that cannot be reached, a storage
# lock — is no answer: the inspect fails, so the harness treats the
# container's state as unknown and never as gone (#2206). No temporary
# file: the error words are read by asking again, stderr only.
answer=""
inspect_container() {
  local errors
  if answer="$("$cli" inspect --format "$1" "$2" 2>/dev/null)"; then
    return 0
  fi
  if errors="$("$cli" inspect --format "$1" "$2" 2>&1 >/dev/null)"; then
    die "$cli inspect $2 failed, then answered: whether it exists is unknown"
  fi
  if says_no_such "$errors" "$2"; then
    return 1
  fi
  die "$cli inspect $2 failed, so whether it exists is unknown: $(printf '%s' "$errors" | tr '\n' ' ')"
}

id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
[ -n "$id" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"
case "$id" in
*/* | *..*) die "invalid environment id: $id" ;;
esac
env_dir="$state_dir/$id"
if [ ! -d "$env_dir" ]; then
  # The environment's state is gone (a manual `rm -rf`, a completed kill
  # whose record outlived it). The container the create would have named
  # may still run: ask the runtime before calling it dead, so a restore
  # (#2024 S4d) never stops a live environment — and marks a truly gone
  # one stopped instead of keeping it unverified forever.
  # Gone or exited is dead; running is running; any other state is
  # unknown (#2206).
  state="dead"
  if inspect_container '{{.State.Status}}' "quecto-$id"; then
    state="$answer"
  fi
  case "$state" in
  running)
    jq -cn --arg cli "$cli" --arg container "quecto-$id" \
      '{status: "running", metadata: {runtime: $cli, container: $container, cause: "state-dir-removed"}}'
    ;;
  exited | dead | stopped)
    jq -cn --arg cli "$cli" \
      '{status: "dead", metadata: {runtime: $cli, cause: "environment-removed"}}'
    ;;
  *) die "container quecto-$id is $state: neither running nor exited, so its state is unknown" ;;
  esac
  exit 0
fi
resolved="$(cd "$env_dir" && pwd -P)"
root="$(cd "$state_dir" && pwd -P)"
case "$resolved" in
"$root"/*) ;;
*) die "environment $id escapes the state root" ;;
esac
container="$(cat "$env_dir/container")"

if ! inspect_container '{{.State.Status}} {{.State.ExitCode}} {{.State.OOMKilled}}' "$container"; then
  jq -cn --arg cli "$cli" --arg container "$container" \
    '{status: "dead", metadata: {runtime: $cli, container: $container, cause: "container-removed"}}'
  exit 0
fi
read -r state exit_code oom <<<"$answer"
# Only a container the runtime calls running is running, and only one it
# calls exited, dead or stopped (Podman) is dead, as `--list` judges it. Any other state — paused, created,
# stopping, restarting, removing — is neither: the inspect fails, so the
# harness treats it as unknown and never tears it down as gone (#2206).
case "$state" in
running)
  status="running"
  cause="member-connection-lost"
  ;;
exited | dead | stopped)
  status="dead"
  if [ "$oom" = "true" ]; then
    cause="oom-killed"
  else
    cause="exit-code-$exit_code"
  fi
  ;;
*) die "container $container is $state: neither running nor exited, so its state is unknown" ;;
esac
jq -cn --arg cli "$cli" --arg status "$status" --arg container "$container" --arg cause "$cause" \
  '{status: $status, metadata: {runtime: $cli, container: $container, cause: $cause}}'
