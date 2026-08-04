#!/usr/bin/env bash
set -euo pipefail

repo="${QUECTO_REPO_URL:-}"
socket_path="${QUECTO_SOCKET_PATH:-}"
read_only="false"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) repo="$2"; shift 2 ;;
    --socket-path) socket_path="$2"; shift 2 ;;
    --read-only) read_only="$2"; shift 2 ;;
    --script-name) shift 2 ;;
    --) shift; break ;;
    *) shift ;;
  esac
done
: "${socket_path:=/tmp/quecto-${QUECTO_AGENT_UUID:-agent}.sock}"
env_id="${QUECTO_ENVIRONMENT_UUID:-quecto-${QUECTO_AGENT_UUID:-$(date +%s)}}"
container_ref="${QUECTO_CONTAINER_REF:-$env_id}"
root="${QUECTO_CONTAINER_ROOT:-${TMPDIR:-/tmp}/quecto-containers}"
workspace="${QUECTO_WORKSPACE_PATH:-$root/$env_id/workspace}"
if [[ -n "$repo" ]]; then
  case "$repo" in
    -*) echo "unsafe repository: leading dash is not allowed" >&2; exit 64 ;;
    *$'\n'*|*$'\r'*) echo "unsafe repository: control characters are not allowed" >&2; exit 64 ;;
  esac
fi
mkdir -p "$workspace" "$(dirname "$socket_path")"
if [[ -n "$repo" ]]; then
  if [[ -d "$workspace/.git" ]]; then
    git -C "$workspace" fetch --all --prune >&2 || true
    git -C "$workspace" pull --ff-only >&2 || true
  elif [[ ! -e "$workspace/.git" ]]; then
    rm -rf -- "$workspace"
    git clone -- "$repo" "$workspace" >&2
  fi
fi
pid_file="$root/$env_id/child.pid"
mkdir -p "$(dirname "$pid_file")"
if [[ $# -gt 0 ]]; then
  (cd "$workspace" && "$@") &
  echo "$!" > "$pid_file"
fi
uid=$(id -u); gid=$(id -g)
printf '{"environment_id":"%s","socket_path":"%s","workspace_path":"%s","container_ref":"%s","metadata":{"repo":"%s","runtime":"docker-reference","uid":%s,"gid":%s,"read_only":%s,"child_started":%s,"pid_file":"%s"}}\n' "$env_id" "$socket_path" "$workspace" "$container_ref" "$repo" "$uid" "$gid" "$read_only" "$([[ $# -gt 0 ]] && echo true || echo false)" "$pid_file"
