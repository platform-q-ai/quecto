# Swarm workbench

`swarm` replaces `python_lab`. It is a compiled native tool using the existing
isolated Python execution, output artifacts, resource limits and cancellation
machinery. There is one tool surface. Existing `tools.python_lab` limits remain
accepted as a configuration alias for `tools.swarm`; serialization uses the new
name. Update tool policy names and saved prompts to `swarm`. Supplying both
configuration keys is an error rather than silently choosing one.

## Start from the TUI or master agent

Ask the master to start a swarm with a goal, constraints, explicit acceptance
criteria, a member limit and a deadline. For example: “Use four members including
the coordinator to implement this change; acceptance tests and independent review
must pass at the final revision; stop after 30 minutes; do not merge.” The master
passes those requirements to a container coordinator, which creates the run and
starts its local workers. A worker finishing its turn is not swarm completion.

The master launches a normal agent through the existing `spawn` container
capability. The selected named `container_configs` entry determines the image,
repository and environment; swarm does not select a special image or start a
separate service. Use the official isolated Docker/Podman adapter and an image
with the current harness and Python 3. See [container configuration](../../docs/container-runtimes.md)
and [subagent control](subagents.md). Agents can load `docs {"name":"swarm"}`
without the documentation files being present in the container checkout.

## Inspection and results for users and master agents

| Need | Current interface |
|---|---|
| Agent/container status | Existing TUI agent views or `agent_cmd` inventory/state commands |
| Goal, criteria, task progress, blockers and evidence | Ask the coordinator to call `swarm {"op":"summary"}` and report the result |
| Detailed live task/file pages | Coordinator uses `board.tasks()` / `board.file_owners()` while the run is running |
| Coordinator report | Master reads `agent_cmd.get_messages` using the coordinator's returned agent UUID |
| Final result | Read summary status, final revision and criterion evidence; distinguish `succeeded` from blocked/failed/cancelled/budget-exhausted |
| Evidence files | Ask the coordinator to export them using ordinary file/Bash tools before environment teardown |

There are no dedicated public swarm creation, update, inspection or result UDS
endpoints, and no swarm dashboard/config panel in this change. Existing UDS agent
supervision carries prompts and reports; it does not expose the durable board as
a structured event feed. That public interface is follow-on work.

On a wake hint, inspect summary first. After a terminal outcome, `op=summary`
remains readable but `op=run` is closed: do not request inbox reads or acknowledgments
through Python. Keep the coordinator available for final reporting and export.
Returned execution artifact paths are workspace-relative inside the container;
join them to `artifact_base`, preserve them through the normal environment file
transport, and do not assume they are host paths. Record the tested commit and
binary build identity with important reports. Container deletion and execution
artifact pruning can remove evidence; board persistence is not an export.

## Container and membership

Use the existing `spawn` container capability with the official Docker/Podman
adapter. The adapter passes the shared checkout, `isolated-pid-v1` context and host PID namespace to every
in-container harness process. The harness requires a different current PID namespace;
host-local launches cannot gain availability from a directory marker. Host-local reference scripts do not confer swarm
availability. The harness rejects `swarm` outside that context, even if someone
plants a coordination file in the host checkout. Linux procfs supplies process
start identities for conservative lifecycle reconciliation.

The first in-container agent is the coordinator and designated Git integrator.
The external supervising parent is not a member. Initial harness startup records
membership in a setup board; creation cannot choose a smaller limit than the
already live/reserved population. Create the run before launching the pool.

Call `swarm` with `op=create`, a nonempty `goal`, a list of `constraints`,
`criteria`, `member_limit` (1 through 10), and `deadline` (Unix seconds, in the
future and no more than seven days away). Each criterion has an `id`, a
`description`, and `kind` of `command` or `review`. For example:

```json
[
  {"id":"acceptance","kind":"command","description":"Acceptance tests pass at the submitted commit"},
  {"id":"review","kind":"review","description":"Independent reviewer accepts the submitted commit"}
]
```

The coordinator counts toward the limit, including with a one-member pool.
Use existing local `spawn` inside the container to establish the fixed pool.
External parents may use existing container joins; startup performs the same
atomic admission. Idle members count. Changing the session name, switching
entrypoints, or spawning descendants does not establish a new run budget.
Nested container launches are rejected. An existing run cannot be reset through
the helper API. This bounds harness-managed agents, not arbitrary subprocesses
or direct provider API calls. Provider inference admission is separate (#1679).

Launch reservations precede process launch. Only reservations for processes that
never started are released automatically. Launched processes are identified by
PID and kernel start time, avoiding recycled-PID mistakes. `op=reconcile` detects
harness death, but a dead harness does **not** prove that its Bash/Python execution
groups stopped: those children can survive and be reparented. Therefore the run
fails, membership and file ownership stay reserved, and replacement claims are
rejected. Stop and discard that container environment before starting a fresh run.
The current adapter cannot safely recover an abruptly exited worker in place;
`recover` requires independently confirmed execution-scope death, which ordinary
harness reconciliation deliberately does not assert. Post-launch rollback is also
conservative. Neither idle time nor a worker's completion message frees a slot.

## Packaged API

Each `op=run` starts a fresh `python3 -I` process. The harness loads compiled-in
helper sources directly and binds the invoking member; neither `PYTHONPATH` nor
an importable file in the checkout controls the helper. Python variables do not
persist. The SQLite database at `.quecto/swarm.sqlite` does.

```python
from swarm import board

summary = board.summary()
task = board.task_create("implement-v1", "Implement the behavior", ["Acceptance tests pass"], [])
claim = board.claim(task["id"])
files = board.reserve(task["id"], claim["token"], ["src/feature.rs", "tests/feature.rs"])
# Use existing tools to implement and test. Store large output as artifacts.
board.submit(task["id"], claim["token"], [
    {"artifact": "evidence/acceptance.log", "revision": "the-exact-commit-or-artifact-digest"}
])
```

`task(id)` reads a task. `tasks(offset=0, limit=50)` and
`file_owners(offset=0, limit=50)` page the board (maximum page size 100). `dependencies(id, ids)` edits dependencies before
claiming; missing, self and cyclic dependencies fail. Ready tasks are claimed
atomically. Unmet dependencies and explicit `block(id, token, reason)` remain
visible as blockers. `release(id, token)` returns owned work and files to the
board. A fresh claim has a new token; stale tokens cannot submit, release or
complete reassigned work. `task_create` and `send` use request IDs for retry
idempotency; reusing an ID with a different payload is an error. Identical
submission and verification retries do not repeat their transitions.

File sets are reserved all at once or not at all. Paths resolve inside the shared
checkout, including symlink aliases and not-yet-created files.
`release_files(task_id, claim_token, reservation_token)` cannot remove a newer
owner's reservation. Only the coordinator may `recover(task_id)`, after the
entire execution scope is confirmed stopped. A harness exit alone cannot grant
that permission in the current adapter. There are no expiring ownership leases.

These are **cooperative reservations**, not mandatory locks: Bash and arbitrary
Python can bypass them. The container is the external containment boundary,
not a security boundary between same-user workers. Do not construct a different
`Workbench`, call underscore lifecycle methods, edit SQLite, or bypass file
ownership. Only the designated integrator changes branches, commits, or integrates
changes in the shared checkout. Concurrent Git merge automation is not provided.

## Messages and waking

Use stable member IDs from `summary()["members"]`:

```python
board.send("schema-question-v1", recipient_id, "Blocked: which schema version should I use?")
for message in board.inbox():
    print(message)
    board.ack(message["id"])
```

Messages are `accepted` when durable and `consumed` when acknowledged. Neither
means work completed. `inbox(include_consumed=True)` also reads consumed history.
Messages are at most 8192 UTF-8 bytes; each inbox admits at most 100 unconsumed
messages. Unknown/dead recipients and full inboxes fail explicitly. The board
bounds tasks to 1000 and request-ledger entries to 10000 per run. Store large data
in artifacts, not task or message text.

After actionable mutations the harness uses existing UDS prompt/follow-up capabilities
for coalesced wake hints. Reads, acknowledgments and reservation bookkeeping do not
generate hints; terminal runs suppress new hints. Failed hints are returned
as `notification_warnings`; accepted SQLite records remain recoverable. When no
work is ready, yield the turn. Do not poll, repeatedly sleep, or infer completion
from an empty ready queue.

## Verification, stopping and progress

Workers submit evidence references. Only the coordinator can verify submitted
tasks with `verify_task(id, claim_token, revision)`, accept evidence using
`evidence(criterion, artifact, revision, kind, passed)`, and `complete(revision)`.
A worker's `evidence` call records an unaccepted submission. A coordinator must
actually inspect command results and obtain the required independent/human
review before accepting them; the helper does not execute tests or act as an
independent reviewer. `command` and `review` evidence are distinguished.

Completion requires accepted evidence for every original criterion at the
specified revision, completed tasks with matching evidence revisions, and no
remaining file reservations. When later dependent work advances the checkout,
the coordinator reruns the earlier task's checks, then calls
`revalidate_task(id, final_revision, fresh_evidence)`. This requires a completed
task and nonempty artifact evidence matching that revision; it records both old
and new evidence in the audit. Workers cannot revalidate. Existing evidence is
never silently relabeled. Submitted tasks, idle agents and an empty queue do
not prove success. `amend(goal, constraints, criteria, reason)` is coordinator-only,
records the amendment, and invalidates prior overall evidence.

`op=summary` reports goal, status, membership usage/limit, task counts and the first 50 task/file details,
blockers, evidence and recent actor/timestamp audit events. Total counts include
entries beyond the first page. File reservations are bounded to 1000 per run. The full
audit remains in SQLite. `stop(status, reason)` distinguishes `blocked`, `failed`,
`cancelled`, and `budget-exhausted`; success is `succeeded`. `op=cancel_run` is the
parent cancellation operation. New work and admission stop at terminal state;
settlement cancels local detached Python jobs before attempting UDS turn abort,
then terminates workers through the process adapter. Every member's watcher checks
for remote terminal outcomes at 500 ms intervals, including while idle, and cancels
its own detached registry. The registry closes against concurrent new launches.
Reconciliation preserves readable partial progress and retains uncertain ownership. Keep the coordinator available to report to the parent.

The required run budget is wall-clock time. A harness timer supervises the deadline
even when agents are idle, and Python execution timeouts cannot exceed the remaining
run time (one-second timeout granularity). No turn or token hard caps are promised.
The helper holds no SQLite transaction across model/tool execution or lifecycle
notifications. SQLite uses short immediate transactions and a bounded contention
timeout, on a suitable **local filesystem** only. Corrupt, missing or locked state
fails explicitly; it never creates a replacement board or bypasses admission.

Execution artifacts remain under `.quecto/swarm/<execution_id>/` with the existing
32-directory pruning policy. The sibling `.quecto/swarm.sqlite` database is never
pruned with those artifacts. Container destruction remains destructive unless the
user preserves its storage; cross-container recovery is not provided.

## Architecture boundaries

The packaged Python domain (`src/domain/swarm_policy.py`) owns authorization,
deadline, admission, completion, and revalidation decisions without I/O.
Application use cases (`src/application/swarm_use_cases.py`) depend on an atomic
coordination repository and injected clock. SQLite implements that port; admission
checks and reservation writes share the same immediate transaction. SQL-facing
workbench/task adapters retain dispatch and the existing task implementation; this
is an incremental extraction, not a second implementation of the policy in Rust.

Rust domain ports expose typed membership, process identity, outcomes, coordination,
process control and clock contracts. `src/application/swarm.rs` owns reconciliation
and settlement sequencing. Infrastructure handles Python wire decoding, Linux
identity checks, UDS commands, cancellation registries and timer scheduling. Pure
policy/fake-port tests supplement the real SQLite and process integration tests.

## Python execution policy and agent guidance

The default `tools.swarm.max_processes` remains `1`, applied as `RLIMIT_NPROC`
to the Python execution process. Child launches such as `subprocess.run` can
therefore fail with `EAGAIN` even when the container has free capacity. This is
an execution-policy restriction; it is not a one-agent membership limit or a
measurement of container exhaustion. Privileged processes may be exempt from
this OS limit. A matching default-limit error includes an actionable diagnostic.
Use the existing Bash tool for Git, checks and other external commands under its
configured policy; Python remains suitable for in-process computation and board
coordination. Agent code must not raise or bypass the configured limit.

`docs {"name":"swarm"}` serves a compiled-in manual from any working directory,
including a container without the product source checkout. It documents typed
arguments, examples, bounds, common errors, evidence proposals versus acceptance,
and terminal inspection/export. Task acceptance is a nonempty `list[str]`;
message bodies are strings. Worker `evidence` calls are proposals (`accepted=0`),
not coordinator acceptance.

All returned artifact references share a `workspace-relative` namespace with an
explicit `artifact_base` naming the container execution workspace. Resolve status,
output and spill paths against that base, not against the first `.quecto`
component of a surrounding container-environment path. Host consumers need the
container's normal artifact transport, not reinterpretation as a host path.

Wake hints are selected from the invoking member's actionable events and coalesced
using a durable per-actor cursor. Reading the board, acknowledging messages and
reservation bookkeeping do not broadcast more work. Message hints target their
recipient; submissions/blockers/evidence target the coordinator; changes that
make work available notify peers. The cursor advances atomically before external
notification, so a failed hint is reported but not endlessly retried; the durable
board/inbox remains authoritative. Terminal runs generate no new actionable hints.
Already queued hints instruct the recipient to inspect `op=summary` first and,
if terminal, avoid Python inbox/ack calls while remaining available for parent
requests and artifact export.
