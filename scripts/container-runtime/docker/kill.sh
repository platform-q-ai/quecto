#!/usr/bin/env bash
# Legacy-path Podman adapter for the Quecto container-runtime contract: `kill`/`cleanup`.
#   kill.sh --state-dir <dir> --op kill      # retained `kill` argv
#   kill.sh --state-dir <dir> --op cleanup   # retained `cleanup` argv
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Removes the environment's container (force) and its state directory,
# after proving the directory resolves under the trusted state root.
set -euo pipefail
export LC_ALL=C

log() { printf 'container-runtime-podman kill: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}
# SECURITY: lifecycle calls are local rootless Podman only; reject every
# remote service/socket/connection selector before touching retained state.
for selector in CONTAINER_HOST CONTAINER_CONNECTION PODMAN_HOST PODMAN_CONNECTION DOCKER_HOST; do
  [ -z "${!selector:-}" ] || die "remote Podman selector $selector is not permitted"
done
[ "$(uname -s)" = "Linux" ] || die "Podman runtime requires Linux"
cli=podman
command -v "$cli" >/dev/null 2>&1 || die "podman is required"
rootless="$($cli info --format '{{.Host.Security.Rootless}}' 2>/dev/null)" || die "Podman is not usable for this user"
[ "$rootless" = "true" ] || die "standard runtime requires rootless Podman"

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
case "$state_dir" in /*) ;; *) die "--state-dir must be an absolute path" ;; esac
[ -d "$state_dir" ] && [ ! -L "$state_dir" ] || die "state dir must be a real directory"
[ -O "$state_dir" ] || die "state dir is not owned by the current user"
[ "$(stat -c '%a' -- "$state_dir" 2>/dev/null)" = 700 ] || die "state dir must have mode 700"
case "$op" in kill|cleanup) ;; *) die "unknown --op: $op" ;; esac
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
# SECURITY: affirmative ASCII allowlist; IDs are names, never paths/options.
# The historical denylist *[!A-Za-z0-9_.-]* is deliberately not used.
if [[ "$id" =~ ^[A-Za-z0-9]([A-Za-z0-9_.-]*[A-Za-z0-9])?$ ]]; then
  : # affirmative strict ASCII identifier allowlist
else
  die "invalid environment id: $id"
fi
env_dir="$state_dir/$id"
if [ ! -d "$env_dir" ] || [ -L "$env_dir" ]; then
  # Already gone (e.g. cleanup after a kill): succeed idempotently.
  printf '%s %s\n' "$op" "$id" >>"$state_dir/kill.log" 2>/dev/null || true
  exit 0
fi
[ -O "$env_dir" ] || die "environment is not owned by the current user: $id"
[ "$(stat -c '%a' -- "$env_dir" 2>/dev/null)" = 700 ] || die "environment must have mode 700: $id"
resolved="$(cd "$env_dir" && pwd -P)"
root="$(cd "$state_dir" && pwd -P)"
case "$resolved" in
"$root"/*) ;;
*) die "environment $id escapes the state root" ;;
esac

container="$(cat "$env_dir/container" 2>/dev/null || true)"
if [ -n "$container" ]; then
  # Podman sends SIGTERM and waits the
  # container's stop timeout (10s) before SIGKILL. The parent runs this from
  # its own SIGTERM handling inside the TUI's two-second exit budget, so
  # bound the grace: one second for the child to cascade, then SIGKILL.
  grace=()
  [ "$cli" = podman ] && grace=(--time 1)
  "$cli" rm -f "${grace[@]}" "$container" >/dev/null 2>&1 || true
fi
rm -rf "$env_dir"
printf '%s %s\n' "$op" "$id" >>"$state_dir/kill.log"
