#!/usr/bin/env bash
# In-container zero-signal proof (#1925 method, recorded by #1940): run the
# harness's full BDD suite inside `quecto-box:local` as the child of a
# signal-logging pid 2 (scripts/bdd-in-box/pid2_signal_log.py). Pid 2 is
# where a swarm coordinator's harness sits; any registry, fixture or
# descendant pid the suite ever signalled would land there.
#
#   scripts/bdd-in-box/run.sh [scratch-dir]   (default /var/tmp/quecto-bdd-in-box)
#
# The scratch dir (NOT /tmp: per-user tmpfs quotas) holds HOME and the signal
# log; the cargo target lives in the named volume `quecto-bdd-in-box-target`
# (remove it with `podman volume rm` afterwards). The suite runs as four
# sequential shards under the same pid 2 until #1959 (forgotten runtimes
# exhaust the 16384-pid cgroup in one process).
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
src="$(cd "$here/../.." && pwd)"
scratch="${1:-/var/tmp/quecto-bdd-in-box}"
vol=quecto-bdd-in-box-target
mkdir -p "$scratch/home/tmp"
rm -f "$scratch/home/pid2-signals.log"
podman volume create "$vol" >/dev/null 2>&1 || true
echo "revision: $(git -C "$src" rev-parse HEAD)"
echo "image: $(podman image inspect quecto-box:local --format '{{.Id}}')"
echo "started: $(date -Is)"
podman run --rm --init --name quecto-bdd-in-box \
  --userns=keep-id --pids-limit 16384 --user 1000:1000 \
  -v "$src:/src" \
  -v "$vol:/tmp/target" \
  -v "$scratch/home:/home/dev" \
  -v "$here/pid2_signal_log.py:/pid2_signal_log.py:ro" \
  -e CARGO_TARGET_DIR=/tmp/target \
  -e HOME=/home/dev \
  -e TMPDIR=/home/dev/tmp \
  -e PID2_SIGNAL_LOG=/home/dev/pid2-signals.log \
  -e RUST_LOG=warn \
  -w /src \
  quecto-box:local \
  python3 /pid2_signal_log.py \
  bash -c 'status=0; for i in 0 1 2 3; do echo "=== shard $i/4 ==="; QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=4 cargo test --workspace --features quecto-agentic-harness/test-support --bins --test bdd || status=1; done; exit $status'
status=$?
echo "exit: $status"
echo "finished: $(date -Is)"
tail -1 "$scratch/home/pid2-signals.log"
exit "$status"
