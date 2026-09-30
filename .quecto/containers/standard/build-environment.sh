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
# Cargo/rustup are authenticated-agent build entry points: never import
# caller-supplied package metadata there. Compiler/cache re-entry must retain
# Cargo's generated protocol, but never admit arbitrary CARGO_* credential names.
case "${1##*/}" in
  rustc|rustdoc|sccache)
    allowed+=(CARGO CARGO_MANIFEST_DIR CARGO_MANIFEST_PATH CARGO_CRATE_NAME
      CARGO_BIN_NAME CARGO_PRIMARY_PACKAGE CARGO_PKG_NAME CARGO_PKG_VERSION
      CARGO_PKG_VERSION_MAJOR CARGO_PKG_VERSION_MINOR CARGO_PKG_VERSION_PATCH
      CARGO_PKG_VERSION_PRE CARGO_PKG_AUTHORS CARGO_PKG_DESCRIPTION
      CARGO_PKG_HOMEPAGE CARGO_PKG_REPOSITORY CARGO_PKG_LICENSE
      CARGO_PKG_LICENSE_FILE CARGO_PKG_RUST_VERSION CARGO_PKG_README
      OUT_DIR)
    ;;
esac
clean=()
for name in "${allowed[@]}"; do
  if [[ -v "$name" ]]; then clean+=("$name=${!name}"); fi
done
# No inherited provider, Git credential helper, or arbitrary build environment.
exec /usr/bin/env -i "${clean[@]}" "$@"
