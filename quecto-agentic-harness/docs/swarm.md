# Swarm coordination

`swarm` is a compiled native tool: members coordinate through structured board
ops (`{"op":"claim","task_id":3}`), each served in-process by the Rust board
over the run's SQLite file (ADR-0030). Members use the `bash` tool for Git,
builds, tests and any other computation.

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
cannot, and neither can the host. Use an image with the current harness. See [container configuration](../../docs/container-runtimes.md)
and [subagent control](subagents.md). Agents can load `docs {"name":"swarm"}`
without the documentation files being present in the container checkout.

## Inspection and results for users and master agents

| Need | Current interface |
|---|---|
| Agent/container status | Existing TUI agent views or `agent_cmd` inventory/state commands |
| Goal, criteria, task progress, blockers and evidence | Ask the coordinator to call `swarm {"op":"summary"}` and report the result |
| Detailed live task/file pages | Coordinator uses `swarm {"op":"tasks"}` / `swarm {"op":"file_owners"}` (`offset`, `limit`) while the run is running |
| Coordinator report | Master reads `agent_cmd.get_messages` using the coordinator's returned agent UUID |
| Final result | A run that ended is `paused` holding `summary.outcome` (`succeeded`, `blocked`, `failed`, `budget-exhausted`) and `summary.outcome_reason`; read the final revision and criterion evidence, then resume or close it with `swarm_control` |
| Evidence files | Ask the coordinator to export them using ordinary file/Bash tools before environment teardown |

There are no dedicated public swarm creation, update, inspection or result UDS
endpoints, and no swarm dashboard/config panel in this change. Existing UDS agent
supervision carries prompts and reports; it does not expose the durable board as
a structured event feed. That public interface is follow-on work.

On a wake hint, inspect summary first. Once the run is no longer running,
`op=summary`, `op=events` and `op=usage` remain readable but every board op is
refused, `inbox` and `ack` included: do not request inbox reads or
acknowledgments then. Keep the coordinator available for final reporting and
export. Record the tested commit and binary build identity with important
reports. Container deletion can remove evidence; board persistence is not an
export.

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

Call `swarm` with `op=create`, a nonempty `goal`, an optional list of
`constraints` (omitted or `null` means none; any other value must be a list of strings),
`criteria`, `member_limit` (1 through 25), and either `deadline_in_seconds`
(seconds from now, 1 to 604800; wins when both are given) or `deadline` (Unix
seconds, in the future and no more than seven days away). This differs from
`swarm_control extend`, whose field is `deadline_seconds`. Each criterion has an `id`, a
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
Nested container launches are rejected: no agent inside an isolated-PID
(swarm-capable) container, swarm member or not, may start another container;
it spawns local sub-agents. An existing run cannot be reset through
the board ops. This bounds harness-managed agents, not arbitrary subprocesses
or direct provider API calls. Provider inference admission is separate (#1679).

Launch reservations precede process launch. Only reservations for processes that
never started are released automatically. Launched processes are identified by
PID and kernel start time, avoiding recycled-PID mistakes. `op=reconcile` detects
harness death, but a dead harness does **not** prove that its Bash execution
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
`recover`, and the run keeps running. What happens to its file
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
`revoke` (`task_id`, `reason`) frees them and records why, or
`recover` with `"release_files":true` frees them explicitly; a plain `recover`
refuses while they are retained. Only a harness death nobody observed this way
(a socket loss, a vanished pid of a member this harness did not launch) is the
conservative quarantine above; a socket loss alone never confirms a death.
Neither idle time nor a worker's completion message frees a slot.

## Board ops

The board is Rust, in the harness process (ADR-0030): its policy is
`src/domain/swarm/`, its use cases `src/application/swarm/use_cases/`, and its
store the rusqlite adapter in `src/infrastructure/persistence/swarm_board/`,
over the same SQLite file, schema, pragmas, rollback journal and
`BEGIN IMMEDIATE` transactions the Python board used, so boards it created
keep working. No interpreter or worker process serves it: each call opens a
connection, runs its transactions and answers.

Members reach it through the `swarm` tool's structured ops. Each op is the
former Python `board.<method>` of the same name, with the same arguments (as
named JSON fields, with the same defaults) and the same JSON result: only the
calling syntax changed, `{"op":"claim","task_id":3}`. `swarm_board_ops.rs`
(`BOARD_OPS`) is the one table of them; the tool schema and the dispatcher's
argument binding are rendered from and held to it.

| Op | Fields (JSON types; defaults) | Result | Who |
|---|---|---|---|
| `task` | `task_id` integer | task, with owner liveness | any member |
| `tasks` | `offset` integer (0), `limit` integer 1–100 (50) | list of tasks | any member |
| `file_owners` | `offset` integer (0), `limit` integer 1–100 (50) | list of file reservations | any member |
| `task_create` | `request` string, `title` string, `acceptance` nonempty list of nonblank strings, `dependencies` list of task ids or null (`null`) | task (replayed on a retry) | any member |
| `dependencies` | `task_id` integer, `dependencies` list of task ids or null | `null` | any member |
| `claim` | `task_id` integer | task with its claim `token` | any member |
| `release` | `task_id` integer, `token` string | `null` | the owner |
| `block` | `task_id` integer, `token` string, `reason` string | `null` | the owner |
| `unblock` | `task_id` integer, `token` string, `reason` string | `null` | the owner |
| `submit` | `task_id` integer, `token` string, `evidence` list of `{artifact, revision}` | `null` | the owner |
| `reserve` | `task_id` integer, `token` string, `paths` list of strings | `{token, paths}` | the owner |
| `release_files` | `task_id` integer, `token` string, `reservation` string | `null` | the owner |
| `send` | `request` string, `recipient` member id, `body` string, `revision` string or null (`null`), `supersedes` message id or null (`null`) | `{id, status}` | any member |
| `withdraw` | `message_id` integer | `null` | the sender |
| `inbox` | `include_consumed` boolean (`false`; bound loosely, so any nonzero number such as `1` also reads consumed history) | list of messages | any member |
| `ack` | `message_id` integer | `null` | the recipient |
| `evidence` | `criterion` string, `artifact` string, `revision` string or null, `kind` `command` or `review`, `passed` boolean | `null` | any member (a worker's is a proposal) |
| `amend` | `goal` string, `constraints` list of strings, `criteria` list of `{id, kind, description}`, `reason` string | `null` | coordinator |
| `verify_task` | `task_id` integer, `token` string, `revision` string or null | `null` | coordinator |
| `revalidate_task` | `task_id` integer, `revision` string or null, `evidence` list of `{artifact, revision}` | `null` | coordinator |
| `recover` | `task_id` integer, `release_files` boolean (`false`) | `null` | coordinator |
| `revoke` | `task_id` integer, `reason` string | task | coordinator |
| `complete` | `revision` string or null | `null` | coordinator |
| `stop` | `status` string, `reason` string | control receipt | coordinator |
| `usage_report` | none | usage report (as `op=usage`) | any member |

Where a board method already had a harness op, the harness op is it:
`summary` (`since`), `events` (`after`, `limit`), `pause` (`reason`),
`resume` (refused for every member), `usage_budget` (`token_limit`,
`strict_unknown`) and `usage`. `create` stays the harness op (it creates,
activates and supervises the run, and takes `deadline_in_seconds`); the
board's own `create` is not exposed. `reconcile` and `cancel_run` are
harness ops too.

How an op is served:

1. **Member input.** The call's text is read as Python's `json.loads` reads
   it (`-0` is the integer 0, a repeated key keeps its last value). A value
   the board's JSON value cannot hold exactly is refused before the board,
   never coerced: an integer outside i64 and u64 (`18446744073709551616`), a
   number that overflows (`1e400`) and a string holding a lone surrogate
   (`"\ud800"`). The refusal is
   `tool error: swarm: "arguments: not representable as a serde_json value: <why>"`, and
   its `swarm_op` record has kind `invalid` and names the field in
   `unreadable_args` (see [Telemetry](#telemetry)). These are inputs Python took (or
   read) but the board refuses; the differential suite lists them as
   permitted divergences. `NaN`, `Infinity` and `-Infinity` never reach the
   tool: they are not JSON, so the agent loop answers the call itself with
   `the arguments for tool 'swarm' were not a JSON object, so it was not run
   ... Received: <the text>`, and no `swarm_op` is recorded.
2. **Binding.** Fields bind by name, as the Python signature binds them: a
   missing required field (`claim: missing required argument task_id`) and a
   field the op does not take (`claim: unexpected argument id`) are refused
   as calling errors (kind `calling`). Loosely typed values the Python board
   accepted, such as a string task id SQLite's affinity matches, behave as
   they did: send task ids as `3`, not `true` or `3.0` (these bind loosely
   as they always did: `true` reads task 1, `3.0` reads task 3). `passed`
   and `release_files` count only `true` as true; `include_consumed` binds
   loosely, so any nonzero number (such as `1`) also returns consumed
   history.
3. **The running gate.** A board op is refused unless the run is `running`
   and within its deadline (owner decision, 2026-09-28: the gate the Python
   `run` op had; a running run whose deadline is missing or not a number is
   refused as past it), with guidance naming what is allowed now: `summary`,
   `events` and `usage (op=usage; the board op usage_report needs a running
   run)`. This covers reads, `inbox`, `ack` and `withdraw` too. The gate
   also refuses with kind `store` and gate `unreadable` in two ways: when
   the `_status` call or a mutating op's first `_event_cursor` call fails,
   the refusal is the store's text; when the status reads but is not text,
   it is `the swarm run status could not be read; call op=summary.`
4. **The call and its lifecycle.** A mutating op reads the board's event
   cursor before and after its call. When the cursor moved, the harness sends
   the wake hints, or settles the run when it no longer runs. A hint that
   failed rides the answer as `notification_warnings`, and a failure of this
   step as `coordination_error` (beside a non-object answer as `result`).
   When the second cursor read fails, that failure is the `coordination_error`
   and no lifecycle runs. When the op is refused but its call moved the
   cursor, or the second cursor read failed, these notes follow the refusal
   text on the next line, as one JSON object. The
   read-only ops (`task`, `tasks`, `file_owners`, `inbox`, `usage_report`)
   skip this step.
5. **The answer** is written as Python's `json.dumps` writes it (insertion
   order, `", "` and `": "` separators, `ensure_ascii`, float `repr`). A board
   refusal keeps the board's exact text: the member sees
   `tool error: swarm: "<message>"`, the same prefix a member-input refusal
   gets.

The board lives in the checkout's own git directory: `.git/quecto/swarm.sqlite`,
or a linked worktree's git directory. Git's work-tree commands never touch it
(`stash -u`, `clean -fdx`, `checkout`, `merge`, `reset --hard`), and a board an
agent once committed at the old path is never taken for this run's. Only a
checkout with no git directory keeps it at `.quecto/swarm.sqlite`, out of reach
of git too. The location follows from the checkout's layout, and once a process
has found its board it keeps that path: a layout changed mid-run (`git init`, a
git directory created or removed) never moves a live board. A board deleted
from under a run cannot be recreated: every call then fails with
`coordination store missing at <path>`, and a member joining later is refused.

A worked sequence, one call per line:

```json
{"op":"summary"}
{"op":"task_create","request":"implement-v1","title":"Implement the behavior","acceptance":["Acceptance tests pass"],"dependencies":[]}
{"op":"claim","task_id":1}
{"op":"reserve","task_id":1,"token":"<claim token>","paths":["src/feature.rs","tests/feature.rs"]}
{"op":"submit","task_id":1,"token":"<claim token>","evidence":[{"artifact":"evidence/acceptance.log","revision":"the-exact-commit-or-artifact-digest"}]}
```

Between `reserve` and `submit`, implement and test with the existing tools,
and store large output as artifacts.

`task` reads a task. `tasks` and `file_owners` page the board (`offset`,
`limit`; maximum page size 100). `dependencies` edits a task's dependencies
before it is claimed; missing, self and cyclic dependencies fail. Ready tasks
are claimed atomically. Unmet dependencies and an explicit `block` remain
visible as blockers. `release` returns owned work and files to the board. A fresh claim has a new token; stale tokens cannot submit, release or
complete reassigned work. `task_create` and `send` use request IDs for retry
idempotency; reusing an ID with a different payload is an error. Identical
submission and verification retries do not repeat their transitions.

File sets are reserved all at once or not at all. Paths resolve inside the shared
checkout, including symlink aliases and not-yet-created files.
`release_files` (`task_id`, the claim `token`, the `reservation` token)
cannot remove a newer owner's reservation. Only the coordinator may `recover`
a task, once the owner's death is confirmed by the harness that launched it
(above), and only the coordinator may `revoke` one (`task_id`, `reason`): it takes a claim back from an
owner that will not finish, alive or not, on a claimed, blocked or submitted
task. The task returns to `ready` with no owner, token, blocker or evidence,
its file reservations go, a `revoked` event records the reason and previous
owner, and the previous owner receives a board message so a member that wakes
later learns its claim is gone (its next owned call fails with `stale or
unowned claim`). Revoking an unowned task is a no-op returning the task. There
are no expiring ownership leases.

A claimed, blocked or submitted task row (`task`, `tasks`, the summary's
first 50 tasks) also says how its owner is doing, read side only (#1969).
`owner_last_activity` is the number of seconds since the owner's most recent
board event by the store clock. `owner_state` is the store's affirmative view
of that member, authoritative signals first: `dead` when the harness that
launched it confirmed its exit; `lost` when its harness loss was recorded
(`scope_unknown` after its latest activation, #1924/#1961); `reserved` when it
was admitted but never launched; `active` or `idle` only for a live launched
member, `idle` meaning it has written no board event for 300 seconds; and
`unknown` for anything else, including an owner id that is not text (only a board edited from outside holds one). An `active` or `idle` owner is named as a `send`
recipient in `contact` (`{"op":"send","request":...,"recipient":"<owner id>","body":...}`); for every
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
the clock alone, with no event to move the cursor, `{"op":"summary","since":<cursor>}`
returns a full summary rather than `unchanged` while an owner is idle who was
not at the time of the cursor's event, and both responses carry
`next_liveness_check_at`, the store-clock instant the earliest active owner
would turn idle (null when none would), so a caller knows when to look again.

These are **cooperative reservations**, not mandatory locks: Bash can bypass
them. The container is the external containment boundary, not a security
boundary between same-user workers. Do not edit SQLite or bypass file
ownership. Only the designated integrator changes branches, commits, or integrates
changes in the shared checkout. Concurrent Git merge automation is not provided.

## Messages and waking

Use stable member IDs from the summary's `members`:

```json
{"op":"send","request":"schema-question-v1","recipient":"<member id>","body":"Blocked: which schema version should I use?"}
{"op":"inbox"}
{"op":"ack","message_id":7}
```

Read each message the inbox answers, then acknowledge it by its `id`.

Any member may message any other member directly; nothing routes through the
coordinator. A question about a task you depend on belongs with that task's
owner, which its row names as `contact`. Messages are `accepted` when durable
and `consumed` when acknowledged. Neither means work completed. `{"op":"inbox","include_consumed":true}` also reads consumed history.
A message may carry the `revision` it is about, and `send` with `supersedes`
(a message id) retires your own earlier unread message to the same recipient
in the same transaction; `withdraw` (`message_id`) retires one without a
replacement. Retired messages
leave the inbox and produce no further wake, but stay in the audit as
`superseded` (with `superseded_by`) or `withdrawn` (#1837). The board itself
lets `withdraw` and `ack`, as bookkeeping, through while the run is paused,
but the tool's running gate refuses every board op then (above), so through
`swarm` they too need a running run. These are vocabulary the members may use; nothing requires them.
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
tasks with `verify_task` (`task_id`, the claim `token`, `revision`), accept
evidence using `evidence` (`criterion`, `artifact`, `revision`, `kind`,
`passed`), and `complete` (`revision`).
A worker's `evidence` call records an unaccepted submission. A coordinator must
actually inspect command results and obtain the required independent/human
review before accepting them; the board does not execute tests or act as an
independent reviewer. `command` and `review` evidence are distinguished.

Completion requires accepted evidence for every original criterion at the
specified revision, completed tasks with matching evidence revisions, and no
remaining file reservations. When later dependent work advances the checkout,
the coordinator reruns the earlier task's checks, then calls
`revalidate_task` with the final `revision` and fresh `evidence`. This requires a completed
task and nonempty artifact evidence matching that revision; it records both old
and new evidence in the audit. Workers cannot revalidate. Existing evidence is
never silently relabeled. Submitted tasks, idle agents and an empty queue do
not prove success. `amend` (`goal`, `constraints`, `criteria`, `reason`) is coordinator-only,
records the reason and complete before/after goal, constraints and criteria, and
invalidates prior overall evidence. The creation event preserves the original
contract. Existing audit events from older versions are not retroactively reconstructed.

`op=summary` reports goal, status, membership usage/limit, task counts and the first 50 task/file details,
blockers and evidence. History is opt-in through `op=events` (`after`, `limit` 1–100); `since=event_cursor` suppresses unchanged summary payloads. Total counts include
entries beyond the first page. File reservations are bounded to 1000 per run. The full
audit remains in SQLite. Every end of a run is a resumable pause that only the
supervisor outside the swarm lifts (#1729): `stop` with `status`
`blocked`, `failed` or `budget-exhausted`, `complete` (`succeeded`),
an observed-token budget and the wall-clock deadline all move the run to
`paused` holding that outcome and reason (`summary.outcome`,
`summary.outcome_reason`, the control receipt's `outcome`). Nothing is killed:
members stay live with their claims, reservations, inboxes and evidence; their
execution and inference suspend as for any pause, while the coordinator keeps
reporting (native `summary`, `events`, `usage`). The supervisor then either resumes the same run (`swarm_control
resume`, which extends the deadline by the paused time, clears the outcome and
wakes every member) or closes it (`swarm_control close`), which makes the held
outcome terminal and settles: each member is ended by the harness that launched it (#2121), which records the
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
once the run holds its outcome, every board op is refused and only the native
reads (`summary`, `events`, `usage`) answer unless the supervisor resumes the
run (it closes for good only on close or cancel).
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
even when agents are idle. There is no turn cap. An optional observed-token budget can durably pause admission; it does not cancel already billed usage or guarantee a provider-side spending cap.
The board holds no SQLite transaction across model/tool execution or lifecycle
notifications. SQLite uses short immediate transactions and a bounded contention
timeout, on a suitable **local filesystem** only. Corrupt, missing or locked state
fails explicitly; it never creates a replacement board or bypasses admission.

Container destruction remains destructive unless the user preserves its
storage; cross-container recovery is not provided.

## Architecture boundaries

The board is one bounded context in Rust (ADR-0030), with dependencies pointing
inward:

- **Domain** (`src/domain/swarm/`): the value records, `BoardError` with its
  stable `RefusalKind`, and the pure policy (`policy.rs`, `validation.rs`,
  `dependencies.rs`, `notification.rs`, `owner.rs`, `usage.rs`,
  `telemetry.rs`): authorization, deadline, admission, completion,
  revalidation, wake-notification, owner-liveness and usage-budget decisions,
  without I/O. `mod.rs` also holds the membership, process identity and
  outcome vocabulary.
- **Application** (`src/application/swarm/`): the capability's role-segregated
  ports (`ports.rs`: the board repository and its transaction's role ports,
  the id source, checkout paths, and the run-control, process control and
  clock ports), one use case per board method (`use_cases/`), their
  request/response types (`dto/`), the shared `board_*.rs` helpers, and
  reconciliation and settlement sequencing (`mod.rs`).
- **Infrastructure**: the rusqlite store and the SQL implementation of the
  ports (`persistence/swarm_board/`: the verbatim schema, pragmas,
  `BEGIN IMMEDIATE` transactions and the Python-compatible JSON codec), the
  checkout-path adapter (`workspace/checkout_paths.rs`), and the tool
  adapters (`tools/swarm_board_dispatch.rs`, the method-name dispatch;
  `tools/swarm_board_ops.rs`, the structured ops; `tools/swarm_bridge.rs`,
  the harness's own calls). Linux identity checks, UDS commands and timer
  scheduling stay infrastructure too.
- **Composition** (`src/composition/swarm.rs`) is the only place the board's
  use cases and adapters are built; the interface only holds and passes the
  handles.

Pure policy and fake-port tests supplement the real SQLite tests and the
differential suite, which runs every operation sequence against the Python
board and the Rust one and compares results, stored rows and errors until the
Python board is deleted (#2283).

## Agent guidance

`docs {"name":"swarm"}` serves a compiled-in manual from any working directory,
including a container without the product source checkout. It is the members'
API reference: one runnable example per board op, with exactly the op's
fields and values of the schema's types, plus bounds, the member-input rule,
common errors, evidence proposals versus acceptance, and terminal
inspection/export. Task acceptance is a nonempty list of strings; message
bodies are strings. Worker `evidence` calls are proposals (`accepted=0`), not
coordinator acceptance. Members use the Bash tool for Git, checks, computation
and other external commands under its configured policy; they must not raise
or bypass configured limits.

Wake hints are selected from the invoking member's actionable events and coalesced
using a durable per-actor cursor. Reading the board, acknowledging messages and
reservation bookkeeping do not broadcast more work. Message hints target their
recipient; submissions/blockers/evidence target the coordinator; changes that
make work available notify only peers free to take it (#2127). Any task event
that leaves claimable work (created, released, verified, claimed, submitted,
blocked, ...) wakes the members other than its actor that hold no claimed,
blocked or submitted task, plus the coordinator unless the event is a worker's
own claim, submission or block. When none of them but the coordinator is free,
members waiting only for review (parked) are woken as well. A worker is not
told about work that became ready while it held its claim, so the guidance has
it check `op=summary` for ready work after submitting before it yields; a
confirmed member death also re-offers ready work. Otherwise a parked member stays parked until a message reaches
it, the contract is amended, or its task is verified or revoked (it is then
free). When a burst of new tasks outnumbers the free members, the rest are handed
on claim by claim rather than every member being woken at once. Each event is judged from its actor's view, by
the sender and again by the receiver's wake check, so both agree. A member
that cannot reserve a file releases the task and yields rather than holding
its claim: its release hands the task on. The cursor advances atomically before external
notification, so a failed hint is reported but not endlessly retried; the durable
board/inbox remains authoritative. Terminal runs generate no new actionable hints.
Already queued hints instruct the recipient to inspect `op=summary` first and,
once the run no longer runs, not to call `inbox` or `ack` (the running gate
refuses them) while remaining available for parent requests and artifact
export.

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
with `{"op":"block","task_id":<id>,"token":"<claim token>","reason":"<question>"}`,
report the exact question to the master and yield the turn. Do not sleep/poll.
`{"op":"stop","status":"blocked","reason":"..."}` ends
the run: it becomes a pause holding `blocked` that only the master can resume or
close, so use it when the whole run cannot proceed without the master, not to wait
for one task. A blocked **task** retains its claim; use `unblock` (`task_id`, `token`, `reason`) after the answer arrives. For a whole-run wait the coordinator may `pause`; only the master resumes (the deadline is extended by the paused interval).

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
`journalctl --user CONTAINER_NAME=quecto-env-<id>`, where `quecto-env-<id>`
is the `metadata.container` of the environment's `get_containers` row (add `-f` to follow, `--since` to
scope); under Docker use `docker logs quecto-env-<id>`. An environment that vanished also leaves a
`kill.log` entry in the adapter's state root naming the operation that removed
it. See [Container runtimes](../../docs/container-runtimes.md#the-official-dockerpodman-adapter).

The compiled [agent manual](docs-tool-embeds/swarm.md#durable-supervisor-controls-and-reports)
contains supervisor command examples, receipt semantics, raw export and budget
configuration. `swarm_control` pause/resume/close/extend/status/usage_budget
bypass the model queue and route through ancestors to the addressed member. Swarm creation and a
general dashboard event API remain separate follow-on work.

A member's own context is kept lean (#2342): once its process takes part in
a swarm its pruning budget is capped at `agents.defaults.swarm_max_context_tokens`
(default 48000, or `QUECTO_SWARM_MAX_CONTEXT_TOKENS`; it never disengages,
and setting it at or above `max_context_tokens` switches it off), and each
full `summary` answer supersedes the member's older
ones, which collapse to recall stubs while the newest stays in full. Each prune
that does either is visible in the event log's `context_pruned` record
(`snapshots_superseded`, `ceiling_tokens`); the moment the cap engages is a
`quecto::swarm_board` tracing event. See
[Sessions](sessions.md#context-management) for the dials.

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
it off a failing provider; `{"op":"revoke","task_id":<id>,"reason":"..."}` reassigns its work
so another member can claim it. `recover` applies once the member's
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

## Telemetry

The Rust board (#2265) records one `swarm_op` in the event log for every board
op it serves (#2303), only while the event log is on
(`"telemetry": {"event_log": {"enabled": true}}`, off by default; swarm members
get no default of their own). With it off nothing is measured or written: the
store takes no timings and keeps SQLite's own busy timeout. The board file
itself is unchanged: its `events` table stays the audit history it always was,
and telemetry lives only in the event log. A record is appended on the board
call's own thread, synchronously, under the log's write gate, so it never
interleaves with another record; a record the log cannot take (a full log, a
failed write) is dropped with one warning, and never changes the op's answer.
The first record that does not fit, from whichever writer, is replaced by the
log's one `log_capped` record, and nothing is written after it.

Telemetry never holds up the board: a record waits for the write gate at most
50 ms. Past that (another writer is stuck), the record is dropped and counted,
and the next record written is preceded, in the same write, by one
`{"event":"swarm_ops_dropped","dropped":N}` line giving how many were dropped
since the last. Drops never followed by a written record (the log caps, or the
session ends first) go unnoted. The write itself runs on the calling thread and
is not bounded: a filesystem that never returns from a write holds the one
call that is writing, while every other call gives up at the gate.
A record carries ids, kinds, durations and sizes only, never a task title,
body, evidence, reason, path or other board text:

| Field | Meaning |
|---|---|
| `turn` | Always `null`: a board op is not filed under an agent turn (the board is called without one) |
| `op` | The board method (`unknown` for a name that is none) |
| `actor_ref` | The caller's member id: the id the caller chose for itself on the board (`members.id`), redacted if it looks like a credential, and cut to its first 128 characters (a refused call can name any id) |
| `role` | `host` for the harness's own ops, including the `summary` the harness reads for itself to judge a run after an op or when it settles one; for a member's own `summary` or `reconcile`, and for a member-facing op (`task_create`, `claim`, ...), when the board answered it as a member's, or refused it only after the operation gate authorised the caller as a member (a stale claim token, a task in the wrong state, a refusal after the op's writes committed), the caller's role in the run as the op read it: `coordinator`, `integrator`, or `worker` for any other member; `null` for a refusal before or by the gate itself (`not_member`, `not_coordinator`, `run_missing`, or a run that permits no new work), or when the op read no run (the board holds none). A stranger is never recorded as `worker`. A member whose death was confirmed still holds its role in the run: the gate admits it to read-only ops, so its reads (`summary`, `events`, ...) record `worker` (or its other role), and only its mutations, which the gate refuses as `not_member`, record `null` (#2313) |
| `run_id` | The run the op found, when it found one and its id is one the board generates (32 lowercase hex digits); `null` otherwise, so an id edited into the board from outside is never recorded |
| `task_id`, `message_id` | The task or message the op acted on, by the id its row holds (a task id given as `"2"` is task 2); left out when it acted on none, or its row's id is not an integer |
| `outcome` | `ok` or `refused` |
| `kind` | For a refusal, its stable kind: `run_missing`, `not_coordinator`, `not_member`, `not_running`, `budget_exhausted` (the deadline has passed, or the run is paused or ended as `budget-exhausted`), `member_limit`, `identity_taken`, `run_exists`, `completion_unmet`, `stale_revision`, `immutable`, `wrong_state`, `not_owner`, `stale_token`, `reserved_by_other`, `dependency_cycle`, `not_found` (a task, message or recipient the board does not hold), `capacity_full` (a bounded board table is full: tasks, file reservations, an inbox or a request ledger), `supervisor_only` (resume, close or extend, which only the supervisor takes), `launch_conflict` (a launch whose process identity conflicts with the member's), `request_id_reused` (a request id reused with different data), `invalid`, `calling` (no such method, or arguments that do not bind), `contended` (the database stayed busy or locked past 500 ms: the write lock at `BEGIN`, a reader holding off the commit, or `SQLITE_LOCKED`), `store_missing`, `store` (any other store failure) or `internal` |
| `committed` | Written `true` only for a refusal that came after the op's writes committed: `create` commits the run, then its summary can refuse; `_bootstrap` commits the placeholder (when it writes one), then its join and summary can refuse; `_join` commits an admission, then its activation can refuse, and an admission or activation, then its summary can refuse. Its `decision` (the branch taken so far) and detail (`_bootstrap`'s `placeholder_created`) are recorded too. Left out for any other refusal |
| `duration_us` | The whole op, in microseconds |
| `lock_wait_us` | From `BEGIN IMMEDIATE` issued to acquired (or given up), summed over the op's transactions; `null` when the op began no transaction, so nothing was measured. **A read's is about 0, and its waits are in `busy_wait_us`:** `_event_cursor`, `_watch`, and a `_snapshot` whose gate has nothing to write, begin a read transaction (`BEGIN DEFERRED`, #2338), which takes no lock at `BEGIN`, so their lock wait is only that statement's own. A dashboard of lock waits should read these ops' `busy_wait_us` (and `busy`) instead |
| `busy_wait_us` | The time the store's busy handler slept for the op, whichever statement found the database busy (`BEGIN`, a read or the commit); `null` when nothing was measured |
| `busy` | Whether the busy handler fired at all: another connection held a lock the op needed (the write lock, or, at commit, a reader); `null` when nothing was measured |
| `commit_us` | The time the op's `COMMIT`s took, summed over its transactions, a failed one included. The whole `COMMIT` is timed, so when a reader holds off the commit the busy handler's sleep is in it too (and in `busy_wait_us`). For an op that wrote, the rollback journal's and the database's `fsync`s, which are most of a writing op's time on a disk-backed board (#2340: about 2.4 ms each on an idle btrfs disk, tens to hundreds of milliseconds while other processes write to the same disk); `0`, not `null`, for an op whose transactions all ended before a `COMMIT` (a refusal rolls back); `null` when nothing was measured (the op began no transaction). Absent from records written before #2340 |
| `cursor_moved` | Whether the op moved the caller's cursor: its notification cursor for `_notifications`, its wake cursor for `_accept_wake`; for a `summary` given a cursor, whether the board's cursor is no longer it (`false` for the `unchanged` answer); `null` for an op that has no cursor to move, and for a refusal |
| `result_bytes` | The size of the JSON the op answered, as its compact serialization (`serde_json`'s, which is not the size of Python's `json.dumps` text with its spaced separators); 0 for a refusal |
| `decision` | What an answered op decided, as a snake_case kind the board names (`recorded`, `grace_pending`, `already_dead`, `coordinator_confirmed`, `lost`, ...); left out for a refusal, but for a `committed` one. `_request_admission`'s names the gate that read it (#2339): `model_gate` once per model request, before its first send (each request also records one `_record_request`, which may add `redelivered` records of its own), `retry_gate` before each reattempt of one (a retry, a stream re-initiation, a resend after an OAuth refresh), `tool_gate` before each tool call; a read at any gate in which the token budget warns or pauses the run records `warned` or `paused` in place of the gate. A member so reads its admission about `1 + t` times per request, `t` being the request's tool calls |
| `run_status` | For the loss and death ops (`_quarantine`, `_confirmed_dead`, `_lose_coordinator`), the run's status as the op found it, before it wrote anything: `setup`, `running`, `paused`, `succeeded`, `blocked`, `failed`, `cancelled`, `budget_exhausted`, or `unknown` for a NULL or edited status (whose text is never recorded); left out for other ops |
| `exit` | For `_confirmed_dead`, the exit kind it was given: `orderly` or `abrupt` |
| `reservations_retained` | For a confirmed death, the file reservations it left held: an abrupt exit's, or 0 once an orderly exit released them |
| `owners_scanned` | For `summary` and `tasks`, the task owners whose liveness the op read, one per owned task (a summary's liveness watch and its page each count theirs) |
| `page_size` | For `summary` (its tasks; left out of the `unchanged` answer), `tasks` and `events`, how many rows the page holds |
| `has_more` | For `events`, whether a later page holds more |
| `fast_path_defeated` | For a `summary` given the board's own cursor, whether an owner turned idle by the clock alone since it, which defeats the `unchanged` answer |
| `placeholder_created` | For `_bootstrap`, whether it wrote the container's placeholder run; its `decision` is the join's branch (`admitted`, `already_live`, `reactivated`), as `_join`'s is, or `placeholder` for a `committed` refusal met before the join took a branch; `create`'s is `fresh` or `over_setup` |
| `missing_args` | For a `calling` or `invalid` refusal (#2341), the required arguments the call left out, by their schema names, every one (not only the first the refusal's text names); a parameter no `swarm` op's schema has (a harness-internal method's) is left out. Left out when there are none |
| `unexpected_args` | For a `calling` or `invalid` refusal, `{"count": N, "known": [...]}`: how many keys (or positional values) the op's signature has no parameter for, and, in `known`, those that are a schema field of some other op (a field sent to the wrong op). Any other key is counted only, never named, so no text a member typed as a key reaches the log. `known` is left out when empty, the field when there are none |
| `wrong_type_args` | For a `calling` or `invalid` refusal of a member-facing op, `[{"arg": ..., "expected": ...}]`: each argument given a value of another JSON type than its schema's, and that type (`integer`, `string`, `boolean`, `null_or_string`, `null_or_integer`, `array_of_string`, `array_of_object`, `null_or_array_of_integer`; `null` first when it is allowed). An array whose items are of another type counts; an item's own properties are not checked (an `evidence` item need only be an object). It compares with the schema, not with the board's own checks, and is kept for any `invalid` refusal, the board's own validation included, so it can name a field that is not the refusal's cause: `task_create` with an empty `acceptance` list and a numeric `request` is refused for the list, and records `request` (which the board binds untyped, as Python does). Left out when there are none |
| `unreadable_args` | For member input refused before the board as `invalid` (#2341): the schema field whose value the board's JSON value cannot hold (`1e400`, an integer beyond i64 and u64, a string holding a lone surrogate, or nesting more than 128 levels deep, counting the object itself as one level), or `arguments` for the text as a whole. `arguments` covers: text the tool cannot read as JSON (the `unknown` op's record: text that is no JSON, and JSON the serde reader refuses where no structured op was read, such as a harness op or a call with no op holding `1e400`); such a value under a key no schema field names; and, for the harness's own calls only, arguments that are no array or object (a member's JSON array or scalar names no op, so it is refused as op `unknown` with kind `calling` and no argument fields). Left out otherwise |
| `polls` | For the run watch's `_watch` aggregate (#2338, decision `unchanged`), the unchanged ticks it accounts for; left out of every record of one call (see [The run watch](#the-run-watch-2338)) |
| `ended_by_loss` | For a recorded loss (`_quarantine`'s `recorded`, `_lose_coordinator`) or a confirmed death, whether the op ended the run by loss; left out when the op recorded neither |

Every name in `missing_args`, `unexpected_args.known`, `wrong_type_args` and
`unreadable_args` is a field name of the `swarm` tool's schema (the board ops'
table, `BOARD_OPS`), or `arguments`. This holds by type: the dispatcher records
only a `SchemaField`, the table's own `&'static str`, which nothing but the
table lookup (`schema_field`) can construct, so no member text (nor any other
string) can become a recorded name, in a release build too; debug builds also
assert it on every record. Each binding refusal also leaves one
`swarm board call arguments` `tracing` record (INFO, target
`quecto::swarm_board`) with the op, the caller's redacted `member` ref (as the
call's own `swarm board call` record has it) and the same names comma-joined
(`missing_args=token,reason`, `unexpected_args=2`, `unexpected_known=title`,
`wrong_type_args=acceptance:array_of_string`, `unreadable_args=task_id`), also
while the event log is off. To see what members got wrong in a run:

```bash
jq -c 'select(.event == "swarm_op" and (.kind == "calling" or .kind == "invalid"))
  | {op, kind, missing_args, unexpected_args, wrong_type_args, unreadable_args}' ~/.quecto/audit/*.jsonl
```

Kinds are additive: a later release may add a kind (each refusal the Python
board raises already has one), but never renames or reuses one. A consumer of
the event log must accept a `kind` it does not know, rather than reject the
record.

Because a kind can never be renamed, the borderline refusals were assigned
deliberately:

- "run not created yet; nothing to cancel" is `run_missing`, as "coordination
  run missing" is: the board holds no run row at all, which is not a run in the
  wrong state (`not_running`).
- "only a message to the same recipient can be {status}" is `wrong_state`:
  the message named exists and is the caller's own, but is addressed to another
  recipient, so it is in the wrong state for the op; the argument itself is
  well formed (`invalid` is kept for malformed arguments).
- "paused run has no pause record" is `internal`: the board writes a pause
  record whenever it pauses a run, so a paused run without one is a broken
  board invariant, not a store failure (`store` is kept for SQLite's own
  failures).

A zero is always a measure, never a stand-in for "not measured". The waits are
summed over the op's own transactions and only those: each op is measured on a
repository built for it alone, never through state shared with another op.

Every call also leaves a `tracing` record on target `quecto::swarm_board`
(DEBUG for a read, including the wake claims `_notifications` and
`_accept_wake`, which run as reads after every board change, unless the token
budget warned or paused the run in it; INFO otherwise), at WARN for a
`contended` refusal or a busy wait over 250 ms; `RUST_LOG=quecto::swarm_board=debug` shows them live.
Because the owner decided the event log is off by default and nothing is
measured while it is off (decision T1), the tracing record's waits are
measured, and the slow-lock WARN can fire, only while the event log is on;
the `contended` WARN fires either way.

### The run watch (#2338)

Every member's harness watches its run on a thread of its own, so it
suspends its inference on a pause and settles the run's end even when no
turn is running. The watch ticks every 500 ms, and each tick is one board
call, `_watch` (Rust-only, recorded with role `host`), in one read
transaction that takes no write lock (ADR-0030). The watch passes the event
cursor of its last snapshot (`since`); the board answers
`{"event_cursor", "unchanged": true}` (decision `unchanged`) while its cursor
is still that one, and `{"event_cursor", "snapshot"}` (decision `snapshot`,
the snapshot as `_snapshot` answers it) otherwise, the two read together.
Every change the watch acts on (a pause, a resume, the run's end, a deadline
extension) writes an event, so the tick after it answers the snapshot, as
soon as when the watch took a snapshot every tick. The watch passes no
cursor, so the board answers the snapshot whatever its cursor, when:

- no snapshot was taken yet, or the last tick could not be read;
- the run is running and its deadline has come (the gate records the
  expiry, which no event announces, through its two IMMEDIATE
  transactions; the tick then answers the paused run);
- a refresh is due: 5 s after a change, doubling while nothing changes, at
  most 60 s apart.

An idle member so reads about one snapshot a minute instead of 120, and a
tick after a change is one call and one record. `_watch` answers and
refuses through `_snapshot`'s gate. Both read the board in their read
transaction only when it holds every column a write transaction adds; a
board an older writer created, lacking one (`members.launcher`,
`messages.superseded_by`, ...), is read through the IMMEDIATE transactions,
which add it first, so the answer and the board are Python's.

The watch's `unchanged` ticks are not one record each: consecutive ones of
one run are folded into one `_watch` record with decision `unchanged` and
`polls` counting them. Its `duration_us`, `lock_wait_us`, `busy_wait_us`,
`commit_us` and `result_bytes` are the slowest tick's, and `cursor_moved` is `false`. A tick
that answered the snapshot (`cursor_moved` `true` when it was given a
cursor), one the busy handler slowed and a refused one are each recorded
alone, as every other call is. An aggregate is written before the watch's
next record of any other kind, once it has been open 60 s, when the watch
ends, before the run's summary is taken (no later tick is written until the
summary is), when recording stops, when the board's handles for the file are
replaced (a session switch) or dropped, and when the agent command ends,
however it ends: a normal return, the orderly shutdown SIGTERM or SIGINT
starts, or a panic unwinding through it. Only an exit that skips the
command's own return loses what the watch held, at most one aggregate (60 s
of ticks): SIGKILL, the OOM killer, a panic outside a tool call (which the
harness's panic hook aborts on), and the forced exit a second SIGTERM or
SIGINT makes more than 45 s into the shutdown (`process::exit`). The aggregate leaves one `tracing` record (`swarm board
watch polls`, DEBUG, with `polls`) on `quecto::swarm_board`; a tick it holds
leaves none. With the event log off, nothing is aggregated and every tick
leaves its own DEBUG `tracing` record, as every read does.

Count the watch's ticks a log's records account for:

```sh
jq -s '[.[] | select(.event == "swarm_op" and .op == "_watch")
  | (.polls // 1)] | add' ~/.quecto/audit/<session>.jsonl
```

### Switching it on

The event log is off by default and is switched on by configuration only
(owner decision T1): set `"telemetry": {"event_log": {"enabled": true}}` in
the global config (`<base_dir>/config.json`, `~/.quecto/config.json` by
default). The global switch covers every agent on the machine, including
swarm members in containers, which start with their own `--config` but read
the global switch too. A repository overlay (`<checkout>/.quecto/config.json`)
may switch it on, never off. There is no swarm-specific switch and no
swarm-specific default. The switch is read when an agent starts: members
already running keep the setting they started with.

### Structured ops

A structured op a member sends (`{"op":"claim","task_id":3}`) is served
through the same dispatcher, so it leaves the `swarm_op` records above: the
op's own record (with its `task_id` or `message_id` and, for a refusal, its
`kind`), plus one per harness-internal call it made, each with role `host`:
`_status` for the running gate and, for a mutating op, `_event_cursor`
before and after, then whatever the post-call lifecycle ran (`_notifications`,
`_accept_wake`, or the settlement's calls). The `summary` the harness reads
for itself, the post-call lifecycle's and a settlement's closing one, is a
harness-internal call too: it is made as the harness (`CallOrigin::Harness`,
through `call_as`), so it is recorded with role `host` alongside the others
and never counted against the member. A member's own `op=summary` and
`op=reconcile` keep the member's role. An op refused before it reached
the board is still recorded as the op's own `swarm_op`, with no run read
(`role` and `run_id` `null`):

| Refused at | `kind` |
|---|---|
| Member input (a value the board's JSON value cannot hold: `1e400`, an integer beyond i64 and u64, a lone surrogate), its field in `unreadable_args` | `invalid` |
| The running gate, when the run is not `running` | `not_running` |
| The running gate, when the running run's deadline has passed, is missing or is not a number | `budget_exhausted` |
| The running gate, when the `_status` call failed, the status read is not text, or a mutating op's first `_event_cursor` call failed (gate `unreadable`) | `store` |
| No valid op: a name the tool does not serve (an internal board method's such as `_close` included), an op that is missing or not a string, or arguments that are not JSON; recorded as op `unknown`, never under the member's name, and never with role `host` | `calling` (`invalid` for arguments that are not JSON, with `unreadable_args` `["arguments"]`) |

Each structured op also leaves one `tracing` event, `swarm structured op`,
on target `quecto::swarm_board` (DEBUG for the read-only ops `task`, `tasks`,
`file_owners`, `inbox` and `usage_report`; INFO for the others), whether or
not the event log is on. It carries no argument text:

| Field | Meaning |
|---|---|
| `op` | The structured op |
| `gate` | `open` (the op reached the board), `arguments` (member input refused), `not_running` or `deadline` (the running gate refused it), or `unreadable` (the run's status, or a mutating op's first event cursor, could not be read) |
| `outcome` | `ok`, `refused` (the member got an error answer) or `failed` (the tool itself failed) |
| `cursor_moved` | For a mutating op that reached the board, whether the event cursor moved (`Some(true)` or `Some(false)`); `None` otherwise, including when the second cursor read failed |
| `lifecycle` | What the post-call step did: `none` (not run), `unchanged`, `notified`, `settled` or `failed` |
| `warnings` | How many wake hints could not be delivered |
| `result_bytes` | The size of the answer text the member received |
| `duration_us` | The whole op, in microseconds |

### Finding a run's records

Every agent writes its own event log, `<base_dir>/audit/<session>.jsonl`, one
JSON record per line, except an ephemeral agent (`--no-session`, `-s -`),
which writes none. The file is named after the sanitised session key, not
the `session` value verbatim: a key of ASCII letters, ASCII digits, `:`, `_`, `-` and `.`
has each `:` replaced by `_` (`cli:ops` → `cli_ops.jsonl`); any other key, or
one starting with `.`, is hex-encoded with a `key_` prefix. A container member's `~/.quecto` is the host's
(identity-mounted), so the logs of every member of a swarm land in the host's
`~/.quecto/audit/`, one file per member session. Each record carries the
agent's `session`, its process id and its `host`: for a container member the
container's name (`quecto-<environment>` with the bundled container scripts),
which tells one swarm's members from another's.

The board's run id is the full summary's `id` (`swarm {"op":"summary"}`, or
the `run` table's `id`). Select a run's board ops by it:

```sh
jq -c 'select(.event == "swarm_op" and .run_id == "<run id>")' ~/.quecto/audit/*.jsonl
```

Records whose op read no run (`"run_id": null`: an op refused at the gate, or
before the board held a run) belong to the run of the container named in
their `host`, at their time. Each line is written compactly
(`"event":"swarm_op"`), so a plain `grep '"event":"swarm_op"'` finds them
too. `swarm_ops_dropped` lines in the same file count records dropped at the
write gate, and `swarm_run_summary_dropped` lines the run summaries dropped. Board text never appears in these records: to see what a task or
message said, read the board itself (`op=events`, or the SQLite file read-only).

A command that reads them back as a report (`quecto swarm report`, #2305) is
planned and not yet available.

### Run summary and the session's log (#2313)

When a run settles (it ended, or was cancelled), the coordinator's harness
writes one `swarm_run_summary` record to its event log, once per run, and only
while the event log is on. It holds two scopes.

The process's own counts (`scope: "process"`). Each member is a process with a
board of its own, so the coordinator's harness sees only its own board calls: a
worker's `claim` or `send` is in the worker's log, not here. These fields fold
the `swarm_op` records this harness wrote for the run, so they equal those
records:

| Field | Meaning |
|---|---|
| `run_id` | The run, as the board generated its id |
| `scope` | Always `process`: the fields below, up to `run`, are this harness's own calls |
| `records` | The run's `swarm_op` records this process folded |
| `calls` | The board calls those records account for: one per record, but a run-watch aggregate (`polls`, #2338) accounts for each tick it holds. Each op's `ok` and `refused` counts are calls, so `_watch`'s `ok` is every tick the watch made, recorded alone or aggregated |
| `ops` | Per op: `ok` (answered), `refused` (by `kind`, left out when none), the nearest-rank `p50` and `p95` and the exact `max` of `duration_us`, `lock_wait_us`, `busy_wait_us` and `commit_us` (#2340: the time the op's `COMMIT`s took; a record that did not measure a wait or a commit time is left out of its percentiles and counted in its `unmeasured`, and a `0` commit time, an op that ran no `COMMIT`, is a measure; a summary written before #2340 has no `commit_us`; a run-watch aggregate, #2338, adds one sample, its slowest tick's, for all the ticks it holds, so `_watch`'s p50 and p95 lean high), `busy` (records whose busy handler fired), and `unsampled` (the percentiles are taken from a uniform sample of at most 4096 of an op's records, drawn over the whole run; the records not held are counted here, and left out when none) |
| `busy` | Records whose busy handler fired, over every op |
| `tasks` | Tasks `created`, `claimed`, `released`, `blocked`, `submitted` and `accepted` (verified) by this process's answered ops, by their decisions |
| `messages` | Messages `sent`, `acked` (consumed) and `withdrawn` by this process's ops, likewise |
| `process_span_us` | This process's own span: from the start of the first record of the run it folded to the summary (not the run's wall time, which is `run.wall_time_us`) |
| `request_usage` | Per member (`actor_ref`, redacted as in `swarm_op`), the provider requests this harness `recorded` on the board (`_record_request`) and those `refused` |
| `unlisted_ops` | Records of ops past the summary's bound of 128 ops, counted in `records` but in no `ops` entry; left out when none |
| `unlisted_requests` | Requests of members past the bound of 64, in no `request_usage` entry (they are still counted under `ops._record_request`); left out when none |

The run's own totals (`run`), every member's work, read from the board file at
settle by one host read (`_run_totals`, recorded as a `swarm_op` with role
`host`); `null` when that read failed (a warning is logged):

| Field | Meaning |
|---|---|
| `run.tasks` | The run's tasks: `total`, and by state `ready`, `claimed`, `blocked` (a `ready` task waiting on a dependency counts here, as `summary` counts it), `submitted` and `completed` |
| `run.messages` | Every message `sent`, and those `acked` (consumed by their recipient) or `withdrawn` |
| `run.usage` | Per member (`actor_ref`, redacted), the board's request ledger: `requests`, `tokens` (the tokens the budget counted), `unknown_usage_requests`, `attempts`, and the reported `input_tokens`, `output_tokens`, `cache_read_tokens` and `cache_write_tokens`; at most 64 members |
| `run.unlisted_usage` | Members past the 64, in no `usage` entry; left out when none |
| `run.wall_time_us` | The run's wall time: from its `created` event to the read at settle, on the board's clock; `null` when the board holds no creation time |

The fold across every member's records (their latencies and refusals) is left
to `quecto swarm report` (#2305), which can fold each member's `swarm_op` lines
of a run offline.

It carries counts, kinds, durations and ids only, never board text, and leaves
one `tracing` record (`swarm run summary`, INFO) on `quecto::swarm_board`. Being
written once, off every board call's path, it waits up to 2 s for the log's
write gate (a `swarm_op` record waits 50 ms); held off past that, it is dropped
and counted apart from the records, in a
`{"event":"swarm_run_summary_dropped","dropped":N}` line written before the next
record (a summary held past the 64 before the session's log opened is counted
there too).
Select a run's summary as its records
are selected:

```sh
jq -c 'select(.event == "swarm_run_summary" and .run_id == "<run id>")' ~/.quecto/audit/*.jsonl
```

The board's records are written to the current session's log: when the
harness switches session, its board calls are recorded in the arriving
session's log from then on, as the agent's own records are. They are not pinned
to the session a call was made for: the process has one board, so its run
watcher's calls, its settlement reads and the run's summary land in whichever
session is current when they are made, and one run's records (and the fold its
summary is taken from) can span two sessions' files. Records the
departing log dropped at its write gate and had not noted yet are noted in the
arriving log's first `swarm_ops_dropped` line.

A swarm member decides whether the event log is on before its admission, from
the configuration its build then loads (the same layers and `QUECTO_*`
overrides, without asking to trust an overlay). When it is on, the admission's
own board calls (`_status`, `_bootstrap`, `_activate`) are measured and held,
at most 64, and written first once the session's log opens, followed by a
`swarm_ops_dropped` line counting any records held past the 64, and a
`swarm_run_summary_dropped` line for any run summary (a board call made while
they are written waits for none of them, and lands after them); when it is off,
or the session opens no log, nothing is measured or written. A
`--backend claude-code` member opens no event log of its own, so its
admission's calls leave `tracing` records only.
