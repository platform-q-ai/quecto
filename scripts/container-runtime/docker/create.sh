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
# Validate BEFORE any environment state exists: `die` exits without firing
# the ERR trap, so every die-able check must precede the mktemp below or a
# failed create would leak an unreported env dir forever.
if [ -n "$config_path" ]; then
  [ -f "$config_path" ] || die "child --config $config_path does not exist"
fi

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
# SECURITY (PR #1401 review): the repo URL is agent-supplied and this clone
# runs on the HOST, before any container exists. Whitelist git transports so
# command-running helpers (`ext::sh -c ...`) cannot execute host commands —
# a containment bypass for the adapter whose whole point is isolation.
GIT_ALLOW_PROTOCOL="file:https:ssh:git" \
  git clone --quiet -- "$QUECTO_CONTAINER_REPO" "$workspace_path/repo"

# --- Runtime-specific section (Docker) ----------------------------------
mounts=(
  -v "$workspace_path:$workspace_path:rw"
  -v "$socket_dir:$socket_dir:rw"
  -v "$child_binary:$child_binary:ro"
  -v "$HOME/.quecto:$HOME/.quecto:rw"
)
if [ -n "$config_path" ] && [[ "$config_path" != "$HOME/.quecto/"* ]]; then
  mounts+=(-v "$config_path:$config_path:ro")
fi
# HOME is preserved and QUECTO_BASE_DIR is deliberately NOT overridden:
# QUECTO_BASE_DIR is quecto's credentials/config home ($HOME/.quecto by
# default). Overriding it inside the container detaches the child from the
# identity-mounted $HOME/.quecto and breaks OAuth providers — do not set it.
envs=(-e "HOME=$HOME")
# SECURITY (PR #1401 review): provider API keys must NOT be passed with
# `docker run -e KEY=value` — that bakes them into the container config,
# readable for the container's whole lifetime via `docker inspect` and
# persisted in /var/lib/docker/containers/<id>/config.v2.json. Instead they
# are written to a 0600 file in the 0700 state dir, identity-mounted ro, and
# sourced by a bootstrap shell that `exec`s the child — so the child still
# ends up as PID 1 with the keys in its environment, but the keys never
# appear in the docker-side container config. (/proc/1/environ inside the
# container is unavoidable: joiners there already share $HOME/.quecto.)
secret_env_file=""
append_secret() {
  # $1=name $2=value — single-quote-escaped export into the 0600 env file.
  if [ -z "$secret_env_file" ]; then
    secret_env_file="$env_dir/provider-env"
    (umask 077 && : >"$secret_env_file")
  fi
  local value="$2"
  printf "export %s='%s'\n" "$1" "${value//\'/\'\\\'\'}" >>"$secret_env_file"
}
for key in ANTHROPIC_API_KEY OPENAI_API_KEY OPENROUTER_API_KEY; do
  if [ -n "${!key:-}" ]; then append_secret "$key" "${!key}"; fi
done
# GitHub access for agents inside the environment (workflows need `gh` and
# git-over-https pushes). A host keyring is unreachable from a container, so
# the token is resolved host-side (`gh auth token`) and rides in via the same
# 0600 secret file; git identity and the gh credential helper are non-secret
# and travel as GIT_CONFIG_* env entries, so the host gitconfig (which may
# carry LFS filters or keyring helpers the image lacks) is never mounted.
gh_token=""
if command -v gh >/dev/null 2>&1; then
  gh_token="$(gh auth token 2>/dev/null || true)"
fi
if [ -n "$gh_token" ]; then
  append_secret GH_TOKEN "$gh_token"
  append_secret GITHUB_TOKEN "$gh_token"
fi
gcfg_i=0
add_git_cfg() {
  envs+=(-e "GIT_CONFIG_KEY_${gcfg_i}=$1" -e "GIT_CONFIG_VALUE_${gcfg_i}=$2")
  gcfg_i=$((gcfg_i + 1))
}
# Deterministic identity: the global gitconfig only (repo-local identity at
# the parent's cwd is an accident of where the spawn ran).
git_name="$(git config --global --get user.name 2>/dev/null || true)"
git_email="$(git config --global --get user.email 2>/dev/null || true)"
[ -n "$git_name" ] && add_git_cfg user.name "$git_name"
[ -n "$git_email" ] && add_git_cfg user.email "$git_email"
if [ -n "$gh_token" ]; then
  add_git_cfg credential.https://github.com.helper "!gh auth git-credential"
fi
[ "$gcfg_i" -gt 0 ] && envs+=(-e "GIT_CONFIG_COUNT=$gcfg_i")
if [ -n "$secret_env_file" ]; then
  mounts+=(-v "$secret_env_file:$secret_env_file:ro")
fi
if [ -n "$secret_env_file" ]; then
  # `sh -c` sources the 0600 file then exec-replaces itself, leaving the
  # child as the container's PID 1. Requires /bin/sh in the image.
  docker run -d --name "$container" \
    --label "quecto.environment_id=$environment_id" \
    "${mounts[@]}" "${envs[@]}" \
    -w "$workspace_path/repo" \
    "$image" /bin/sh -c '. "$0" && exec "$@"' "$secret_env_file" "$@" >/dev/null
else
  docker run -d --name "$container" \
    --label "quecto.environment_id=$environment_id" \
    "${mounts[@]}" "${envs[@]}" \
    -w "$workspace_path/repo" \
    "$image" "$@" >/dev/null
fi
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
