#!/usr/bin/env bash
# Reproducible Podman runtime smoke test. Does not install/build or fall back to Docker.
set -euo pipefail
export LC_ALL=C

repo_root=$(git rev-parse --show-toplevel)
: "${CONTAINER_RUNTIME_CONFIG:?Set CONTAINER_RUNTIME_CONFIG to an absolute config JSON path}"
case "$CONTAINER_RUNTIME_CONFIG" in
  /*) ;;
  *) echo 'CONTAINER_RUNTIME_CONFIG must be absolute' >&2; exit 2 ;;
esac
command -v podman >/dev/null || { echo 'Podman is required; install it on the host.' >&2; exit 127; }
if ! podman info --format '{{.Host.Security.Rootless}}' >/dev/null 2>&1; then
  echo 'Podman is unavailable or not configured for this user.' >&2
  exit 125
fi
printf 'podman=%s\nconfig=%s\nrepo=%s\n' "$(podman version --format '{{.Client.Version}}')" "$CONTAINER_RUNTIME_CONFIG" "$repo_root"
# The harness performs creation and teardown; this script only proves prerequisites.
# Run the application smoke separately with the documented config and --mode new.
