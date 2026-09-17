#!/usr/bin/env bash
# F8/F9 runtime contract checks.  These exercise the retained adapters with a
# recording Podman double; no real runtime or host socket is needed.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
create="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/create.sh"
join="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/exec.sh"
inspect="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/inspect.sh"
kill="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/kill.sh"
for file in "$create" "$join" "$inspect" "$kill"; do
  test -x "$file"
  bash -n "$file"
done
# Every embedded lifecycle operation is local rootless Podman-only and uses an
# affirmative, ASCII-only ID allowlist (not a denylist with traversal gaps).
for file in "$create" "$join" "$inspect" "$kill"; do
  grep -q 'CONTAINER_HOST' "$file"
done
for file in "$join" "$inspect" "$kill"; do
  grep -Fq '[A-Za-z0-9]' "$file"
done
grep -q 'state dir must be a real directory' "$create"
grep -q 'state dir must have mode 700' "$create"
for file in "$join" "$inspect" "$kill"; do
  grep -q 'environment' "$file"
  grep -q 'environment' "$file"
done
# Immutable image approval and all four approved bindings are fail-closed.
grep -q 'QUECTO_PODMAN_APPROVED_IMAGE' "$create"
grep -q 'QUECTO_PODMAN_APPROVAL_RECORD' "$create"
grep -q 'QUECTO_PODMAN_APPROVED_RECORD_SHA256' "$create"
for field in recipe context effective_assets mount_intent; do
  grep -q 'hash_field="${field}_sha256"' "$create"
done
# The selected config is normalized into a private sidecar and the host path
# cannot survive in either create or join argv.
grep -q 'jq -c . "$config_path"' "$create"
grep -q 'config_mount_dir="/run/quecto/configs"' "$create"
grep -q 'config_mount_dir="/run/quecto/configs"' "$join"
grep -q 'child_argv\[i+1\]="\$config_mount_dir/create.json"' "$create"
grep -q 'child_argv\[i+1\]="\$config_mount_dir/join.json"' "$join"
grep -q 'child_argv\[@\]' "$create"
grep -q 'child_argv\[@\]' "$join"
# Each GH variable has an independent grant; granting one cannot copy the
# other ambient token.  This is deliberately checked in source and behavior.
for file in "$create"; do
  grep -q 'secret_grant_allowed GH_TOKEN' "$file"
  grep -q 'secret_grant_allowed GITHUB_TOKEN' "$file"
  ! grep -q 'secret_grant_allowed github' "$file"
done

fake="$(mktemp -d)"
trap 'rm -rf "$fake"' EXIT
mkdir -m 700 "$fake/state" "$fake/bin"
cat >"$fake/bin/podman" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"${FAKE_PODMAN_LOG:?}"
case "${1:-}" in
  info) printf 'true\n' ;;
  inspect) printf 'false 0 false\n' ;;
  rm) : ;;
  *) : ;;
esac
EOF
chmod 700 "$fake/bin/podman"
# A remote selector is rejected before any retained state is read or runtime
# command is reached.
if CONTAINER_HOST=tcp://127.0.0.1:9999 PATH="$fake/bin:$PATH" FAKE_PODMAN_LOG="$fake/podman.log" \
  QUECTO_CONTAINER_ENVIRONMENT_ID=env-good "$kill" --state-dir "$fake/state" --op cleanup 2>"$fake/remote.err"; then
  echo 'remote Podman selector was accepted' >&2
  exit 1
fi
! test -s "$fake/podman.log"
# Unicode, separators, option-looking IDs are rejected by every retained op.
for bad in $'env-ümlaut' 'env/escape' '..' '-env' 'env-' '--help'; do
  if PATH="$fake/bin:$PATH" FAKE_PODMAN_LOG="$fake/podman.log" \
    QUECTO_CONTAINER_ENVIRONMENT_ID="$bad" "$kill" --state-dir "$fake/state" --op cleanup >/dev/null 2>&1; then
    echo "invalid ID accepted: $bad" >&2
    exit 1
  fi
done
# A valid retained state entry can be inspected and cleaned up, and the fake
# records the rootless preflight plus exactly the requested operation.
env="$fake/state/env-good"
mkdir -m 700 "$env"
printf '%s\n' quecto-env-good >"$env/container"
PATH="$fake/bin:$PATH" FAKE_PODMAN_LOG="$fake/podman.log" \
  QUECTO_CONTAINER_ENVIRONMENT_ID=env-good "$inspect" --state-dir "$fake/state" >/dev/null
PATH="$fake/bin:$PATH" FAKE_PODMAN_LOG="$fake/podman.log" \
  QUECTO_CONTAINER_ENVIRONMENT_ID=env-good "$kill" --state-dir "$fake/state" --op cleanup
! test -e "$env"
grep -q 'info' "$fake/podman.log"
grep -q 'inspect' "$fake/podman.log"
grep -q 'rm -f' "$fake/podman.log" 2>/dev/null || grep -q '^rm ' "$fake/podman.log"
printf '%s\n' 'F8-F9 runtime contract checks: PASS'
