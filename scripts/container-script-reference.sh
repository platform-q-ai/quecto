#!/usr/bin/env bash
# Reference implementation of the Quecto container_scripts `create` contract
# (see docs/container-scripts.md). It runs the child directly on the host —
# substitute the marked section with your container runtime (Docker, Podman,
# devcontainer CLI, ...) to provide real isolation.
#
# Invocation:  create-argv... -- <child-binary> <child-args...>
# Environment: QUECTO_CONTAINER_REPO, QUECTO_CONTAINER_SCRIPT,
#              QUECTO_CONTAINER_ENVIRONMENT_REF, QUECTO_BASE_DIR
# (The paired cleanup/kill scripts instead receive QUECTO_CONTAINER_ENVIRONMENT_ID,
# the runtime environment_id this script reports below; an exec script receives
# QUECTO_CONTAINER_ENVIRONMENT_ID and QUECTO_CONTAINER_SCRIPT and prints
# {"metadata":{},"socket_path":"..."} — see docs/container-scripts.md.)
set -euo pipefail

# Split our own argv from the child command after `--`.
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then
    shift
    break
  fi
  shift
done
if [ "$#" -eq 0 ]; then
  echo "missing child command after --" >&2
  exit 1
fi

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
if [ -z "$socket_path" ]; then
  echo "child command has no --socket argument" >&2
  exit 1
fi

environment_id="env-$$"
workspace_path="$(mktemp -d "${TMPDIR:-/tmp}/quecto-env.XXXXXX")"

# Check out the requested repository into the workspace.
if [ -n "${QUECTO_CONTAINER_REPO:-}" ]; then
  git clone --quiet "$QUECTO_CONTAINER_REPO" "$workspace_path/repo"
fi

# --- Runtime-specific section -------------------------------------------
# A real script creates the isolated environment here (e.g. `docker run`)
# and starts the child inside it exactly once, with the socket path shared
# back to the host. This reference starts the child directly.
"$@" >/dev/null 2>&1 &
# ------------------------------------------------------------------------

# Report the create result: exactly one JSON object on stdout.
printf '{"environment_id":"%s","workspace_path":"%s","metadata":{"script":"%s","ref":"%s"},"socket_path":"%s"}\n' \
  "$environment_id" \
  "$workspace_path" \
  "${QUECTO_CONTAINER_SCRIPT:-}" \
  "${QUECTO_CONTAINER_ENVIRONMENT_REF:-}" \
  "$socket_path"
