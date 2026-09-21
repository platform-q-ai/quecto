#!/usr/bin/env bash
# Official Docker/Podman adapter for the Quecto container-runtime contract: `create`.
# Modeled on the host-local reference set at scripts/container-runtime/ (see
# docs/container-runtimes.md for the contract). Verified manually against a
# local Docker daemon; the CI-exercised default remains the host-local set.
#
#   create.sh --state-dir <dir> [--image <img>] -- <child-binary> <child-args...>
#   create.sh --state-dir <dir> [--image <img>] [--repo <url>] --preflight-only
# The repository is BAKED INTO the container config's own argv via --repo
# (#1410): Quecto passes no source information. No --repo → sandbox config
# (empty workspace, no clone).
# Environment: QUECTO_CONTAINER_CONFIG,
#              QUECTO_CONTAINER_ENVIRONMENT_REF
#
# Preflight (#2024 S4b): one list of checks — container runtime, jq, git,
# gh, image present (never pulled implicitly), --repo reachable, state dir
# writable — runs before any environment state exists. A normal create dies
# at the first failure with a distinct message and exit code (see the
# EXIT_* table); `--preflight-only` evaluates every check, prints one
# `status<TAB>check<TAB>detail<TAB>remedy` line per check on stdout
# (status ok|warn|fail) and exits non-zero when any failed. `quecto
# container doctor` runs this mode for the effective container config.
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
# A --repo URL may carry credentials (https://user:token@host/…); every
# message that names it shows the URL with its userinfo replaced by ***.
# The token stays in the config file and the clone; never in a preflight
# line, a log line, or the create result.
redact_url() { printf '%s' "$1" | sed -E 's#(://)[^/]+@#\1***@#g'; }
# Distinct exit codes so a failure is classifiable from its status alone.
EXIT_USAGE=2
EXIT_NO_RUNTIME=3
EXIT_NO_JQ=4
EXIT_NO_GIT=5
EXIT_NO_IMAGE=6
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
image="${QUECTO_DOCKER_IMAGE:-quecto-dev:local}"
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
  --image)
    [ "$#" -ge 2 ] || usage "--image needs a value"
    image="$2"
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
# Every check reports through `report`; the mode decides what a failure
# does. Nothing here creates environment state, so a failed create never
# leaks an unreported environment directory.
preflight_failed=0
report() {
  # $1=ok|warn|fail  $2=check  $3=detail  $4=remedy  $5=exit code on fail
  if [ "$preflight_only" = 1 ]; then
    # One line per check: a tab or newline inside a detail (a URL, a path)
    # would split the fields the doctor parses.
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

# Runtime CLI: rootless Podman by default. Membership of the `docker` group
# is root-equivalent on the host (the daemon runs as root and has no policy
# layer, so anything holding the socket can mount / and escalate), which is
# exactly what an autonomous agent spawner must not hand out. Rootless
# Podman runs the container as the invoking user with a user namespace, so
# an escape lands as that user, not root. QUECTO_CONTAINER_CLI overrides;
# Docker stays a fallback for hosts without Podman.
cli="${QUECTO_CONTAINER_CLI:-}"
cli_path=""
if [ -n "$cli" ]; then
  cli_path="$(command -v "$cli" 2>/dev/null || true)"
  if [ -n "$cli_path" ]; then
    report ok runtime-cli "QUECTO_CONTAINER_CLI=$cli at $cli_path" ""
  else
    report fail runtime-cli "QUECTO_CONTAINER_CLI=$cli is not on PATH" \
      "install $cli or unset QUECTO_CONTAINER_CLI to use podman (preferred) or docker" "$EXIT_NO_RUNTIME"
    cli=""
  fi
else
  for candidate in podman docker; do
    cli_path="$(command -v "$candidate" 2>/dev/null || true)"
    if [ -n "$cli_path" ]; then
      cli="$candidate"
      break
    fi
  done
  if [ -n "$cli" ]; then
    report ok runtime-cli "$cli at $cli_path" ""
  else
    report fail runtime-cli "no container runtime on PATH: neither podman nor docker was found" \
      "install podman (rootless, preferred) or docker and make sure it is on PATH, or set QUECTO_CONTAINER_CLI" "$EXIT_NO_RUNTIME"
  fi
fi

if jq_path="$(command -v jq 2>/dev/null)"; then
  report ok jq "jq at $jq_path" ""
else
  report fail jq "jq is not on PATH (needed to encode the create result)" \
    "install jq" "$EXIT_NO_JQ"
fi

git_path=""
if [ -n "$repo" ]; then
  if git_path="$(command -v git 2>/dev/null)"; then
    report ok git "git at $git_path" ""
  else
    report fail git "git is not on PATH (needed to clone --repo $repo_shown)" \
      "install git" "$EXIT_NO_GIT"
  fi
else
  report ok git "not needed: sandbox config (no --repo)" ""
fi

# gh is optional: without it members get no GitHub token (pushes and the
# GitHub API fail inside the container), so it is a warning, never a die.
if gh_path="$(command -v gh 2>/dev/null)"; then
  report ok gh "gh at $gh_path" ""
else
  report warn gh "gh is not on PATH: members will have no GitHub token (GH_TOKEN)" \
    "install gh and run 'gh auth login' so members can push and use the GitHub API"
fi

# A bound for the runtime and repository probes: an unreachable daemon or
# host must not stall a create for the tool's own connect timeout.
probe_timeout="${QUECTO_REPO_CHECK_TIMEOUT:-15}"
case "$probe_timeout" in
''|*[!0-9]*|0*) usage "QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)" ;;
esac
bounded() {
  if command -v timeout >/dev/null 2>&1; then
    timeout "$probe_timeout" "$@"
  else
    "$@"
  fi
}
last_line() { printf '%s' "$1" | tr -d '\r' | tail -n 1; }

# The image is looked up in the local store only: a create never pulls
# implicitly (an unattended pull of an unexpected image is not this
# script's decision to make); the `run` below says so too (`--pull=never`),
# so a tag that vanished between preflight and run fails instead of
# fetching whatever a registry serves under that name. A runtime that cannot answer (daemon down,
# socket permission, timeout) is reported as its own failure, not as a
# missing image.
if [ -n "$cli" ]; then
  image_rc=0
  case "$cli" in
  podman) image_error="$(bounded "$cli" image exists "$image" 2>&1 >/dev/null)" || image_rc=$? ;;
  *) image_error="$(bounded "$cli" image inspect "$image" 2>&1 >/dev/null)" || image_rc=$? ;;
  esac
  if [ "$image_rc" = 0 ]; then
    report ok image "image $image is present" ""
  elif [ "$image_rc" = 124 ]; then
    report fail image "$cli did not answer within ${probe_timeout}s while looking up image $image" \
      "check that the $cli daemon/service is running and reachable (${cli} info); QUECTO_REPO_CHECK_TIMEOUT raises the bound" "$EXIT_NO_RUNTIME"
  elif [ "$image_rc" = 1 ] && { [ -z "$image_error" ] || printf '%s' "$image_error" | grep -qi 'no such image\|image not known'; }; then
    report fail image "image $image is not present in the local $cli store" \
      "build it ($cli build -t $image <dir with its Containerfile>) or pull it ($cli pull $image); create never pulls implicitly" "$EXIT_NO_IMAGE"
  else
    report fail image "$cli could not look up image $image (exit $image_rc): $(last_line "$image_error")" \
      "check that the $cli daemon/service is running and that this user may use it (${cli} info)" "$EXIT_NO_RUNTIME"
  fi
else
  report warn image "image $image was not checked: no container runtime" \
    "fix the runtime-cli check first"
fi

# What an image must hold is tooling-neutral (#2073). The harness itself
# needs only a shell (proven by running the probe at all) and git inside the
# container; every other tool is the image's own promise, declared in its
# `ai.quecto.required-tools` label (a whitespace-separated list of
# executable names) and proven here before an agent is admitted, so an image
# that lost a tool fails at the doctor and not after code has been changed.
# No label, no extra check: nothing here knows a language.
required_tools_label="ai.quecto.required-tools"
required_tools_max=64

# Allowlist for one declared tool: a bare ASCII executable name, whatever the
# host's locale makes of [A-Za-z]. A name is only ever passed to the probe as
# an argument, never spliced into its script.
is_tool_name() {
  local LC_ALL=C
  [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$ ]]
}

# Runs the image and proves every argument is on its PATH. Sets probe_rc and
# probe_error (stderr only: a runtime warning is not the probe's answer).
probe_image_tools() {
  probe_rc=0
  probe_error="$(bounded "$cli" run --rm --pull=never "$image" sh -c '
    for tool in "$@"; do
      command -v "$tool" >/dev/null || { printf "missing %s\n" "$tool" >&2; exit 1; }
    done
  ' sh "$@" 2>&1 >/dev/null)" || probe_rc=$?
}

# Reports a failed probe for what it was: the image's fault only when the
# probe itself said so; a runtime that did not answer or could not run the
# image is the runtime's.
report_probe_failure() {
  # $1=check  $2=what the image fails to be  $3=remedy for the image
  local answer
  answer="$(last_line "$probe_error")"
  if [ "$probe_rc" = 1 ] && [[ "$answer" == "missing "* ]]; then
    report fail "$1" "image $image $2: $answer" "$3" "$EXIT_NO_IMAGE"
  elif [ "$probe_rc" = 124 ]; then
    report fail "$1" "$cli did not answer within ${probe_timeout}s while running image $image" \
      "check that the $cli daemon/service is running and reachable (${cli} info); QUECTO_REPO_CHECK_TIMEOUT raises the bound" "$EXIT_NO_RUNTIME"
  elif [ "$probe_rc" = 126 ] || [ "$probe_rc" = 127 ]; then
    report fail "$1" "image $image has no usable shell (exit $probe_rc): $answer" \
      "install a POSIX shell (sh) in the image and rebuild it" "$EXIT_NO_IMAGE"
  else
    report fail "$1" "$cli could not run image $image (exit $probe_rc): $answer" \
      "check that the $cli daemon/service is running and that this user may use it (${cli} info)" "$EXIT_NO_RUNTIME"
  fi
}

base_ok=0
if [ -n "$cli" ] && [ "$image_rc" = 0 ]; then
  probe_image_tools git
  if [ "$probe_rc" = 0 ]; then
    base_ok=1
    report ok image-base "image $image provides a shell and git" ""
  else
    report_probe_failure image-base "cannot host an agent" \
      "install a POSIX shell and git in the image and rebuild it"
  fi
elif [ -n "$cli" ]; then
  report fail image-base "image $image could not be checked for a shell and git" \
    "fix the image check first" "$EXIT_NO_IMAGE"
else
  report warn image-base "the image's shell and git were not checked: no container runtime" \
    "fix the runtime-cli check first"
fi

if [ "$base_ok" = 1 ]; then
  # stdout alone is the label: the runtimes print warnings on stderr while
  # exiting 0, and a warning is not a list of tools. stderr is read only to
  # explain a failure.
  label_rc=0
  label_format="{{ index .Config.Labels \"$required_tools_label\" }}"
  label_value="$(bounded "$cli" image inspect --format "$label_format" "$image" 2>/dev/null)" || label_rc=$?
  label_value="${label_value//$'\r'/}"
  # An absent label prints an empty line (or Go's "<no value>").
  if [ "$label_value" = "<no value>" ]; then label_value=""; fi
  required_tools=()
  bad_tool=""
  if [ "$label_rc" = 0 ]; then
    read -r -d '' -a required_tools <<<"$label_value" || true
    # (An empty array under `set -u` is an error before bash 4.4.)
    if [ "${#required_tools[@]}" -gt 0 ]; then
      for tool in "${required_tools[@]}"; do
        if is_tool_name "$tool"; then continue; fi
        bad_tool="${tool:0:64}"
        break
      done
    fi
  fi
  if [ "$label_rc" = 124 ]; then
    report fail required-tools "$cli did not answer within ${probe_timeout}s while reading the $required_tools_label label of image $image" \
      "check that the $cli daemon/service is running and reachable (${cli} info); QUECTO_REPO_CHECK_TIMEOUT raises the bound" "$EXIT_NO_RUNTIME"
  elif [ "$label_rc" != 0 ]; then
    label_error="$(bounded "$cli" image inspect --format "$label_format" "$image" 2>&1 >/dev/null)" || true
    report fail required-tools "$cli could not read the $required_tools_label label of image $image (exit $label_rc): $(last_line "$label_error")" \
      "check that the $cli daemon/service is running and that this user may use it (${cli} info)" "$EXIT_NO_RUNTIME"
  elif [ -n "$bad_tool" ]; then
    report fail required-tools "image $image declares '$bad_tool' in $required_tools_label, which is not a tool name" \
      "list bare executable names separated by spaces (ASCII letters, digits, '.', '_', '+', '-'; at most 64 characters each) and rebuild the image" "$EXIT_NO_IMAGE"
  elif [ "${#required_tools[@]}" -gt "$required_tools_max" ]; then
    report fail required-tools "image $image declares ${#required_tools[@]} tools in $required_tools_label; at most $required_tools_max are checked" \
      "shorten the label and rebuild the image" "$EXIT_NO_IMAGE"
  elif [ "${#required_tools[@]}" = 0 ]; then
    report ok required-tools "image $image declares no required tools ($required_tools_label is not set)" ""
  else
    probe_image_tools "${required_tools[@]}"
    if [ "$probe_rc" = 0 ]; then
      report ok required-tools "image $image provides the tools it declares: ${required_tools[*]}" ""
    else
      report_probe_failure required-tools "does not provide a tool it declares in $required_tools_label" \
        "rebuild the image from its Containerfile, or correct the label"
    fi
  fi
elif [ -n "$cli" ]; then
  report fail required-tools "the tools image $image declares were not checked" \
    "fix the image-base check first" "$EXIT_NO_IMAGE"
else
  report warn required-tools "the image's declared tools were not checked: no container runtime" \
    "fix the runtime-cli check first"
fi

if [ -n "$repo" ]; then
  if [ -n "$git_path" ]; then
    # Same transport whitelist as the clone below (PR #1401 review), no
    # credential prompt, and the bound above so an unreachable host cannot
    # stall the create for git's own connect timeout.
    repo_rc=0
    repo_error="$(GIT_ALLOW_PROTOCOL="file:https:ssh:git" GIT_TERMINAL_PROMPT=0 \
      bounded git ls-remote --exit-code -- "$repo" HEAD 2>&1 >/dev/null)" || repo_rc=$?
    if [ "$repo_rc" = 0 ]; then
      report ok repo "--repo $repo_shown is reachable" ""
    elif [ "$repo_rc" = 2 ] && [ -z "$repo_error" ]; then
      # `--exit-code` 2: reachable, no HEAD — a freshly initialised
      # repository, which the clone below accepts.
      report warn repo "--repo $repo_shown is reachable but empty (no HEAD)" \
        "push an initial commit if members are expected to find one"
    elif [ "$repo_rc" = 124 ]; then
      report fail repo "--repo $repo_shown did not answer within ${probe_timeout}s" \
        "check the host, your network and credentials (ssh key or 'gh auth login'); QUECTO_REPO_CHECK_TIMEOUT raises the bound" "$EXIT_REPO_UNREACHABLE"
    else
      # git's first `fatal:` line names the cause; the trailing advice
      # ("and the repository exists.") does not.
      repo_cause=""
      while IFS= read -r line; do
        case "$line" in fatal:*) repo_cause="$line"; break ;; esac
      done <<<"$repo_error"
      repo_error="${repo_cause:-$(last_line "$repo_error")}"
      report fail repo "--repo $repo_shown is unreachable: ${repo_error:-git ls-remote exited $repo_rc}" \
        "check the URL, your network and credentials (ssh key or 'gh auth login'), or fix --repo in the container config" "$EXIT_REPO_UNREACHABLE"
    fi
  else
    report warn repo "--repo $repo_shown was not checked: git is missing" "fix the git check first"
  fi
else
  report ok repo "not needed: sandbox config (no --repo)" ""
fi

# The state root is created owner-only by a real create; the preflight
# only judges it (an existing root must be ours and writable, a missing
# one needs a writable parent) so `--preflight-only` leaves no trace.
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
[ -n "${QUECTO_CONTAINER_ENVIRONMENT_REF:-}" ] || die "QUECTO_CONTAINER_ENVIRONMENT_REF must be set"

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
# Every die-able check precedes the environment mktemp below so a failed
# create never leaks an unreported environment directory.
#
# The admission dir keeps its configured spelling for the record exec.sh
# compares and for the child, which opens that spelling inside the container;
# every mount uses its lexical normal form (`admission_mount`), so the path
# that is classified is the path that is mounted. A spelling is admitted only
# when it is absolute, made of characters a `-v src:dst:mode` spec can carry,
# and differs from its normal form by nothing but doubled slashes, `/./` and
# a trailing slash — those open the same directory wherever the normal form
# is mounted; a `..` does not.
admission_dir="${QUECTO_ADMISSION_DIR:-}"
admission_mount=""
admission_root=""
admission_leaf=""
admission_masked=0
if [ -n "$admission_dir" ]; then
  [ -d "$admission_dir" ] || die "QUECTO_ADMISSION_DIR '$admission_dir' is not a directory"
  [[ "$admission_dir" == /* ]] || die "QUECTO_ADMISSION_DIR '$admission_dir' must be an absolute path"
  [[ "$admission_dir" =~ ^[^[:cntrl:]:]+$ ]] \
    || die "QUECTO_ADMISSION_DIR must be made of characters that are printable in this locale, without ':' (it is mounted as src:dst:mode)"
  [[ "$HOME" == /* && "$HOME" =~ ^[^[:cntrl:]:]+$ ]] || die "HOME must be an absolute, printable path without ':' when QUECTO_ADMISSION_DIR is set"
  admission_mount="$(realpath -ms -- "$admission_dir")" && [ -n "$admission_mount" ] \
    || die "cannot normalize QUECTO_ADMISSION_DIR '$admission_dir': this adapter needs GNU realpath (-m, -s)"
  # The spelling with only its doubled slashes, `/./` and trailing slash
  # removed. Equal to the normal form exactly when there is no `..` in it.
  admission_squeezed="$admission_dir"
  while [[ "$admission_squeezed" == *//* ]]; do admission_squeezed="${admission_squeezed//\/\///}"; done
  while [[ "$admission_squeezed" == */./* ]]; do admission_squeezed="${admission_squeezed//\/.\///}"; done
  admission_squeezed="${admission_squeezed%/.}"
  [ "$admission_squeezed" = / ] || admission_squeezed="${admission_squeezed%/}"
  [ "${admission_squeezed:-/}" = "$admission_mount" ] \
    || die "QUECTO_ADMISSION_DIR '$admission_dir' is not its normal form '$admission_mount' ('..' is not supported: the child opens the configured spelling, and only the normal form is mounted); spell it without '..'"
  admission_root="$(dirname -- "$admission_mount")"
  admission_leaf="$(basename -- "$admission_mount")"
  # Exactly one client component beneath a parent: this is what is
  # pre-created inside the mask and mounted read-write.
  [ "${admission_root%/}/$admission_leaf" = "$admission_mount" ] \
    || die "QUECTO_ADMISSION_DIR '$admission_dir' has an unsupported client leaf"
  real_root="$(realpath -m -- "$admission_root")" && [ -n "$real_root" ] \
    || die "cannot resolve QUECTO_ADMISSION_DIR parent '$admission_root'"
  real_leaf="$(realpath -m -- "$admission_mount")" && [ -n "$real_leaf" ] \
    || die "cannot resolve QUECTO_ADMISSION_DIR '$admission_dir'"
  # The client leaf is a real directory of its parent. A symbolic link there
  # would make the read-write mount expose whatever it points at — the
  # authority itself, for `client -> .`.
  [ "$real_leaf" = "${real_root%/}/$admission_leaf" ] \
    || die "QUECTO_ADMISSION_DIR '$admission_dir' is a symbolic link; the client directory must be a real directory of its authority"
  home_quecto="$(realpath -ms -- "$HOME/.quecto")" && [ -n "$home_quecto" ] \
    || die "cannot normalize '$HOME/.quecto'"
  real_quecto="$(realpath -m -- "$HOME/.quecto")" && [ -n "$real_quecto" ] \
    || die "cannot resolve '$HOME/.quecto'"
  real_socket_dir="$(realpath -m -- "$socket_dir")" && [ -n "$real_socket_dir" ] \
    || die "cannot resolve socket dir '$socket_dir'"
  case "$real_root" in
  "$real_quecto")
    die "QUECTO_ADMISSION_DIR parent '$admission_root' is the identity-mounted ~/.quecto itself; use a subdirectory"
    ;;
  "$real_quecto"/*)
    # The authority lives inside the identity-mounted ~/.quecto, so the mask
    # must land INSIDE that mount, at the same place. The identity mount is
    # spelled "$HOME/.quecto"; a symlinked HOME is fine as long as the
    # admission dir is spelled through the same alias (the harness derives it
    # from HOME, so it is). What is refused is a mismatch — the canonical
    # spelling under an aliased HOME, or a symlink inside ~/.quecto — because
    # the mask would then sit beside the authority instead of over it.
    case "$admission_root" in
    "$home_quecto"/*) ;;
    *) die "QUECTO_ADMISSION_DIR '$admission_dir' resolves inside ~/.quecto but is not spelled under '$home_quecto'; the read-only mask would miss the identity mount — spell it under \$HOME/.quecto" ;;
    esac
    [ "${admission_root#"$home_quecto"}" = "${real_root#"$real_quecto"}" ] \
      || die "QUECTO_ADMISSION_DIR '$admission_dir' reaches its authority through a symbolic link inside ~/.quecto; the read-only mask would miss the real directory"
    admission_masked=1
    ;;
  /*)
    # Outside ~/.quecto nothing is masked, so the authority must be outside
    # every read-write mount too. The socket dir is mounted read-write whole:
    # an authority inside it would hand the container its journal, admin
    # socket and owner token.
    case "${real_root%/}/" in
    "${real_socket_dir%/}"/*)
      die "QUECTO_ADMISSION_DIR parent '$admission_root' is inside the socket dir '$socket_dir', which the container gets read-write and unmasked; keep the admission authority under ~/.quecto or outside the socket dir"
      ;;
    esac
    ;;
  *) die "QUECTO_ADMISSION_DIR '$admission_dir' has an unsupported parent path" ;;
  esac
fi

env_dir="$(mktemp -d "$state_dir/env-XXXXXXXXXX")" || die "failed to create environment dir under $state_dir"
# The resolved state root travels on the container as a label so
# `inspect --list` (#2024 S4d) can list this root's containers alone.
state_root="$(cd "$state_dir" && pwd -P)"
environment_id="$(basename "$env_dir")"
container="quecto-$environment_id"
# Rollback on any later failure: remove partial state AND any container we
# managed to start — an unreported environment can never be cleaned up by
# Quecto.
trap '"$cli" rm -f "$container" >/dev/null 2>&1 || true; rm -rf "$env_dir"' ERR
workspace_path="$env_dir/workspace"
mkdir "$workspace_path"
printf '%s\n' "$QUECTO_CONTAINER_ENVIRONMENT_REF" >"$env_dir/ref"

# The clone source is this config's own --repo (#1410); no --repo → sandbox.
child_cwd="$workspace_path"
source="none"
if [ -n "$repo" ]; then
  log "checking out $repo_shown"
  # SECURITY (PR #1401 review): this clone runs on the HOST, before any
  # container exists. Whitelist git transports so command-running helpers
  # (`ext::sh -c ...`) cannot execute host commands — a containment bypass
  # for the adapter whose whole point is isolation. The URL is now baked
  # into the trusted config's argv rather than agent-supplied, but the
  # restriction stays: config files travel.
  GIT_ALLOW_PROTOCOL="file:https:ssh:git" \
    git clone --quiet -- "$repo" "$workspace_path/repo"
  child_cwd="$workspace_path/repo"
  source="repo"
fi

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
# Shared inference admission (#1679 P3): the authority's client directory is
# identity-mounted so the child reaches the same private socket by path. The
# capability is reported only when the mount is actually present; an
# admission-enabled parent refuses to launch without it. When the authority
# lives under the identity-mounted $HOME/.quecto, its root (journal, admin
# socket, owner token) is masked with a directory holding nothing but the
# client mountpoint, and only client/ is re-exposed underneath it.
admission_capability=""
if [ -n "$admission_dir" ]; then
  # Classification and every rejecting check ran before mktemp (a `die`
  # here would leak the environment directory); only the mounts remain.
  if [ "$admission_masked" = 1 ]; then
    # An owner-only host directory bound read-only over the authority root
    # hides journal/admin/token identically under Docker and Podman (a tmpfs
    # would be copied up by Podman); client/ is re-bound beneath it. The
    # client mountpoint is created before the parent becomes read-only:
    # rootless Podman with runc cannot manufacture a nested bind destination
    # afterwards (#2068).
    mask_dir="$env_dir/admission-mask"
    mkdir -m 700 -- "$mask_dir"
    mkdir -m 700 -- "$mask_dir/$admission_leaf"
    mounts+=(-v "$mask_dir:$admission_root:ro")
  fi
  mounts+=(-v "$admission_mount:$admission_mount:rw")
  admission_capability="shared-directory-v1"
  printf '%s\n' "$admission_dir" >"$env_dir/admission-dir"
fi
# HOME is preserved and QUECTO_BASE_DIR is deliberately NOT overridden:
# QUECTO_BASE_DIR is quecto's credentials/config home ($HOME/.quecto by
# default). Overriding it inside the container detaches the child from the
# identity-mounted $HOME/.quecto and breaks OAuth providers — do not set it.
# Member harness logs go to the container's journald stream. Without RUST_LOG
# the redacting subscriber is a no-op and an environment that dies leaves no
# trace of why (termination signal, teardown, socket close). The host can
# still override the level per spawn.
envs=(-e "RUST_LOG=${RUST_LOG:-info}" -e "HOME=$HOME" -e "QUECTO_SWARM_CONTAINER=isolated-pid-v1" -e "QUECTO_SWARM_HOST_PID_NS=$(readlink /proc/self/ns/pid)" -e "QUECTO_SWARM_CHECKOUT=$child_cwd" -e "QUECTO_SWARM_BOOTSTRAP=1")
# Run as the host user so the identity-mounted paths keep their ownership.
# Under rootless Podman, --userns=keep-id maps the host uid/gid to the same
# ids inside the container (the default rootless mapping would send uid 1000
# to a subuid and every mounted file would look foreign); Docker already maps
# container uids 1:1 to the host.
run_as=(--user "$(id -u):$(id -g)")
if [ "$cli" = podman ]; then
  run_as+=(--userns=keep-id)
fi
# --init puts a minimal init (catatonit under Podman, tini under Docker) at
# PID 1 with the child as its direct descendant. The child otherwise IS PID 1
# and inherits two duties a normal binary does not perform: reaping orphaned
# grandchildren (nested agents whose parent exited pile up as zombies) and
# handling signals (PID 1 ignores SIGTERM without a handler, so `stop` hangs
# until the SIGKILL escalation). The child is still the container's liveness:
# the init exits when it does.
run_as+=(--init)
# Threads count against the container's pid cgroup, and the runtime default
# (Podman `pids_limit = 2048`) is exhausted by an in-container `cargo test`
# or a parallel build. Once the cgroup is full every fork and thread spawn
# fails with EAGAIN, worker agents lose their subprocesses and the founding
# agent cannot even tear down cleanly, so the whole environment dies and
# every member disconnects at once. Keep a fence, but a generous one;
# QUECTO_CONTAINER_PIDS_LIMIT overrides it (-1 defers to the user slice).
# `0` is refused: Docker reads it as "daemon default" and Podman ignores it
# (falling back to the 2048 this fence exists to replace).
pids_limit="${QUECTO_CONTAINER_PIDS_LIMIT:-16384}"
case "$pids_limit" in
  -1) ;;
  ''|0|*[!0-9]*) die "QUECTO_CONTAINER_PIDS_LIMIT must be -1 or a positive integer" ;;
esac
run_as+=(--pids-limit "$pids_limit")
# SECURITY (PR #1401 review): provider API keys must NOT be passed with
# `run -e KEY=value` — that bakes them into the container config,
# readable for the container's whole lifetime via `inspect` and
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
for key in ANTHROPIC_API_KEY OPENAI_API_KEY OPENROUTER_API_KEY FIREWORKS_API_KEY; do
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
  "$cli" run --pull=never -d --name "$container" \
    --label "quecto.environment_id=$environment_id" \
    --label "quecto.state_dir=$state_root" \
    "${run_as[@]}" "${mounts[@]}" "${envs[@]}" \
    -w "$child_cwd" \
    "$image" /bin/sh -c '. "$0" && exec "$@"' "$secret_env_file" "$@" >/dev/null
else
  "$cli" run --pull=never -d --name "$container" \
    --label "quecto.environment_id=$environment_id" \
    --label "quecto.state_dir=$state_root" \
    "${run_as[@]}" "${mounts[@]}" "${envs[@]}" \
    -w "$child_cwd" \
    "$image" "$@" >/dev/null
fi
# ------------------------------------------------------------------------

printf '%s\n' "$container" >"$env_dir/container"
jq -cn --arg container "$container" --arg socket "$socket_path" \
  '{container: $container, socket: $socket}' >>"$env_dir/children.jsonl"
printf '%s\n' "$environment_id" >>"$state_dir/creates.log"

# metadata.repository is how listings/TUI learn the source truthfully: the
# config owns its repository, so only the script can report it (#1410).
# metadata.checkout is the members' working directory and swarm checkout root
# (QUECTO_SWARM_CHECKOUT), identity-mounted so the supervising session can
# read the coordination store there after the members' sockets are gone and
# keep the environment instead of destroying a resumable run (#1924).
jq -cn \
  --arg id "$environment_id" \
  --arg workspace "$workspace_path" \
  --arg checkout "$child_cwd" \
  --arg socket "$socket_path" \
  --arg config "${QUECTO_CONTAINER_CONFIG:-}" \
  --arg image "$image" \
  --arg container "$container" \
  --arg cli "$cli" \
  --arg source "$source" \
  --arg repository "$repo_shown" \
  --arg admission "$admission_capability" \
  '{environment_id: $id, workspace_path: $workspace, metadata: ({runtime: $cli, image: $image, container: $container, config: $config, source: $source, checkout: $checkout} + (if $repository == "" then {} else {repository: $repository} end)), socket_path: $socket} + (if $admission == "" then {} else {admission_capability: $admission} end)'
