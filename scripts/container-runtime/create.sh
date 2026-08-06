#!/usr/bin/env bash
# Canonical Quecto container-runtime reference script: `create` (#1369).
#
# Contract (see docs/container-runtimes.md):
#   create-argv... -- <child-binary> <child-args...>
# Environment: QUECTO_CONTAINER_REPO, QUECTO_CONTAINER_SCRIPT,
#              QUECTO_CONTAINER_ENVIRONMENT_REF, QUECTO_BASE_DIR
#
# This reference runtime is host-local: it checks the repository out into a
# per-environment workspace under --state-dir and starts the child directly on
# the host, so it works everywhere (including CI without Docker). To adapt it
# to a real container runtime, replace ONLY the marked runtime-specific
# section — the argv/JSON contract and the state layout stay identical.
# Runtime knowledge (Docker/Podman/devcontainer flags) belongs in these
# scripts, never in Quecto's Rust code.
set -euo pipefail

log() { printf 'container-runtime create: %s\n' "$*" >&2; }
die() {
  log "$@"
  exit 1
}

command -v jq >/dev/null 2>&1 || die "jq is required to encode the create result"

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
# Identity split: create receives the session REF; exec/kill/inspect/cleanup
# instead receive QUECTO_CONTAINER_ENVIRONMENT_ID (the id minted below).
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_REF:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_REF must be set"
[ -n "${QUECTO_CONTAINER_REPO:-}" ] || die "QUECTO_CONTAINER_REPO must be set"

# The child's CLI carries the UDS endpoint the parent will connect to.
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

# State-root hardening: create the root owner-only, and refuse to adopt a
# pre-existing root owned by someone else (e.g. an attacker-planted directory
# under a world-writable parent such as /var/tmp).
mkdir -p -m 700 "$state_dir"
[ -O "$state_dir" ] || die "state dir $state_dir is not owned by the current user"
# Environment directories are minted with mktemp: unpredictable suffix, mode
# 700, and a hard failure instead of silently reusing (or following a symlink
# planted at) a pre-existing path.
env_dir="$(mktemp -d "$state_dir/env-XXXXXXXXXX")" || die "failed to create environment dir under $state_dir"
environment_id="$(basename "$env_dir")"
workspace_path="$env_dir/workspace"
mkdir "$workspace_path"
printf '%s\n' "$QUECTO_CONTAINER_ENVIRONMENT_REF" >"$env_dir/ref"

# Safe repository handling: the URL is one literal argv element after `--`,
# never shell-interpolated, and the clone target is confined to the workspace.
log "checking out $QUECTO_CONTAINER_REPO"
git clone --quiet -- "$QUECTO_CONTAINER_REPO" "$workspace_path/repo"

# --- Runtime-specific section -------------------------------------------
# A real adapter creates the isolated environment here (e.g. `docker run`
# with the workspace mounted) and starts the child inside it EXACTLY once,
# with the socket path shared back to the host. The host-local reference
# starts the child directly. Quecto never starts a fallback child itself.
"$@" >/dev/null 2>&1 &
child_pid=$!
# ------------------------------------------------------------------------

jq -cn --argjson pid "$child_pid" --arg socket "$socket_path" \
  '{pid: $pid, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$environment_id" >>"$state_dir/creates.log"

# Exactly one JSON object on stdout — encoded with a real JSON encoder.
jq -cn \
  --arg id "$environment_id" \
  --arg workspace "$workspace_path" \
  --arg socket "$socket_path" \
  --arg script "${QUECTO_CONTAINER_SCRIPT:-}" \
  '{environment_id: $id, workspace_path: $workspace, metadata: {runtime: "host-local", script: $script}, socket_path: $socket}'
