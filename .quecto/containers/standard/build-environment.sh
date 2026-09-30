#!/bin/bash -p
# Deliberately a name allowlist, not a list of known credential names.
# Build artifacts can still contain arbitrary data read or embedded by a build.
set -euo pipefail
[ "$#" -gt 0 ] || { printf '%s\n' 'build-environment: command required' >&2; exit 2; }
allowed=(PATH HOME USER LOGNAME LANG LC_ALL LC_CTYPE TZ TERM TMPDIR TMP TEMP
  CARGO_HOME RUSTUP_HOME RUSTC_WRAPPER SCCACHE_DIR SCCACHE_CACHE_SIZE
  CARGO_BUILD_JOBS SCCACHE_IDLE_TIMEOUT
  CARGO_TARGET_DIR RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTDOCFLAGS
  LLVM_PROFILE_FILE)
clean=()
for name in "${allowed[@]}"; do
  if [[ -v "$name" ]]; then clean+=("$name=${!name}"); fi
done
# No inherited provider, Git credential helper, or arbitrary build environment.
exec /usr/bin/env -i "${clean[@]}" "$@"
