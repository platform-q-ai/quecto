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
capability. **The swarm runs in the container the coordinator was spawned
into**: the master picks a `container_configs` entry by name —
`agent_cmd {"agent_id":"*","command":"get_container_configs"}` lists the
effective names for the master's checkout, and the spawn tool description
carries the same roster — and launches the coordinator with
`"container": {"mode":"new","container_config":"<name>"}` (`"container": true`
selects this repo's `standard` entry when one exists — no global default
overrides it; run `quecto container init` first if the roster shows none —
else the labelled default). That entry determines the image, repository
and environment; swarm does not select a special image or start a separate
service. Workers are then spawned by the coordinator with `container`
omitted (a local spawn inside the shared container). **Only the official
isolated-PID Docker/Podman adapter (`scripts/container-runtime/docker`) can
host a swarm**; the host-local reference scripts (`scripts/container-runtime/*.sh`)
cannot, and neither can the host. Use an image with the current harness and
Python 3. See [container configuration](../../docs/container-runtimes.md)
and [subagent control](subagents.md). Agents can load `docs {"name":"swarm"}`
without the documentation files being present in the container checkout.

## Inspection and results for users and master agents

| Need | Current interface |
|---|---|
| Agent/container status | Existing TUI agent views or `agent_cmd` inventory/state commands |
| Goal, criteria, task progress, blockers and evidence | Ask the coordinator to call `swarm {"op":"summary"}` and report the result |
| Detailed live task/file pages | Coordinator uses `board.tasks()` / `board.file_owners()` while the run is running |
| Coordinator report | Master reads `agent_cmd.get_messages` using the coordinator's returned agent UUID |
| Final result | A run that ended is `paused` holding `summary.outcome` (`succeeded`, `blocked`, `failed`, `budget-exhausted`) and `summary.outcome_reason`; read the final revision and criterion evidence, then resume or close it with `swarm_control` |
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
`criteria`, `member_limit` (1 through 25), and `deadline` (Unix seconds, in the
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
groups stopped: those children can survive and be reparented. Therefore a running
run pauses holding `failed` (a paused run keeps its pause clock and any verdict
the coordinator had already proposed), membership and file ownership stay
reserved, and replacement claims are rejected. Close and discard that container
environment before starting a fresh run.
That loss is recorded once per member, and only by an observer with authority
over the member's fate (#1961): the harness that launched it (the actor of its
reservation, recorded as the member's `launcher`). Another member's reconcile
may see the vanished pid but records nothing while the launcher lives; once the
launcher is itself dead or lost, any member may record the loss. Every recording
waits a grace of ten seconds from the first authorised observation (one
`scope_observed` event per observer and member), because the launcher's
owned-handle reaper normally confirms the death first (below) and a confirmed
death never pauses the run. A later reconcile seeing the same vanished pid
(after the master resumed the run) does not pause it again, and `revoke`
reassigns the lost member's work. The bootstrapped coordinator has no launcher:
its loss is recorded from outside the container (#1924), unchanged.

A member exit the launching harness observed itself is different (#1961): the
harness that spawned a member owns its process, and when that process exits
(on its own, or because the coordinator ended it with `agent_cmd kill`, or a
launch was rolled back after the child exited) the exit is authoritative for
the member's harness. The member is confirmed dead, its active tasks block for
`recover(task_id)`, and the run keeps running. What happens to its file
reservations depends on how it ended, because reservations are cooperative and
Bash tool children run in their own process groups: an **orderly** end (an exit
code, a protocol shutdown, a delegated kill, or a fallback signal this harness
sent to the member's whole group) ran the member's own teardown, so its
reservations are released and the tasks read `worker death confirmed;
coordinator recovery required`; an **abrupt** end (a signal nobody here sent —
the OOM killer, an operator — or an unobservable exit) may leave an orphaned
`cargo test` or build still writing the reserved paths, so the reservations are
retained, the tasks read `worker death confirmed (abrupt exit; reservations
retained); coordinator recovery required`, and the `death_confirmed` event
carries `reservations_retained` and the reason. The coordinator decides:
`revoke(task_id, reason)` frees them and records why, or
`recover(task_id, release_files=True)` frees them explicitly; a plain `recover`
refuses while they are retained. Only a harness death nobody observed this way
(a socket loss, a vanished pid of a member this harness did not launch) is the
conservative quarantine above; a socket loss alone never confirms a death.
Neither idle time nor a worker's completion message frees a slot.

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
owner's reservation. Only the coordinator may `recover(task_id)`, once the
owner's death is confirmed by the harness that launched it (above), and only
the coordinator may `revoke(task_id, reason)`: it takes a claim back from an
owner that will not finish, alive or not, on a claimed, blocked or submitted
task. The task returns to `ready` with no owner, token, blocker or evidence,
its file reservations go, a `revoked` event records the reason and previous
owner, and the previous owner receives a board message so a member that wakes
later learns its claim is gone (its next owned call fails with `stale or
unowned claim`). Revoking an unowned task is a no-op returning the task. There
are no expiring ownership leases.

A claimed, blocked or submitted task row (`task(id)`, `tasks()`, the summary's
first 50 tasks) also says how its owner is doing, read side only (#1969).
`owner_last_activity` is the number of seconds since the owner's most recent
board event by the store clock. `owner_state` is the store's affirmative view
of that member, authoritative signals first: `dead` when the harness that
launched it confirmed its exit; `lost` when its harness loss was recorded
(`scope_unknown` after its latest activation, #1924/#1961); `reserved` when it
was admitted but never launched; `active` or `idle` only for a live launched
member, `idle` meaning it has written no board event for 300 seconds; and
`unknown` for anything else. An `active` or `idle` owner is named as a `send`
recipient in `contact` (`board.send(request, '<owner id>', body)`); for every
other owner `contact` is null and `recovery` says how the coordinator moves the
work (`recover(task) or revoke(task, reason)` for a dead owner; resume the run,
then `revoke`, for a lost one). Board activity is the only liveness the store
sees: an idle owner may be mid-turn running a long command, so `idle` is a
prompt to look (or `send`), not proof of a stall, and nothing follows from it
automatically. A provider suspension is a harness fact the board cannot see;
`agent_cmd status` (get_state's `automaticTurnsSuspended`) is the source for
that. Unowned tasks carry none of these fields. The summary's `counts` add
`members_without_claim` (live or reserved members other than the coordinator
holding no active claim) and `members_dead`. Because an owner turns idle by
the clock alone, with no event to move the cursor, `summary(since=cursor)`
returns a full summary rather than `unchanged` while an owner is idle who was
not at the time of the cursor's event, and both responses carry
`next_liveness_check_at`, the store-clock instant the earliest active owner
would turn idle (null when none would), so a caller knows when to look again.

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

Any member may message any other member directly; nothing routes through the
coordinator. A question about a task you depend on belongs with that task's
owner, which its row names as `contact`. Messages are `accepted` when durable
and `consumed` when acknowledged. Neither means work completed. `inbox(include_consumed=True)` also reads consumed history.
A message may carry the `revision` it is about, and `send(..., supersedes=id)`
retires your own earlier unread message to the same recipient in the same
transaction; `withdraw(id)` retires one without a replacement. Retired messages
leave the inbox and produce no further wake, but stay in the audit as
`superseded` (with `superseded_by`) or `withdrawn` (#1837). Sending needs a
running run; `withdraw`, like `ack`, is bookkeeping and also works while the run
is paused. These are vocabulary the members may use; nothing requires them.
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

Workers submit evidence references. Once submitted, those references are immutable
under the claim token (identical retries are harmless). To revise rejected evidence,
release and reclaim the task, then submit with the new token so an earlier review
cannot verify the replacement. A submitted task cannot be changed to blocked. Only the coordinator can verify submitted
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
records the reason and complete before/after goal, constraints and criteria, and
invalidates prior overall evidence. The creation event preserves the original
contract. Existing audit events from older versions are not retroactively reconstructed.

`op=summary` reports goal, status, membership usage/limit, task counts and the first 50 task/file details,
blockers and evidence. History is opt-in through `op=events` (`after`, `limit` 1–100); `since=event_cursor` suppresses unchanged summary payloads. Total counts include
entries beyond the first page. File reservations are bounded to 1000 per run. The full
audit remains in SQLite. Every end of a run is a resumable pause that only the
supervisor outside the swarm lifts (#1729): `stop(status, reason)` with
`blocked`, `failed` or `budget-exhausted`, `complete(revision)` (`succeeded`),
an observed-token budget and the wall-clock deadline all move the run to
`paused` holding that outcome and reason (`summary.outcome`,
`summary.outcome_reason`, the control receipt's `outcome`). Nothing is killed:
members stay live with their claims, reservations, inboxes and evidence; their
execution and inference suspend as for any pause, while the coordinator keeps
reporting (native `summary`, `events`, `usage`; its own finished interpreter is
cancelled). The supervisor then either resumes the same run (`swarm_control
resume`, which extends the deadline by the paused time, clears the outcome and
wakes every member) or closes it (`swarm_control close`), which makes the held
outcome terminal and settles: local Python is cancelled everywhere, and each
member is ended by the harness that launched it (#2121), which records the
end as deliberate first, so a close posts no "exited unexpectedly" notes. The
coordinator also ends members whose launcher is gone. Any other member only
stops its own work and waits. If the coordinator's harness is gone, each
member ends its own launchees and the members whose launcher is gone, itself
last. A member still alive 90 seconds after settling (three teardown
conclusion bounds) ends itself; its launcher then reports that exit, since
the teardown did not go as planned. A member that joined from outside the swarm is ended by the
coordinator over its endpoint, so the harness outside that launched it still
reports its exit. Workers are aborted and asked to shut down by delegation (the `shutdown` protocol over the
endpoint each member registered, the locally owned handle only for a member
this harness launched itself; a member reachable neither way is reported as
a settlement failure — no member is ever ended by its pid, and a member's
recorded pid is only the store's liveness observation), and the
coordinator harness stays for reporting. A run paused for `budget-exhausted` refuses to resume until
the supervisor grants budget (`swarm_control extend` with `deadline_seconds`,
or `usage_budget`); the refusal names what to grant. Members, including the
coordinator's `swarm {"op":"resume"}`, cannot resume or close a run. Only
`cancelled` (`op=cancel_run`, the parent cancellation operation) is terminal at
once. Put final report data on the board before calling `complete` or `stop`:
the interpreter that proposes the outcome is cancelled as soon as the watcher
observes it, and the execution registry admits nothing but native reads until
the supervisor resumes the run (it closes for good only on close or cancel). Each Python invocation owns
its ordinary process group until cleanup: even if Python returns first, remaining
ordinary children are terminated before its result is published. On Linux, the
interpreter is reaped only after group cleanup, preventing PID reuse during
termination. Timeout and dropped-invocation cleanup follow the same rule.
With operator-configured subprocess permissions, wait/join children whose work
must finish. Background mode makes the invocation asynchronous; it does not let
children outlive the invocation. This does not add containment for intentional
process-group/session escapes.
Reconciliation preserves readable partial progress and retains uncertain ownership. Keep the coordinator available to report to the parent.

A swarm container lives as long as its swarm, and no longer (#1924, #2070).
A swarm ends only when its owner says so: the supervisor outside the swarm
closes the run into its outcome (`swarm_control close`), or the owner
explicitly leaves everything it owns behind — delete-all, a session
transition (`/new`, `/resume`) that succeeds, or **an ordinary exit of the
TUI that owns the harness** (Ctrl-D, `/exit`, `/quit` with the default
kill-on-exit). A `/resume` claims and loads
its target BEFORE the fleet is settled, so one that is refused — the session
is held by another process, missing or unreadable — ends nothing. When the
swarm has ended, the final member's exit removes the container, its checkout
and its board with the retained `kill`, exactly as for an ordinary
container; nothing is kept. (These reach the swarms whose coordinator is a
live direct child of the harness that acts: one owned a level further down
still needs `kill_container`, and so does a container already emptied and
`retained` — except on the announced TUI exit below, which ends those too.)

The TUI's ordinary exit reaches the harness as a bare termination signal —
the same signal a logout, a reboot or an operator's `kill` sends — so the
TUI **announces** the exit first: the `persist_session` it sends before
signalling carries `restoreReason: "ordinary_tui_exit_stopped"`, and the
harness records that the owner is exiting. The shutdown that follows is
then the owner's word: its fleet teardown gives every swarm's container up,
and the harness also ends the environments this harness process had
created, emptied and kept `retained` earlier (a coordinator that crashed
and left its box behind; one a previous process created is `restored` and
stays explicit-kill-only). Those kills run concurrently, and every retained
kill script is bounded (20 s, `KILL_SCRIPT_BOUND`): past it the script is
killed and the record is `cleanup-failed` with the reason, retryable by an
explicit `kill_container`, and the exit goes on. A TUI that is killed
outright or crashes sends no announcement (its owned harness then shuts down
on the kernel's SIGTERM, #2053), so that shutdown keeps every swarm resumable;
a TUI ending on SIGHUP, SIGTERM or SIGINT runs the ordinary exit and announces;
`--detach-on-exit` announces nothing either, because the harness lives on.
The announcement is held by the connection that made it and withdrawn when
that connection closes — a TUI that dies in its exit window leaves nothing
raised for a later crash to mistake for the owner's word; a shutdown that
already read it decided once and is unaffected — and it is read on the
connection's reader task, so an exit while a turn is running still counts.

Nothing else ends a swarm. While its run has not been closed — `running`,
`paused`, paused holding an outcome nobody closed yet, or `cancelled` (the
coordinator agent cancels its own run; an agent never ends a swarm) — the
final member's exit withholds the teardown: the record becomes `retained`,
so the board, checkout and unpushed branches survive and the run can be
inspected and, once relaunching a coordinator is wired, resumed. That holds
for a crash, a lost coordinator, an `agent_cmd kill` of that one member, and
the master's own shutdown, which can be a crash too (a termination signal,
its last client gone, a lost parent). A retained
container is removed by an explicit `kill_container` (from the host master
or a later session). A store that exists but cannot be read is kept as well:
it is never proof that the run ended.

When the
coordinator's socket closes after an orderly end (the run already paused
holding an outcome), the record becomes `retained` with `metadata.retained`
reading `run ended: <outcome>; ...` and the run is untouched. When it closes
while the run is `running` or paused without an outcome (its harness took a
termination signal, crashed, or was killed behind the master's back), that is
a loss: the coordinator is quarantined exactly as an in-swarm reconcile treats
a lost harness, the run is paused holding `failed`, `metadata.retained` names
the lost coordinator, and the control receipt's `resume_blockers` names the
coordinator that must be relaunched before a resume can proceed (a resume
attempted before that is refused with the same text). Relaunching it against
the surviving store is not wired yet; a join into the retained environment is
admitted for inspection but does not revive it.

The required run budget is wall-clock time. A harness timer supervises the deadline
even when agents are idle, and Python execution timeouts cannot exceed the remaining
run time (one-second timeout granularity). There is no turn cap. An optional observed-token budget can durably pause admission; it does not cancel already billed usage or guarantee a provider-side spending cap.
The helper holds no SQLite transaction across model/tool execution or lifecycle
notifications. SQLite uses short immediate transactions and a bounded contention
timeout, on a suitable **local filesystem** only. Corrupt, missing or locked state
fails explicitly; it never creates a replacement board or bypasses admission.

Execution artifacts remain under `.quecto/swarm/<execution_id>/` with the existing
32-finished-directory retention limit per tool instance. Execution IDs carry an
opaque owner prefix; pruning only touches that registry’s directories and excludes
its live executions. Other members’ output and directories from previous tool
instances are retained until explicitly exported/cleaned or the environment is
discarded. The sibling `.quecto/swarm.sqlite` database is never
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

The Rust domain (`src/domain/swarm.rs`) holds the typed membership, process
identity and outcome vocabulary; the coordination, run-control, process control
and clock ports are the swarm capability's (`src/application/swarm/ports.rs`),
and `src/application/swarm/mod.rs` owns reconciliation and settlement sequencing. Infrastructure handles Python wire decoding, Linux
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

## Workflow exclusion and awaiting approval

Workflow is fully unavailable for swarm coordinators and workers: no workflow
tool, engine, guards or automatic nudges are installed. The distinction is swarm
participation, not containerization (#1715): a container whose run is still the
bootstrap placeholder is an ordinary container and keeps workflow like a
host-local agent. Once the run has been created, worker launches reject
`workflow: true`, `workflow_guards: true` and a non-null `workflow_spec`, a
join into that container with any of them is refused at startup before
inference, and `create` is rejected while the creator is running a workflow
(guards, a bound spec or a selected template). An idle, merely available
workflow tool does not block creation; it refuses every action once the run
exists. Members that joined the container before the run was created become
swarm agents the moment it exists: their workflow tool refuses and their
local launches reject workflows, but an engine they already engaged (guards or
a bound spec) is not torn down, so create the run before spawning members. A
host-local master may still use a workflow to supervise the swarm.

For a clarification or approval, keep the run **running**, mark the affected task
with `board.block(task_id, claim_token, reason)`, report the exact question to the
master and yield the turn. Do not sleep/poll. `board.stop('blocked', ...)` ends
the run: it becomes a pause holding `blocked` that only the master can resume or
close, so use it when the whole run cannot proceed without the master, not to wait
for one task. A blocked **task** retains its claim; use `unblock(id, token, reason)` after the answer arrives. For a whole-run wait the coordinator may `pause`; only the master resumes (the deadline is extended by the paused interval).

The master sends the answer with `agent_cmd` `prompt` when idle, or `steer` when
it must interrupt a busy coordinator. A queued `follow_up` waits for the current
turn to finish. The coordinator should explicitly acknowledge the answer and
apply it to the task before optional inbox work. Transport acceptance means the
command was queued, not that the model read or acted on it; retrieve the report
with `get_report` to verify handling. `get_state.controlReceipts` correlates queue/start/completion/failure/cancellation with the command ID; turn completion is not proof of semantic compliance. A full/closed dispatch queue returns an
explicit failure without cancelling the current turn or recording pending steering.
Explicit `abort` remains effective even when the dispatch queue is full. A terminal run
cannot be revived by steering: preserve its report and start a fresh environment
when further implementation is authorized.

### Local trial regression handling

Wake recipients are selected against the current transactional board state: consumed messages and already-claimed tasks do not generate stale hints, and dependency work wakes peers only when it is claimable. Identical blocker and evidence updates are no-ops; changed blockers still notify the coordinator. Contract amendments still notify members. Hints already accepted by a recipient may become stale before execution; always inspect the summary and durable inbox before acting.

Reusable tool-runtime construction receives swarm context explicitly from the production entrypoint; it does not discover or join a pool from ambient environment variables. Ordinary Bash commands strip the six `QUECTO_SWARM_*` launch-context variables. Consequently test binaries and harness subprocesses started by build/test commands do not enroll in the live pool. Use managed `spawn` for actual swarm members; do not use Bash to launch participating agents. This separation is cooperative environment hygiene, not a security boundary.

An accepted steering request takes priority over buffered follow-up work at the idle boundary. Forwarded prompt/steer/follow-up requests retain their correlation ID; the immediate response carries `data.status: "accepted"`. Acceptance is queue admission. A subsequent `queued` response confirms pending retention; a full pending queue returns a correlated failure instead of dropping work silently. Inspect the agent transcript to verify actual handling; neither acceptance nor turn completion proves that requested work succeeded. For busy agents with no final answer in the unread report, bounded report selection favors the newest progress. Explicit history pages remain available for omitted older entries.

## Operational diagnostics and budgets

Member harness logs are captured by the container runtime: the Docker/Podman
adapter passes `RUST_LOG` (default `info`; the host's value wins) into every
environment, so the members' tracing output reaches the container's journald
stream. Under rootless Podman (journald driver) read them with
`journalctl --user CONTAINER_NAME=quecto-env-<id>`, where `env-<id>` is the
environment id from `get_containers` (add `-f` to follow, `--since` to
scope); under Docker use `docker logs quecto-env-<id>`. An environment that vanished also leaves a
`kill.log` entry in the adapter's state root naming the operation that removed
it. See [Container runtimes](../../docs/container-runtimes.md#the-official-dockerpodman-adapter).

The compiled [agent manual](docs-tool-embeds/swarm.md#durable-supervisor-controls-and-reports)
contains supervisor command examples, receipt semantics, raw export and budget
configuration. `swarm_control` pause/resume/close/extend/status/usage_budget
bypass the model queue and route through ancestors to the addressed member. Swarm creation and a
general dashboard event API remain separate follow-on work.

Each attempted logical request records available provider input/output/cache
usage, unavailable values as null, retry and OAuth-refresh counters, outcome,
duration, context estimate, and a hash of the logical system/tool prefix. The
prefix comparison includes rejected logical requests and does not prove provider
cache eligibility. Counters describe instrumented orchestration calls, not
independently observed HTTP transactions or billing. Audit fallback usage is
labelled `context_estimate`; provider objects are labelled `provider_usage_object`.
Session diagnostics retain the latest 64 requests plus cumulative counts. The
swarm SQLite ledger retains up to 10,000 request observations and fails explicitly
at capacity. Duplicate request IDs are counted once. An accounting outbox retains
cancelled and unacknowledged observations for idempotent retry at turn boundaries;
persistence failure stops automatic work.

`op=usage` exposes per-member totals and the latest 10 observations. Budget limits
count observed context-input plus output tokens, warn once at 80%, and pause at
the limit. Strict unknown-usage handling pauses on an attempted request lacking
usage. Budgets default off; explicit null disables them. Concurrent requests can
overshoot before their usage arrives. This is not a provider billing guarantee.

Runtime provenance reports process identity, package version, optional build-time
revision/dirty status, and a background-computed executable SHA-256. A workload
checkout revision is never substituted for the harness build revision. Digest
collection can be pending or unavailable and does not block session inspection.

`get_report` is cursor-neutral and bounded to 8,192 content bytes. Its explicit
`export_raw:true` option produces retained message/spill JSONL and a SHA-256
manifest under the target runtime's `artifacts/session-exports` directory. Exports
are retained until user cleanup; each is limited to 256 MiB. Message state is
captured at an epoch/revision; spill reads follow that snapshot and may exclude
concurrent appends. The manifest states this scope. Previously cleared/evicted
messages are not reconstructed. Use normal container artifact transport to copy
these files to the host.

Resume (supervisor only) restores admission, clears any outcome the run was
holding and extends the deadline. It also re-arms every
member whose automatic turns were suspended by a provider failure: the
suspension is dated by the control generation current after the failed turn
(a pause and a resume each bump it), a resume wakes every live member with
its new generation (the store's notification policy targets nobody for a
resume, so the resuming process sends these wakes itself; a `swarm_control`
resume also wakes its own process, while a coordinator resuming through the
`swarm` tool is mid-turn and needs no wake), and a member that sees a newer
generation than its suspension re-arms and runs one turn to continue its
interrupted work. Resuming an already running run repeats the fan-out with
the unchanged generation: nobody re-arms or gains a turn from it, so it is
safe but costs one wake round trip per member. Members a resume could not
reach are listed as `wake_warnings` on the receipt. Neither a prompt nor a steer
is needed after a resume. A durable store rejection is not re-armed this
way; it waits for an explicit instruction. Any explicit instruction (prompt,
follow_up or steer) re-arms a suspended member once the run admits it (a
paused run keeps it queued): a parent's fast-acked prompt to an idle member is
queued as a follow-up and executes rather than waiting behind the suspension
(#1712), while buffered automatic notifications and the harness's own wake
nudges stay queued until the member is re-armed. Controls sent to a busy
member are accepted or rejected on command-queue and pending-queue capacity
alone; poll connections do not consume that capacity (#1720). Do not pause a
run because one member failed: pause is a whole-run wait. Resume the run (the
parent may send `swarm_control` `resume` to the coordinator's socket at any
time; it is
handled below the model) and, only if a member is still stuck, steer it. When a
member holds a claim it will not finish, the coordinator has three tools and
no prescribed order: an explicit `agent_cmd` `steer` or `follow_up` re-arms a
member suspended by a provider failure (#1712); `agent_cmd` `set_model` moves
it off a failing provider; `board.revoke(task_id, reason)` reassigns its work
so another member can claim it. `recover(task_id)` applies once the member's
death is confirmed (its harness exited under the coordinator's own
observation, for instance after `agent_cmd kill`). A run paused because a
strict budget saw an answered request without usage re-pauses on the next
admission until `strict_unknown` is disabled (or the budget removed); raising
the limit alone does not clear it. Cancelled and rejected attempts never count
as unknown usage. A coordinator issuing `swarm {"op":"pause"}` from inside its own
turn suspends that turn: the pause receipt is durable on the board but the
calling turn ends without a tool result. Send an explicit prompt to
continue processing retained instructions. Paused queued instructions remain
retained without an inference attempt. Terminal completion notices do not start
automatic report turns. Raw exports contain retained wire message records, run
with at most two concurrent exports, and release the multi-client reader so
supervisor controls remain available during spill reads.
