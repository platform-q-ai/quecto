#!/usr/bin/env bash
# Type-check the workspace for macOS from Linux (#2237), on Apple Silicon
# (aarch64-apple-darwin) and Intel (x86_64-apple-darwin).
#
# The release binaries are built for macOS only when a merged PR carries the
# `release-binaries` label, so a macOS-only compile error (a non-`Send`
# `libc::siginfo_t` held across an `.await`, #2237) went unseen until a
# release. `cargo check` never links, so no macOS SDK is needed; only the C
# dependencies' build scripts (ring) call a C compiler and archiver, and
# their outputs are never used by a check. Stubs that create empty outputs
# stand in for both.
#
# Warnings fail the check (`-D warnings`, set for this script's cargo only):
# a macOS-only unused import or dead branch is caught here like any error.
#
# Limitation: the stub C compiler accepts every input, so a build script
# that probes the C compiler to set a cfg (does this header or flag
# compile?) sees every probe succeed. Such cfgs may differ from a real
# macOS build, so this check proves the Rust types, not the C probes.
#
# Needs each target's rust standard library installed; it installs nothing
# itself. In CI (`CI=true`) a missing target fails the check; locally it is
# skipped with a message, and at least one target must be checked.
set -euo pipefail

targets=(aarch64-apple-darwin x86_64-apple-darwin)
stubs="$(mktemp -d)"
trap 'rm -rf "$stubs"' EXIT

cat >"$stubs/cc" <<'STUB'
#!/bin/sh
# Stub C compiler: create the -o output empty.
out=""
while [ $# -gt 0 ]; do
  if [ "$1" = "-o" ]; then shift; out="$1"; fi
  shift
done
if [ -n "$out" ]; then : >"$out"; fi
exit 0
STUB
cat >"$stubs/ar" <<'STUB'
#!/bin/sh
# Stub archiver: create each named .a archive empty.
for arg in "$@"; do
  case "$arg" in
    *.a) if [ ! -e "$arg" ]; then : >"$arg"; fi ;;
  esac
done
exit 0
STUB
chmod +x "$stubs/cc" "$stubs/ar"

sysroot="$(rustc --print sysroot)"
checked=0
for target in "${targets[@]}"; do
  if [ -d "$sysroot/lib/rustlib/$target/lib" ]; then
    echo "==> cargo check --target $target"
    env_target="${target//-/_}"
    env RUSTC_WRAPPER='' RUSTFLAGS='-D warnings' \
      "CC_${env_target}=$stubs/cc" "AR_${env_target}=$stubs/ar" \
      cargo check --workspace --all-targets --features quecto-agentic-harness/test-support \
      --target "$target" "$@"
    checked=$((checked + 1))
  elif [ "${CI:-}" = "true" ]; then
    echo "error: rust target $target is not installed (rustup target add $target)" >&2
    exit 1
  else
    echo "skip: rust target $target is not installed here (rustup target add $target); CI checks it" >&2
  fi
done

if [ "$checked" -eq 0 ]; then
  echo "error: no macOS rust target is installed; nothing was checked" >&2
  exit 1
fi
