#!/usr/bin/env bash
# Canonical Quecto container-runtime reference script: `create` (#1369).
#
# Contract (see docs/container-runtimes.md):
#   create-argv... [--repo <url>] -- <child-binary> <child-args...>
#   create-argv... [--repo <url>] --preflight-only
# Environment: QUECTO_CONTAINER_CONFIG, QUECTO_CONTAINER_ENVIRONMENT_REF
#
# Preflight (#2024 S4b): jq, git and --repo reachability (when given), and
# the state dir, checked before any environment state exists. A normal
# create dies at the first failure with a distinct message and exit code;
# `--preflight-only` evaluates every check, prints one
# `status<TAB>check<TAB>detail<TAB>remedy` line per check on stdout and
# exits non-zero when any failed (`quecto container doctor` drives it).
#
# The repository is BAKED INTO the container config's own argv via --repo
# (#1410): Quecto passes no source information, and the parent's location is
# irrelevant. A config without --repo is a sandbox: empty workspace, no
# clone, still fully valid.
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
# A --repo URL may carry credentials (https://user:token@host/…); every
# message that names it shows the URL with its userinfo replaced by ***.
# The token stays in the config file and the clone; never in a preflight
# line, a log line, or the create result.
redact_url() { printf '%s' "$1" | sed -E 's#(://)[^/@]+@#\1***@#g'; }
EXIT_USAGE=2
EXIT_NO_JQ=4
EXIT_NO_GIT=5
EXIT_REPO_UNREACHABLE=7
EXIT_STATE_DIR=8
die() {
  log "$@"
  exit 1
}
usage() {
  log "$@"
  exit "$EXIT_USAGE"
}

state_dir=""
repo=""
preflight_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
  --state-dir)
    [ "$#" -ge 2 ] || usage "--state-dir needs a value"
    state_dir="$2"
    shift 2
    ;;
  --repo)
    [ "$#" -ge 2 ] || usage "--repo needs a value"
    repo="$2"
    shift 2
    ;;
  --preflight-only)
    preflight_only=1
    shift
    ;;
  --)
    shift
    break
    ;;
  *) usage "unknown argument: $1" ;;
  esac
done
[ -n "$state_dir" ] || usage "--state-dir is required"
repo_shown="$(redact_url "$repo")"

# --- Preflight ------------------------------------------------------------
preflight_failed=0
report() {
  # $1=ok|warn|fail  $2=check  $3=detail  $4=remedy  $5=exit code on fail
  if [ "$preflight_only" = 1 ]; then
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "${3//[$'\t\n\r']/ }" "${4//[$'\t\n\r']/ }"
    [ "$1" = fail ] && preflight_failed=1
    return 0
  fi
  case "$1" in
  fail)
    log "$3; $4"
    exit "$5"
    ;;
  warn) log "$3; $4" ;;
  esac
}
if jq_path="$(command -v jq 2>/dev/null)"; then
  report ok jq "jq at $jq_path" ""
else
  report fail jq "jq is not on PATH (needed to encode the create result)" "install jq" "$EXIT_NO_JQ"
fi
git_path=""
if [ -n "$repo" ]; then
  if git_path="$(command -v git 2>/dev/null)"; then
    report ok git "git at $git_path" ""
  else
    report fail git "git is not on PATH (needed to clone --repo $repo_shown)" "install git" "$EXIT_NO_GIT"
  fi
else
  report ok git "not needed: sandbox config (no --repo)" ""
fi
probe_timeout="${QUECTO_REPO_CHECK_TIMEOUT:-15}"
case "$probe_timeout" in
''|*[!0-9]*|0*) usage "QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)" ;;
esac
if [ -n "$repo" ] && [ -n "$git_path" ]; then
  ls_remote=(git ls-remote --exit-code -- "$repo" HEAD)
  if command -v timeout >/dev/null 2>&1; then
    ls_remote=(timeout "$probe_timeout" "${ls_remote[@]}")
  fi
  repo_rc=0
  repo_error="$(GIT_ALLOW_PROTOCOL="file:https:ssh:git" GIT_TERMINAL_PROMPT=0 \
    "${ls_remote[@]}" 2>&1 >/dev/null)" || repo_rc=$?
  if [ "$repo_rc" = 0 ]; then
    report ok repo "--repo $repo_shown is reachable" ""
  elif [ "$repo_rc" = 2 ] && [ -z "$repo_error" ]; then
    report warn repo "--repo $repo_shown is reachable but empty (no HEAD)" \
      "push an initial commit if members are expected to find one"
  elif [ "$repo_rc" = 124 ]; then
    report fail repo "--repo $repo_shown did not answer within ${probe_timeout}s" \
      "check the host, your network and credentials; QUECTO_REPO_CHECK_TIMEOUT raises the bound" "$EXIT_REPO_UNREACHABLE"
  else
    repo_cause=""
    while IFS= read -r line; do
      case "$line" in fatal:*) repo_cause="$line"; break ;; esac
    done <<<"$repo_error"
    repo_error="${repo_cause:-$(printf '%s' "$repo_error" | tr -d '\r' | tail -n 1)}"
    report fail repo "--repo $repo_shown is unreachable: ${repo_error:-git ls-remote exited $repo_rc}" \
      "check the URL, your network and credentials, or fix --repo in the container config" "$EXIT_REPO_UNREACHABLE"
  fi
elif [ -n "$repo" ]; then
  report warn repo "--repo $repo_shown was not checked: git is missing" "fix the git check first"
else
  report ok repo "not needed: sandbox config (no --repo)" ""
fi
# State-root hardening: a real create makes the root owner-only and refuses
# to adopt one owned by someone else (an attacker-planted directory under a
# world-writable parent such as /var/tmp); the preflight only judges it.
if [ "$preflight_only" = 0 ]; then
  mkdir -p -m 700 "$state_dir" 2>/dev/null || true
fi
if [ -d "$state_dir" ]; then
  if [ -O "$state_dir" ] && [ -w "$state_dir" ]; then
    report ok state-dir "state dir $state_dir is writable and owned by the current user" ""
  elif [ ! -O "$state_dir" ]; then
    report fail state-dir "state dir $state_dir is not owned by the current user" \
      "choose a --state-dir under a directory you own" "$EXIT_STATE_DIR"
  else
    report fail state-dir "state dir $state_dir is not writable" \
      "choose a --state-dir under a directory you own" "$EXIT_STATE_DIR"
  fi
else
  # Walk up to the first existing component: it must be a writable
  # directory (a file in the way is as fatal as an unwritable parent).
  state_parent="$state_dir"
  while [ ! -e "$state_parent" ] && [ "$state_parent" != "/" ] && [ "$state_parent" != "." ]; do
    state_parent="$(dirname "$state_parent")"
  done
  if [ -d "$state_parent" ] && [ -w "$state_parent" ]; then
    report ok state-dir "state dir $state_dir will be created under writable $state_parent" ""
  elif [ -e "$state_parent" ] && [ ! -d "$state_parent" ]; then
    report fail state-dir "state dir $state_dir cannot be created: $state_parent is not a directory" \
      "choose a --state-dir under a directory you own" "$EXIT_STATE_DIR"
  else
    report fail state-dir "state dir $state_dir cannot be created: $state_parent is not writable" \
      "choose a --state-dir under a directory you own" "$EXIT_STATE_DIR"
  fi
fi
if [ "$preflight_only" = 1 ]; then
  if [ "$preflight_failed" = 0 ]; then exit 0; else exit 1; fi
fi
# --------------------------------------------------------------------------

[ "$#" -gt 0 ] || usage "missing child command after --"
# Identity split: create receives the session REF; exec/kill/inspect/cleanup
# instead receive QUECTO_CONTAINER_ENVIRONMENT_ID (the id minted below).
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_REF:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_REF must be set"

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

# Environment directories are minted with mktemp: unpredictable suffix, mode
# 700, and a hard failure instead of silently reusing (or following a symlink
# planted at) a pre-existing path.
env_dir="$(mktemp -d "$state_dir/env-XXXXXXXXXX")" || die "failed to create environment dir under $state_dir"
# Rollback on any later failure: an environment that was never reported to
# Quecto can never be reached by the `cleanup` operation, so a partial
# creation (e.g. a failed clone) must remove its own state instead of
# leaking one directory per failure.
trap 'rm -rf "$env_dir"' ERR
environment_id="$(basename "$env_dir")"
workspace_path="$env_dir/workspace"
mkdir "$workspace_path"
printf '%s\n' "$QUECTO_CONTAINER_ENVIRONMENT_REF" >"$env_dir/ref"

# Safe repository handling: the URL is one literal argv element from this
# config's own --repo, never shell-interpolated, and the clone target is
# confined to the workspace. No --repo → sandbox config: the workspace stays
# empty and the child starts in it directly.
child_cwd="$workspace_path"
source="none"
if [ -n "$repo" ]; then
  log "checking out $repo_shown"
  git clone --quiet -- "$repo" "$workspace_path/repo"
  child_cwd="$workspace_path/repo"
  source="repo"
fi

# --- Runtime-specific section -------------------------------------------
# A real adapter creates the isolated environment here (e.g. `docker run`
# with the workspace mounted) and starts the child inside it EXACTLY once,
# with the socket path shared back to the host. The host-local reference
# starts the child directly, INSIDE the environment's checkout (or the empty
# sandbox workspace) so the agent genuinely operates in its isolated
# workspace. Quecto never starts a fallback child itself.
(cd "$child_cwd" && exec "$@") >/dev/null 2>&1 &
child_pid=$!
# ------------------------------------------------------------------------

jq -cn --argjson pid "$child_pid" --arg socket "$socket_path" \
  '{pid: $pid, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$environment_id" >>"$state_dir/creates.log"

# Exactly one JSON object on stdout — encoded with a real JSON encoder.
# metadata.repository is how listings/TUI learn the source truthfully: the
# config owns its repository, so only the script can report it (#1410).
jq -cn \
  --arg id "$environment_id" \
  --arg workspace "$workspace_path" \
  --arg socket "$socket_path" \
  --arg config "${QUECTO_CONTAINER_CONFIG:-}" \
  --arg source "$source" \
  --arg repository "$repo_shown" \
  --arg checkout "$child_cwd" \
  '{environment_id: $id, workspace_path: $workspace, metadata: ({runtime: "host-local", config: $config, source: $source, checkout: $checkout} +(if $repository == "" then {} else {repository: $repository} end)), socket_path: $socket}'
