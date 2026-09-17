#!/usr/bin/env bash
# F8/F9 host contract checks. These are static/argv-shape checks: no fake
# Podman runtime is used. The script remains useful on hosts without Podman.
set -euo pipefail
root="$(git rev-parse --show-toplevel)"
create="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/create.sh"
join="$root/quecto-agentic-harness/assets/standard-container/scripts/runtime/exec.sh"
for file in "$create" "$join"; do
  test -x "$file"
  bash -n "$file"
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
grep -q 'jq -c . "\$config_path"' "$create"
grep -q 'config_mount_dir="/run/quecto/configs"' "$create"
grep -q 'config_mount_dir="/run/quecto/configs"' "$join"
grep -q 'child_argv\[i+1\]="\$config_mount_dir/create.json"' "$create"
grep -q 'child_argv\[i+1\]="\$config_mount_dir/join.json"' "$join"
grep -q '"\${child_argv\[@\]}"' "$create"
grep -q '"\${child_argv\[@\]}"' "$join"
# Join has an explicit local-rootless preflight before state access.
awk '/rootless=/{found=1} /state_dir=/{if (found) exit 0} END{if (!found) exit 1}' "$join"
printf '%s\n' 'F8-F9 runtime contract checks: PASS'
