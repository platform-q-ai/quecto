#!/usr/bin/env bash
# Legacy-path Podman adapter for the Quecto container-runtime contract: `kill`/`cleanup`.
#   kill.sh --state-dir <dir> --op kill      # retained `kill` argv
#   kill.sh --state-dir <dir> --op cleanup   # retained `cleanup` argv
# Environment: QUECTO_CONTAINER_ENVIRONMENT_ID
#
# Removes the environment's container (force) and its state directory,
# after proving the directory resolves under the trusted state root.
set -euo pipefail

log() { printf 'container-runtime-podman kill: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

# Runtime CLI: rootless Podman is mandatory. The explicit override is accepted
# only as a spelling of Podman, preventing accidental runtime substitution.
[ "$(uname -s)" = "Linux" ] || die "Podman runtime requires Linux"
cli=podman
[ -z "${QUECTO_CONTAINER_CLI:-}" ] || [ "${QUECTO_CONTAINER_CLI}" = podman ] || die "Podman-only adapter rejects QUECTO_CONTAINER_CLI"
[ -n "$cli" ] && command -v "$cli" >/dev/null 2>&1 || die "podman is required"
# Every lifecycle operation performs the same affirmative local-rootless
# preflight. Destructive operations must never fall back to Docker.
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
case "$op" in kill|cleanup) ;; *) die "unknown --op: $op" ;; esac
id="${QUECTO_CONTAINER_ENVIRONMENT_ID:-}"
[ -n "$id" ] || die "QUECTO_CONTAINER_ENVIRONMENT_ID must be set"
# Affirmative identifier allowlist prevents traversal and option injection.
case "$id" in
  *[!A-Za-z0-9_.-]*|.*|*-|*.) die "invalid environment id: $id" ;;
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
