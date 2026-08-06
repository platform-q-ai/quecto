#!/usr/bin/env bash
# Official Docker adapter for the Quecto container-runtime contract: `create`.
# Modeled on the host-local reference set at scripts/container-runtime/ (see
# docs/container-runtimes.md for the contract). Verified manually against a
# local Docker daemon; the CI-exercised default remains the host-local set.
#
#   create.sh --state-dir <dir> [--image <img>] -- <child-binary> <child-args...>
# Environment: QUECTO_CONTAINER_REPO, QUECTO_CONTAINER_SCRIPT,
#              QUECTO_CONTAINER_ENVIRONMENT_REF
#
# Design: one container per environment; the child IS the container's main
# process (docker's view of the container == the child's liveness). All
# host paths the child needs are identity-mounted (same path inside and
# outside), so the parent's --socket/--config CLI args need no rewriting
# and the UDS socket the child binds appears directly on the host:
#   - the per-environment workspace (rw)  — the isolated checkout
#   - the parent's socket dir (rw)        — UDS endpoint + launch sidecars
#   - the child binary (ro)
#   - the --config file (ro), when given
#   - ~/.quecto (rw) with HOME preserved  — auth/sessions behave like a
#     host child; isolation targets the PR workspace, not user identity
set -euo pipefail

log() { printf 'container-runtime-docker create: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the create result"
command -v docker >/dev/null 2>&1 || die "docker is required"

state_dir=""
image="${QUECTO_DOCKER_IMAGE:-quecto-box:local}"
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || die "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  --image)
    [ "$#" -ge 2 ] || die "--image needs a value"
    image="$2"
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
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_REF:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_REF must be set"
[ -n "${QUECTO_CONTAINER_REPO:-}" ] || die "QUECTO_CONTAINER_REPO must be set"

child_binary="$1"
[ -x "$child_binary" ] || die "child binary $child_binary is not executable"

# The child's CLI carries the UDS endpoint and (optionally) its config file.
socket_path=""
config_path=""
prev=""
for arg in "$@"; do
  case "$prev" in
  --socket) socket_path="$arg" ;;
  --config) config_path="$arg" ;;
  esac
  prev="$arg"
done
[ -n "$socket_path" ] || die "child command has no --socket argument"
socket_dir="$(dirname "$socket_path")"
[ -d "$socket_dir" ] || die "socket dir $socket_dir does not exist"

mkdir -p -m 700 "$state_dir"
[ -O "$state_dir" ] || die "state dir $state_dir is not owned by the current user"
env_dir="$(mktemp -d "$state_dir/env-XXXXXXXXXX")" || die "failed to create environment dir under $state_dir"
environment_id="$(basename "$env_dir")"
container="quecto-$environment_id"
# Rollback on any later failure: remove partial state AND any container we
# managed to start — an unreported environment can never be cleaned up by
# Quecto.
trap 'docker rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$env_dir"' ERR
workspace_path="$env_dir/workspace"
mkdir "$workspace_path"
printf '%s\n' "$QUECTO_CONTAINER_ENVIRONMENT_REF" >"$env_dir/ref"

log "checking out $QUECTO_CONTAINER_REPO"
git clone --quiet -- "$QUECTO_CONTAINER_REPO" "$workspace_path/repo"

# --- Runtime-specific section (Docker) ----------------------------------
mounts=(
  -v "$workspace_path:$workspace_path:rw"
  -v "$socket_dir:$socket_dir:rw"
  -v "$child_binary:$child_binary:ro"
  -v "$HOME/.quecto:$HOME/.quecto:rw"
)
if [ -n "$config_path" ] && [[ "$config_path" != "$HOME/.quecto/"* ]]; then
  [ -f "$config_path" ] || die "child --config $config_path does not exist"
  mounts+=(-v "$config_path:$config_path:ro")
fi
# HOME is preserved and QUECTO_BASE_DIR is deliberately NOT overridden:
# QUECTO_BASE_DIR is quecto's credentials/config home ($HOME/.quecto by
# default). Overriding it inside the container detaches the child from the
# identity-mounted $HOME/.quecto and breaks OAuth providers — do not set it.
envs=(-e "HOME=$HOME")
for key in ANTHROPIC_API_KEY OPENAI_API_KEY OPENROUTER_API_KEY; do
  if [ -n "${!key:-}" ]; then envs+=(-e "$key=${!key}"); fi
done
docker run -d --name "$container" \
  --label "quecto.environment_id=$environment_id" \
  "${mounts[@]}" "${envs[@]}" \
  -w "$workspace_path/repo" \
  "$image" "$@" >/dev/null
# ------------------------------------------------------------------------

printf '%s\n' "$container" >"$env_dir/container"
jq -cn --arg container "$container" --arg socket "$socket_path" \
  '{container: $container, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$environment_id" >>"$state_dir/creates.log"

jq -cn \
  --arg id "$environment_id" \
  --arg workspace "$workspace_path" \
  --arg socket "$socket_path" \
  --arg script "${QUECTO_CONTAINER_SCRIPT:-}" \
  --arg image "$image" \
  --arg container "$container" \
  '{environment_id: $id, workspace_path: $workspace, metadata: {runtime: "docker", image: $image, container: $container, script: $script}, socket_path: $socket}'
