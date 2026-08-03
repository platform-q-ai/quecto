#!/usr/bin/env bash
set -euo pipefail

# Container entrypoint used by scripts/docker-harness-local-tui.sh.
# It prepares a development checkout, installs the container-side Quecto
# binaries from that checkout, then starts the UDS harness and optional API.

RUN_AS=()
if [[ "$(id -u)" == "0" && -n "${HOST_UID:-}" ]]; then
  HOST_GID="${HOST_GID:-$HOST_UID}"
  if ! group_name="$(getent group "$HOST_GID" | cut -d: -f1)"; then
    addgroup -g "$HOST_GID" -S appuser >/dev/null
    group_name="appuser"
  fi
  if ! getent passwd "$HOST_UID" >/dev/null 2>&1; then
    adduser -S -D -H -h /home/appuser -u "$HOST_UID" -G "$group_name" appuser >/dev/null
  fi
  RUN_AS=(su-exec "$HOST_UID:$HOST_GID")
fi

CONTAINER_SOCKET="${CONTAINER_SOCKET:-/socket/quecto.sock}"
QUECTO_BASE_DIR="${QUECTO_BASE_DIR:-/home/appuser/.quecto}"
QUECTO_DEV_REPO_DIR="${QUECTO_DEV_REPO_DIR:-/workspace/quecto}"
QUECTO_REPO_URL="${QUECTO_REPO_URL:-git@github.com:platform-q-ai/quecto.git}"
QUECTO_INSTALL_ROOT="${QUECTO_INSTALL_ROOT:-/workspace/.cargo-install}"

rm -f "$CONTAINER_SOCKET"
mkdir -p "$(dirname "$CONTAINER_SOCKET")" "$HOME" "$QUECTO_BASE_DIR" /workspace
if [[ -n "${QUECTO_CONFIG_SEED_PATH:-}" && -f "$QUECTO_CONFIG_SEED_PATH" ]]; then
  echo "Seeding Quecto config from $QUECTO_CONFIG_SEED_PATH" >&2
  cp "$QUECTO_CONFIG_SEED_PATH" "$QUECTO_BASE_DIR/config.json"
  chmod 600 "$QUECTO_BASE_DIR/config.json" || true
fi
if [[ -n "${QUECTO_CREDENTIALS_SEED_PATH:-}" && -f "$QUECTO_CREDENTIALS_SEED_PATH" ]]; then
  echo "Seeding Quecto credentials from $QUECTO_CREDENTIALS_SEED_PATH" >&2
  cp "$QUECTO_CREDENTIALS_SEED_PATH" "$QUECTO_BASE_DIR/credentials.json"
  chmod 600 "$QUECTO_BASE_DIR/credentials.json" || true
fi
if [[ -n "${QUECTO_GH_CONFIG_SEED_DIR:-}" && -d "$QUECTO_GH_CONFIG_SEED_DIR" ]]; then
  echo "Seeding GitHub CLI auth from $QUECTO_GH_CONFIG_SEED_DIR" >&2
  export GH_CONFIG_DIR="$HOME/.config/gh"
  mkdir -p "$GH_CONFIG_DIR"
  cp -R "$QUECTO_GH_CONFIG_SEED_DIR/." "$GH_CONFIG_DIR/"
  chmod -R go-rwx "$GH_CONFIG_DIR" || true
else
  # Prefer GH_TOKEN/GITHUB_TOKEN auth and isolate gh from any stale stored
  # account state. This prevents `gh auth status` from failing because of an
  # invalid copied/default account while a valid GH_TOKEN is present.
  export GH_CONFIG_DIR="${GH_CONFIG_DIR:-$HOME/.config/gh-token-only}"
  mkdir -p "$GH_CONFIG_DIR"
fi
chown -R "${HOST_UID:-0}:${HOST_GID:-0}" "$HOME" "$QUECTO_BASE_DIR" /workspace "$(dirname "$CONTAINER_SOCKET")" 2>/dev/null || true

if [[ -z "${GIT_SSH_COMMAND:-}" ]]; then
  export GIT_SSH_COMMAND="ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
fi

if [[ "${QUECTO_DEV_CLONE:-1}" == "1" ]]; then
  if [[ -d "$QUECTO_DEV_REPO_DIR/.git" ]]; then
    echo "Updating Quecto repo in $QUECTO_DEV_REPO_DIR" >&2
    "${RUN_AS[@]}" git -C "$QUECTO_DEV_REPO_DIR" fetch --all --prune || true
  else
    echo "Cloning Quecto repo from $QUECTO_REPO_URL to $QUECTO_DEV_REPO_DIR" >&2
    rm -rf "$QUECTO_DEV_REPO_DIR"
    "${RUN_AS[@]}" git clone "$QUECTO_REPO_URL" "$QUECTO_DEV_REPO_DIR"
  fi

  if [[ -n "${QUECTO_REPO_REF:-}" ]]; then
    "${RUN_AS[@]}" git -C "$QUECTO_DEV_REPO_DIR" checkout "$QUECTO_REPO_REF"
    if "${RUN_AS[@]}" git -C "$QUECTO_DEV_REPO_DIR" show-ref --verify --quiet "refs/remotes/origin/$QUECTO_REPO_REF"; then
      echo "Pulling latest origin/$QUECTO_REPO_REF" >&2
      "${RUN_AS[@]}" git -C "$QUECTO_DEV_REPO_DIR" pull --ff-only origin "$QUECTO_REPO_REF"
    fi
  fi
fi

cd "$QUECTO_DEV_REPO_DIR"

if [[ "${QUECTO_INSTALL:-1}" == "1" ]]; then
  QUECTO_INSTALL_PACKAGES="${QUECTO_INSTALL_PACKAGES:-quecto-agentic-harness}"
  echo "Installing Quecto packages into $QUECTO_INSTALL_ROOT: $QUECTO_INSTALL_PACKAGES" >&2
  "${RUN_AS[@]}" mkdir -p "$QUECTO_INSTALL_ROOT"
  for package in $QUECTO_INSTALL_PACKAGES; do
    echo "Installing $package from $QUECTO_DEV_REPO_DIR" >&2
    "${RUN_AS[@]}" env CARGO_INSTALL_ROOT="$QUECTO_INSTALL_ROOT" \
      cargo install --path "$package" --locked --force
  done
fi

export PATH="$QUECTO_INSTALL_ROOT/bin:$PATH"

"${RUN_AS[@]}" env \
  PATH="$PATH" \
  QUECTO_AGENTS_DEFAULTS_WORKSPACE="$QUECTO_DEV_REPO_DIR" \
  quecto agent --mode uds --no-sandbox --socket "$CONTAINER_SOCKET" --persist ${QUECTO_AGENT_ARGS:-} &
agent_pid=$!

for _ in $(seq 1 "${QUECTO_SOCKET_TIMEOUT:-900}"); do
  [[ -S "$CONTAINER_SOCKET" ]] && break
  kill -0 "$agent_pid" >/dev/null 2>&1 || {
    echo "quecto agent exited before socket was ready" >&2
    wait "$agent_pid"
  }
  sleep 0.1
done

[[ -S "$CONTAINER_SOCKET" ]] || { echo "timed out waiting for $CONTAINER_SOCKET" >&2; exit 1; }
echo "quecto-agent-socket: $CONTAINER_SOCKET" >&2

if [[ "${API_ENABLED:-0}" == "1" ]]; then
  if ! command -v quecto-api >/dev/null 2>&1; then
    echo "API_ENABLED=1 but quecto-api is not installed in $QUECTO_INSTALL_ROOT/bin." >&2
    echo "Add quecto-api to QUECTO_INSTALL_PACKAGES." >&2
    exit 1
  fi
  "${RUN_AS[@]}" env PATH="$PATH" quecto-api --socket "$CONTAINER_SOCKET" --host 0.0.0.0 --port "${API_PORT:-8080}" &
  echo "quecto-api: http://127.0.0.1:${API_PORT:-8080}" >&2
fi

if [[ "${TRANSPORT:-direct}" == "tcp-proxy" ]]; then
  socat TCP-LISTEN:"${PROXY_PORT:-17777}",reuseaddr,fork UNIX-CONNECT:"$CONTAINER_SOCKET" &
  echo "quecto-uds-tcp-bridge: 127.0.0.1:${PROXY_PORT:-17777}" >&2
fi

wait "$agent_pid"
