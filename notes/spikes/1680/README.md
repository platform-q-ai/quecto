# #1680 swarm workbench — exploratory spike (not issue completion)

This branch adds an **opt-in** installable observation runner and repurposes
Python Lab as `swarm` only for its workers (`QUECTO_SWARM_SPIKE=1`). Default
Quecto remains unchanged; workers get only one surface, not two Python tools.
The existing `tools.python_lab` configuration and execution/output/background
cancellation machinery are retained. Packaged stdlib helpers are embedded in the
binary and explicitly loaded under `python3 -I`; no pip/PYTHONPATH dependency.
SQLite files live at your explicit path, outside Python execution artifact pruning.

## Build / local-install handoff

Run **inside a disposable Linux Docker/Podman container**, not on your host.
The spike fails closed unless `/proc/self/mountinfo` matches supported Docker or
Podman mounted-hostname/containerenv signatures. File-marker presence alone is
not accepted; other runtimes/layouts may be rejected. This is narrow deployment
recognition, **not trusted container attestation**.

```sh
git checkout spike/1680-swarm-workbench
cargo build --locked -p quecto-agentic-harness --bins
# Optional isolated installation, no global PATH/config modifications:
cargo install --locked --path quecto-agentic-harness \
  --bin quecto --bin quecto-swarm-spike --root "$PWD/.spike-install"
export SPIKE="$PWD/.spike-install/bin/quecto-swarm-spike"
mkdir -p "$PWD/.spike-observation"
export OBS="$PWD/.spike-observation"
"$SPIKE" demo "$OBS/fake.sqlite" notes/spikes/1680/demo-config.json
"$SPIKE" inspect "$OBS/fake.sqlite"
```

The explicit demo config increases Python Lab's process/thread limit from 1 to
256 because Linux RLIMIT_NPROC counts threads and other processes for that UID.
It is **not the agent membership cap**. No configured limits are silently reset.
The demo uses two concurrent **fake arithmetic worker threads**, distinct SQLite
connections, task/dependency claims, durable inbox messages, and a scripted
coordinator verifying the sum of squares is 55. It is not evidence of LLM reasoning.
Existing DB paths are refused, never overwritten.

Inspect without the tool wrapper (SQLite CLI is optional):

```sh
python3 - "$OBS/fake.sqlite" <<'PY'
import sqlite3, sys
c = sqlite3.connect('file:' + sys.argv[1] + '?mode=ro', uri=True)
for table in ['swarm', 'members', 'tasks', 'messages', 'evidence', 'reservations']:
    print(table, c.execute('select * from ' + table).fetchall())
PY
```

## Actual harness worker pool

```sh
# CONFIG must be an explicit existing Quecto config for the desired provider.
# The normal credentials lookup applies. This incurs provider requests/costs.
# No provider selection or host credentials are changed by this spike.
"$SPIKE" live "$OBS/live.sqlite" /absolute/path/to/container-config.json 3 120
# Concurrent observation from another terminal:
"$SPIKE" inspect "$OBS/live.sqlite"
# Preferred cancellation: Ctrl-C in the runner terminal (kills/reaps worker groups).
# Board-only cancellation, no remote process signalling:
"$SPIKE" cancel "$OBS/live.sqlite"
```

`live` launches existing `quecto agent --no-session` CLI processes in this one
container, with a fixed pool of 2–10 slots **including one scripted internal
coordinator**. It does not use the full `spawn` readiness/notification registry;
that integration is deliberately deferred. It reserves the entire pool before
launch and never replaces workers. One advisory kernel lock serializes spike
pools at `/tmp/quecto-swarm-spike-pool.lock`. Idle/stopping slots stay reserved
until exit; default `spawn` and `agent_cmd` entrypoints reject in spike workers,
including attempts to re-enable their tool policy. Nested runner calls inherit
worker mode and reject; the kernel pool lock also blocks overlapping runners.

The membership ceiling is for this **narrow runner-managed pool**, not arbitrary
Quecto processes elsewhere in the container. Same-user arbitrary Python/Bash can
strip environment, change lock/DB files, or run other binaries: **no malicious
worker/process sandbox guarantee**. Do not use this to claim issue B is complete.
No credentials are required by the fake demo. Arithmetic workload does not need
repository writes, Git, subprocess tools, or external network beyond inference.
Live workers still possess ordinary tools; instructions are not mandatory locks.

Worker stdout/stderr are in `DB-with-extension-.logs/`. At 12 iterations or the
configured 5–600 second wall deadline, existing CLI/runtime budgets stop work.
Runner cancellation first marks the board, then kills/reaps dedicated process
groups, preserving the DB. Python executions use existing Lab limits. This is
best-effort cleanup, not crash-proof supervision; SIGKILL of the runner is not
settlement. Board-only cancel prevents helper mutations but does not itself kill
workers; use Ctrl-C for immediate process cancellation. Completion is accepted
only after every worker exits successfully and the coordinator checks the exact
five arithmetic evidence pairs. The real-provider run has not been measured for
quality, latency, cost or fairness. One fast worker may claim all tasks.

## Acceptance criteria deliberately omitted / weakened

- A: no default/global rename or config/help migration; no authenticated parent
  amendments, constraints schema, or trusted runtime identity plumbing.
- B: no dynamic membership, replacement/reconciliation, shared general harness
  admission ledger, one-slot model coordinator, or full lifecycle integration.
- C: coordinator creates tasks; dependencies must preexist (cycles impossible).
  No blocked/submitted/review distinction, retry IDs, recovery/reassignment.
- D: durable messages, but no bounded inbox, ack/retry dedup, host wake hints or
  lifecycle notifications. Workers stop on no ready work; never busy-poll.
- E: single-path cooperative reservations only; no atomic multi-file reservation,
  reservation tokens, symlink/case-alias canonicalization, or dead-worker reclaim.
- F: helper budget fields are **self-reported bookkeeping**, not provider token
  accounting; `token_budget=0` in demos means no claimed metering. Deadline checks
  reject work but do not independently watchdog/transition status. Cancellation,
  deadline and worker failure currently share cancelled state; incomplete evidence
  can leave an active board for inspection (never false success). Free-form done
  attestation outside demo is not independent verification.
- G: coherent snapshot but no full timestamped event journal, live/reserved
  heartbeat projection, artifact hashes, payload/database growth quotas, or
  corruption recovery. Large real evidence should be external artifacts.

Store, identity and goal protections are **cooperative API correctness**, not
security between same-user workers. SQLite local filesystem only. Container
removal destroys unpreserved storage. The designated integrator alone may change
branches/commit; no concurrent Git automation exists here.

**#1679 seam:** each CLI worker retains normal provider admission behaviour. This
branch does not share a scheduler or depend on C2 work. Future integration belongs
at harness launch/admission, not Python's SQLite task budget.

## Deterministic validation and rollback

```sh
python3 -m unittest discover -s quecto-agentic-harness/src/infrastructure/tools/swarm_python -v
cargo test --locked -p quecto-agentic-harness --lib swarm
cargo clippy --locked -p quecto-agentic-harness --bin quecto-swarm-spike -- -D warnings
cargo fmt --all -- --check
```

Helper and embedded-import tests were observed RED then GREEN. Fake demo verifies
real concurrent SQLite behaviour without providers. Tests are not all #1680 ACs.

After stopping the runner and confirming worker processes have exited:

```sh
# Only these explicitly created disposable paths; preserve DB first if desired.
rm -r -- "$PWD/.spike-install" "$PWD/.spike-observation"
unset SPIKE OBS QUECTO_SWARM_SPIKE
```

Leave the empty kernel-lock file alone (unlinking a live lock can split its inode).
Python Lab artifacts remain in the normal workspace artifact location and follow
normal pruning. No host install/config changes are authorized or performed.
Do not merge or close #1680 based on this spike.

## Observe real Quecto processes without paid inference (recommended first)

The local fake OpenAI-compatible server returns deterministic `swarm` tool calls.
This exercises **real independent `quecto agent` processes**, packaged Python
execution, claims, peer messages, evidence and coordinator verification. It is
explicitly **not LLM reasoning**. Start the server in a separate terminal inside
the same container:

```sh
python3 scripts/swarm-spike-fake-provider.py --port 8765
# In another terminal, using the SPIKE/OBS variables above:
"$SPIKE" live "$OBS/harness-fake.sqlite" \
  "$PWD/notes/spikes/1680/fake-provider-config.json" 3 30
"$SPIKE" inspect "$OBS/harness-fake.sqlite"
# Stop the server with Ctrl-C. For cancellation observation restart it with:
python3 scripts/swarm-spike-fake-provider.py --port 8765 --delay-seconds 60
# Start another fresh live DB, then Ctrl-C its runner while requests are pending.
```

The server binds loopback only, uses dummy credentials, and makes no external
calls. Fake workers partition arithmetic tasks (not the production scheduler),
exchange peer hello messages, and briefly pause to make ownership observable.
Do not send production credentials to this test server.

Automated real-process test (build both binaries first):

```sh
python3 scripts/test-swarm-spike.py "$SPIKE"
```

This allocates an ephemeral loopback port and temp DB/config, asserts five exact
results, three fixed members, cap/nested/overlap rejection and SIGINT cancellation.
It launches no paid provider, cleans its own temporary files, and does not modify
global config. Board cancellation settlement is bounded to two seconds before
workers are killed; if settlement times out, the error explicitly requires partial
DB inspection. The wall timer starts before board setup, not after launching.
