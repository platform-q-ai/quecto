#!/usr/bin/env bash
set -euo pipefail

# Run the Quecto harness in Docker and attach the local quecto-tui to it.
#
# The harness development checkout and Quecto container state live in Docker
# named volumes. The only host write is a temporary socket directory, removed on
# exit. Host SSH keys may be mounted read-only for private repo access.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

IMAGE="${QUECTO_DOCKER_IMAGE:-quecto-harness:local}"
TRANSPORT="${QUECTO_DOCKER_TRANSPORT:-auto}"
API_ENABLED="${QUECTO_DOCKER_API:-0}"
API_PORT="${QUECTO_DOCKER_API_PORT:-8080}"
PROXY_PORT="${QUECTO_DOCKER_PROXY_PORT:-17777}"
# Ports pinned by the caller are honoured exactly; unpinned ports auto-advance
# past anything already bound so parallel instances do not collide.
API_PORT_PINNED=0
PROXY_PORT_PINNED=0
[[ -n "${QUECTO_DOCKER_API_PORT:-}" ]] && API_PORT_PINNED=1
[[ -n "${QUECTO_DOCKER_PROXY_PORT:-}" ]] && PROXY_PORT_PINNED=1
SOCKET_TIMEOUT="${QUECTO_SOCKET_TIMEOUT:-900}"
QUECTO_REPO_URL="${QUECTO_REPO_URL:-git@github.com:platform-q-ai/quecto.git}"
QUECTO_REPO_REF="${QUECTO_REPO_REF:-master}"
QUECTO_DEV_CLONE="${QUECTO_DEV_CLONE:-1}"
QUECTO_INSTALL="${QUECTO_INSTALL:-1}"
QUECTO_INSTALL_PACKAGES_EXPLICIT="${QUECTO_INSTALL_PACKAGES:-}"
QUECTO_INSTALL_PACKAGES="${QUECTO_INSTALL_PACKAGES:-quecto-agentic-harness}"
QUECTO_INSTALL_ROOT="${QUECTO_INSTALL_ROOT:-/workspace/.cargo-install}"
QUECTO_WORKSPACE_VOLUME="${QUECTO_WORKSPACE_VOLUME:-}"
QUECTO_HOME_VOLUME="${QUECTO_HOME_VOLUME:-}"
QUECTO_INSTANCE="${QUECTO_INSTANCE:-}"
QUECTO_TUI_BIN="${QUECTO_TUI_BIN:-quecto-tui}"
BUILD_IMAGE=1

usage() {
  cat <<EOF
Run the Quecto harness in Docker and attach the local quecto-tui to it.

The container clones/pulls Quecto master into a Docker named volume and starts
quecto agent in UDS mode. This script starts an already-installed host TUI and
passes it the temporary socket path.

Examples:
  scripts/docker-harness-local-tui.sh
  scripts/docker-harness-local-tui.sh --api
  QUECTO_SSH_DIR="\$HOME/.ssh" scripts/docker-harness-local-tui.sh
  QUECTO_DOCKER_TRANSPORT=tcp-proxy scripts/docker-harness-local-tui.sh

Parallel harnesses (each gets its own workspace/state volumes):
  QUECTO_REPO_REF=feature/a scripts/docker-harness-local-tui.sh
  QUECTO_REPO_REF=feature/b scripts/docker-harness-local-tui.sh
  scripts/docker-harness-local-tui.sh --instance scratch

Environment:
  QUECTO_DOCKER_IMAGE       Docker image tag (default: quecto-harness:local)
  QUECTO_DOCKER_TRANSPORT   auto | direct | tcp-proxy (default: auto)
  QUECTO_DOCKER_API         1 to start quecto-api in the container
  QUECTO_DOCKER_API_PORT    host/container API port (pinned; default 8080 auto-advances if busy)
  QUECTO_DOCKER_PROXY_PORT  host/container TCP bridge port (pinned; default 17777 auto-advances if busy)
  QUECTO_SOCKET_TIMEOUT     seconds to wait for harness socket (default: 900)
  QUECTO_REPO_URL           repo cloned in container (default: git@github.com:platform-q-ai/quecto.git)
  QUECTO_REPO_REF           branch/tag/SHA to check out and pull (default: master)
  QUECTO_DEV_CLONE          1 to clone/update repo at entrypoint (default: 1)
  QUECTO_INSTALL            1 to install Quecto binaries in the container entrypoint (default: 1)
  QUECTO_INSTALL_PACKAGES   workspace packages to cargo-install in the container
  QUECTO_INSTALL_ROOT       container cargo-install root (default: /workspace/.cargo-install)
  QUECTO_INSTANCE           instance name isolating volumes (default: slug of QUECTO_REPO_REF)
  QUECTO_WORKSPACE_VOLUME   Docker volume for /workspace (default: quecto-workspace-\$QUECTO_INSTANCE)
  QUECTO_HOME_VOLUME        Docker volume for /home/appuser/.quecto (default: quecto-home-\$QUECTO_INSTANCE)
  QUECTO_ALLOW_SHARED_WORKSPACE  1 to skip the "volume already in use" guard
  QUECTO_TUI_BIN            host TUI command to run (default: quecto-tui)
  QUECTO_SSH_DIR            optional host SSH directory mounted read-only
  QUECTO_CONFIG_FILE        host config.json to seed into container Quecto home
  QUECTO_CREDENTIALS_FILE   host credentials.json to seed into container Quecto home
  QUECTO_GH_CONFIG_DIR      optional host GitHub CLI config dir to seed; normally GH_TOKEN is used instead
  QUECTO_AGENT_ARGS         extra args for \`quecto agent --mode uds ...\`

Options:
  --api                 Start quecto-api in the container on localhost:${API_PORT}
  --transport MODE      auto, direct, or tcp-proxy
  --image TAG           Docker image tag to build/use
  --instance NAME       Isolate this harness under its own Docker volumes
  --api-port PORT       Pin the API port (fails if busy instead of auto-advancing)
  --proxy-port PORT     Pin the TCP bridge port (fails if busy instead of auto-advancing)
  --no-build            Do not build the Docker image first
  -h, --help            Show this help
  --                    Remaining arguments are passed to local quecto-tui
EOF
}

TUI_ARGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --api)
      API_ENABLED=1
      shift
      ;;
    --transport)
      TRANSPORT="${2:-}"
      shift 2
      ;;
    --image)
      IMAGE="${2:-}"
      shift 2
      ;;
    --instance)
      QUECTO_INSTANCE="${2:-}"
      shift 2
      ;;
    --api-port)
      API_PORT="${2:-}"
      API_PORT_PINNED=1
      shift 2
      ;;
    --proxy-port)
      PROXY_PORT="${2:-}"
      PROXY_PORT_PINNED=1
      shift 2
      ;;
    --no-build)
      BUILD_IMAGE=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      TUI_ARGS+=("$@")
      break
      ;;
    *)
      TUI_ARGS+=("$1")
      shift
      ;;
  esac
done

# Each instance gets its own /workspace and Quecto state volumes so that
# concurrent harnesses never share one git working tree or cargo install root.
# The instance name defaults to the checked-out ref, so parallel runs on
# different branches are isolated while repeat runs on one branch keep their
# warm build cache.
if [[ -z "$QUECTO_INSTANCE" ]]; then
  QUECTO_INSTANCE="$(printf '%s' "$QUECTO_REPO_REF" \
    | tr '[:upper:]' '[:lower:]' \
    | sed -e 's#[^a-z0-9_.-]#-#g' -e 's#^[-._]*##' -e 's#[-._]*$##')"
fi
if [[ -z "$QUECTO_INSTANCE" ]]; then
  echo "Could not derive an instance name from QUECTO_REPO_REF='$QUECTO_REPO_REF'." >&2
  echo "Pass --instance NAME or set QUECTO_INSTANCE." >&2
  exit 2
fi
if [[ ! "$QUECTO_INSTANCE" =~ ^[a-zA-Z0-9][a-zA-Z0-9_.-]*$ ]]; then
  echo "Invalid instance name '$QUECTO_INSTANCE' (expected [a-zA-Z0-9][a-zA-Z0-9_.-]*)" >&2
  exit 2
fi
QUECTO_WORKSPACE_VOLUME="${QUECTO_WORKSPACE_VOLUME:-quecto-workspace-$QUECTO_INSTANCE}"
QUECTO_HOME_VOLUME="${QUECTO_HOME_VOLUME:-quecto-home-$QUECTO_INSTANCE}"

case "$TRANSPORT" in
  auto|direct|tcp-proxy) ;;
  *) echo "Invalid --transport '$TRANSPORT' (expected auto, direct, or tcp-proxy)" >&2; exit 2 ;;
esac

if [[ "$TRANSPORT" == "auto" ]]; then
  if [[ "$(uname -s)" == "Linux" ]]; then
    TRANSPORT="direct"
  else
    TRANSPORT="tcp-proxy"
  fi
fi

port_is_free() {
  local port="$1"
  if command -v ss >/dev/null 2>&1; then
    ! ss -ltnH "sport = :$port" 2>/dev/null | grep -q .
  else
    ! (echo >"/dev/tcp/127.0.0.1/$port") >/dev/null 2>&1
  fi
}

# Resolve a host port, auto-advancing only when the caller did not pin it.
# $1 label, $2 port variable name, $3 pinned flag, $4.. ports to avoid
resolve_port() {
  local label="$1" var="$2" pinned="$3"
  shift 3
  local avoid=("$@")
  local port="${!var}"

  if [[ ! "$port" =~ ^[0-9]+$ ]] || ((port < 1 || port > 65535)); then
    echo "Invalid $label port '$port' (expected 1-65535)" >&2
    exit 2
  fi

  local candidate="$port" taken
  for _ in $(seq 0 200); do
    taken=0
    for used in ${avoid[@]+"${avoid[@]}"}; do
      [[ "$candidate" == "$used" ]] && taken=1
    done
    if ((taken == 0)) && port_is_free "$candidate"; then
      if [[ "$candidate" != "$port" ]]; then
        echo "$label port $port is in use; using $candidate instead." >&2
      fi
      printf -v "$var" '%s' "$candidate"
      return 0
    fi
    if [[ "$pinned" == "1" ]]; then
      echo "$label port $port is already in use on 127.0.0.1." >&2
      echo "Free it, or pick another with --${label,,}-port / QUECTO_DOCKER_${label^^}_PORT." >&2
      exit 1
    fi
    ((candidate++))
    ((candidate > 65535)) && break
  done
  echo "Could not find a free $label port starting at $port." >&2
  exit 1
}

if [[ "$API_ENABLED" == "1" ]]; then
  resolve_port api API_PORT "$API_PORT_PINNED"
  # quecto-api lives in its own workspace package; without it the entrypoint
  # publishes the port but has no binary to serve on it.
  if [[ -z "${QUECTO_INSTALL_PACKAGES_EXPLICIT:-}" && " $QUECTO_INSTALL_PACKAGES " != *" quecto-api "* ]]; then
    QUECTO_INSTALL_PACKAGES="$QUECTO_INSTALL_PACKAGES quecto-api"
  fi
fi
if [[ "$TRANSPORT" == "tcp-proxy" ]]; then
  resolve_port proxy PROXY_PORT "$PROXY_PORT_PINNED" "$API_PORT"
fi

command -v docker >/dev/null 2>&1 || { echo "docker is required" >&2; exit 1; }
if ! docker compose version >/dev/null 2>&1; then
  echo "docker compose is required" >&2
  exit 1
fi
if [[ "$TRANSPORT" == "tcp-proxy" ]]; then
  command -v socat >/dev/null 2>&1 || { echo "socat is required locally for tcp-proxy transport" >&2; exit 1; }
fi
if ! command -v "$QUECTO_TUI_BIN" >/dev/null 2>&1; then
  echo "Host TUI command '$QUECTO_TUI_BIN' was not found on PATH." >&2
  echo "Install quecto-tui locally first, or set QUECTO_TUI_BIN=/path/to/quecto-tui." >&2
  exit 1
fi

# Two live harnesses sharing one workspace volume would check out and pull the
# same git tree from under each other, so refuse unless explicitly allowed.
if [[ "${QUECTO_ALLOW_SHARED_WORKSPACE:-0}" != "1" ]]; then
  workspace_users="$(docker ps --quiet --filter "volume=$QUECTO_WORKSPACE_VOLUME" 2>/dev/null || true)"
  if [[ -n "$workspace_users" ]]; then
    echo "Workspace volume '$QUECTO_WORKSPACE_VOLUME' is already in use by a running container:" >&2
    docker ps --filter "volume=$QUECTO_WORKSPACE_VOLUME" --format '  {{.ID}}  {{.Names}}' >&2
    echo "Start this harness under its own instance instead, e.g.:" >&2
    echo "  $0 --instance my-second-branch" >&2
    echo "Set QUECTO_ALLOW_SHARED_WORKSPACE=1 to override." >&2
    exit 1
  fi
fi

COMPOSE_PROJECT_NAME="quecto-harness-$RANDOM-$$"
COMPOSE_SERVICE="quecto-harness"
RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/quecto-harness.XXXXXX")"
SOCKET_DIR="$RUNTIME_DIR/socket"
EMPTY_SSH_DIR="$RUNTIME_DIR/empty-ssh"
LOCAL_SOCKET="$SOCKET_DIR/quecto.sock"
SOCAT_PID=""
COMPOSE_OVERRIDE="$RUNTIME_DIR/docker-compose.override.yml"
mkdir -p "$SOCKET_DIR" "$EMPTY_SSH_DIR"

if [[ -z "${QUECTO_SSH_DIR:-}" ]]; then
  if [[ -d "${HOME:-}/.ssh" ]]; then
    QUECTO_SSH_DIR="$HOME/.ssh"
  else
    QUECTO_SSH_DIR="$EMPTY_SSH_DIR"
  fi
fi

QUECTO_SSH_AUTH_SOCK=""
if [[ -n "${SSH_AUTH_SOCK:-}" && -S "${SSH_AUTH_SOCK:-}" ]]; then
  QUECTO_SSH_AUTH_SOCK="$SSH_AUTH_SOCK"
fi

# Prefer token auth for GitHub CLI/API access inside the container. Copying
# ~/.config/gh is not always enough because host gh auth may depend on an OS
# keychain or credential helper that is unavailable in Docker.
if [[ -z "${GH_TOKEN:-}" && -z "${GITHUB_TOKEN:-}" ]] && command -v gh >/dev/null 2>&1; then
  if token="$(gh auth token -h github.com 2>/dev/null)" && [[ -n "$token" ]]; then
    export GH_TOKEN="$token"
    export GITHUB_TOKEN="$token"
  fi
elif [[ -n "${GH_TOKEN:-}" && -z "${GITHUB_TOKEN:-}" ]]; then
  export GITHUB_TOKEN="$GH_TOKEN"
elif [[ -z "${GH_TOKEN:-}" && -n "${GITHUB_TOKEN:-}" ]]; then
  export GH_TOKEN="$GITHUB_TOKEN"
fi

if [[ -z "${QUECTO_CONFIG_FILE:-}" && -f "${HOME:-}/.quecto/config.json" ]]; then
  QUECTO_CONFIG_FILE="$HOME/.quecto/config.json"
fi
QUECTO_CONFIG_SEED_PATH=""
if [[ -n "${QUECTO_CONFIG_FILE:-}" ]]; then
  if [[ ! -f "$QUECTO_CONFIG_FILE" ]]; then
    echo "QUECTO_CONFIG_FILE does not exist or is not a file: $QUECTO_CONFIG_FILE" >&2
    exit 1
  fi
  QUECTO_CONFIG_FILE="$(cd "$(dirname "$QUECTO_CONFIG_FILE")" && pwd)/$(basename "$QUECTO_CONFIG_FILE")"
  QUECTO_CONFIG_SEED_PATH="/seed/config.json"
fi

if [[ -z "${QUECTO_CREDENTIALS_FILE:-}" && -f "${HOME:-}/.quecto/credentials.json" ]]; then
  QUECTO_CREDENTIALS_FILE="$HOME/.quecto/credentials.json"
fi
QUECTO_CREDENTIALS_SEED_PATH=""
if [[ -n "${QUECTO_CREDENTIALS_FILE:-}" ]]; then
  if [[ ! -f "$QUECTO_CREDENTIALS_FILE" ]]; then
    echo "QUECTO_CREDENTIALS_FILE does not exist or is not a file: $QUECTO_CREDENTIALS_FILE" >&2
    exit 1
  fi
  QUECTO_CREDENTIALS_FILE="$(cd "$(dirname "$QUECTO_CREDENTIALS_FILE")" && pwd)/$(basename "$QUECTO_CREDENTIALS_FILE")"
  QUECTO_CREDENTIALS_SEED_PATH="/seed/credentials.json"
fi

QUECTO_GH_CONFIG_SEED_DIR=""
if [[ -n "${QUECTO_GH_CONFIG_DIR:-}" ]]; then
  if [[ ! -d "$QUECTO_GH_CONFIG_DIR" ]]; then
    echo "QUECTO_GH_CONFIG_DIR does not exist or is not a directory: $QUECTO_GH_CONFIG_DIR" >&2
    exit 1
  fi
  QUECTO_GH_CONFIG_DIR="$(cd "$QUECTO_GH_CONFIG_DIR" && pwd)"
  QUECTO_GH_CONFIG_SEED_DIR="/seed/gh"
fi

if [[ "$QUECTO_REPO_URL" == git@* && "$QUECTO_SSH_DIR" == "$EMPTY_SSH_DIR" && -z "$QUECTO_SSH_AUTH_SOCK" ]]; then
  echo "Private SSH repo URL requires SSH credentials." >&2
  echo "Set QUECTO_SSH_DIR to a host SSH directory, or run with SSH_AUTH_SOCK available." >&2
  echo "Example: QUECTO_SSH_DIR=\"\$HOME/.ssh\" $0" >&2
  exit 1
fi

cat >"$COMPOSE_OVERRIDE" <<EOF
services:
  quecto-harness:
EOF
if [[ -n "$QUECTO_SSH_AUTH_SOCK" ]]; then
  cat >>"$COMPOSE_OVERRIDE" <<EOF
    environment:
      SSH_AUTH_SOCK: /ssh-agent
    volumes:
      - type: bind
        source: ${QUECTO_SSH_AUTH_SOCK}
        target: /ssh-agent
EOF
fi
if [[ -n "$QUECTO_CONFIG_SEED_PATH" || -n "$QUECTO_CREDENTIALS_SEED_PATH" || -n "$QUECTO_GH_CONFIG_SEED_DIR" ]]; then
  echo "    volumes:" >>"$COMPOSE_OVERRIDE"
  if [[ -n "$QUECTO_CONFIG_SEED_PATH" ]]; then
    cat >>"$COMPOSE_OVERRIDE" <<EOF
      - type: bind
        source: ${QUECTO_CONFIG_FILE}
        target: /seed/config.json
        read_only: true
EOF
  fi
  if [[ -n "$QUECTO_CREDENTIALS_SEED_PATH" ]]; then
    cat >>"$COMPOSE_OVERRIDE" <<EOF
      - type: bind
        source: ${QUECTO_CREDENTIALS_FILE}
        target: /seed/credentials.json
        read_only: true
EOF
  fi
  if [[ -n "$QUECTO_GH_CONFIG_SEED_DIR" ]]; then
    cat >>"$COMPOSE_OVERRIDE" <<EOF
      - type: bind
        source: ${QUECTO_GH_CONFIG_DIR}
        target: /seed/gh
        read_only: true
EOF
  fi
fi
if [[ "$API_ENABLED" == "1" || "$TRANSPORT" == "tcp-proxy" ]]; then
  {
    echo "    ports:"
    if [[ "$API_ENABLED" == "1" ]]; then
      echo "      - \"127.0.0.1:${API_PORT}:${API_PORT}\""
    fi
    if [[ "$TRANSPORT" == "tcp-proxy" ]]; then
      echo "      - \"127.0.0.1:${PROXY_PORT}:${PROXY_PORT}\""
    fi
  } >>"$COMPOSE_OVERRIDE"
else
  echo "    # No host ports are published for direct UDS mode." >>"$COMPOSE_OVERRIDE"
fi

COMPOSE=(docker compose -f "$REPO_ROOT/docker-compose.harness.yml" -f "$COMPOSE_OVERRIDE" --project-directory "$REPO_ROOT" --project-name "$COMPOSE_PROJECT_NAME")

cleanup() {
  set +e
  if [[ -n "$SOCAT_PID" ]]; then kill "$SOCAT_PID" >/dev/null 2>&1 || true; fi
  "${COMPOSE[@]}" down --remove-orphans >/dev/null 2>&1 || true
  rm -rf "$RUNTIME_DIR"
}
trap cleanup EXIT INT TERM

export QUECTO_DOCKER_IMAGE="$IMAGE"
export QUECTO_DOCKER_TRANSPORT="$TRANSPORT"
export QUECTO_DOCKER_API="$API_ENABLED"
export QUECTO_DOCKER_API_PORT="$API_PORT"
export QUECTO_DOCKER_PROXY_PORT="$PROXY_PORT"
export QUECTO_SOCKET_TIMEOUT="$SOCKET_TIMEOUT"
export QUECTO_REPO_URL
export QUECTO_REPO_REF
export QUECTO_DEV_CLONE
export QUECTO_INSTALL
export QUECTO_INSTALL_PACKAGES
export QUECTO_INSTALL_ROOT
export QUECTO_WORKSPACE_VOLUME
export QUECTO_HOME_VOLUME
export QUECTO_SOCKET_HOST="$SOCKET_DIR"
export QUECTO_SSH_DIR
export QUECTO_CONFIG_SEED_PATH
export QUECTO_CREDENTIALS_SEED_PATH
export QUECTO_GH_CONFIG_SEED_DIR
export QUECTO_AGENT_ARGS="${QUECTO_AGENT_ARGS:-}"
export HOST_UID="$(id -u)"
export HOST_GID="$(id -g)"

if [[ "$BUILD_IMAGE" == "1" ]]; then
  "${COMPOSE[@]}" build "$COMPOSE_SERVICE"
fi

"${COMPOSE[@]}" up --no-build "$COMPOSE_SERVICE" &
COMPOSE_PID=$!

if [[ "$TRANSPORT" == "direct" ]]; then
  for _ in $(seq 1 "$SOCKET_TIMEOUT"); do
    [[ -S "$LOCAL_SOCKET" ]] && break
    if ! kill -0 "$COMPOSE_PID" >/dev/null 2>&1; then
      echo "harness compose service exited before socket was ready" >&2
      wait "$COMPOSE_PID" || true
      exit 1
    fi
    sleep 1
  done
  [[ -S "$LOCAL_SOCKET" ]] || {
    echo "timed out after ${SOCKET_TIMEOUT}s waiting for mounted socket $LOCAL_SOCKET" >&2
    echo "The first run can take several minutes because the container entrypoint cargo-installs Quecto packages." >&2
    exit 1
  }
else
  for _ in $(seq 1 "$SOCKET_TIMEOUT"); do
    if (echo >/dev/tcp/127.0.0.1/"$PROXY_PORT") >/dev/null 2>&1; then break; fi
    if ! kill -0 "$COMPOSE_PID" >/dev/null 2>&1; then
      echo "harness compose service exited before TCP bridge was ready" >&2
      wait "$COMPOSE_PID" || true
      exit 1
    fi
    sleep 1
  done
  LOCAL_PROXY_SOCKET="$RUNTIME_DIR/quecto-proxy.sock"
  socat UNIX-LISTEN:"$LOCAL_PROXY_SOCKET",fork,unlink-early TCP:127.0.0.1:"$PROXY_PORT" &
  SOCAT_PID=$!
  LOCAL_SOCKET="$LOCAL_PROXY_SOCKET"
fi

echo "Compose project:    $COMPOSE_PROJECT_NAME" >&2
echo "TUI socket:         $LOCAL_SOCKET" >&2
if [[ "$API_ENABLED" == "1" ]]; then
  echo "API gateway:        http://127.0.0.1:$API_PORT" >&2
fi

set +e
"$QUECTO_TUI_BIN" --socket "$LOCAL_SOCKET" "${TUI_ARGS[@]}"
TUI_STATUS=$?
cleanup
exit "$TUI_STATUS"
