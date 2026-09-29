# Swarm coordination

`swarm` coordinates one fixed pool of 1–25 agents, including its coordinator,
in a shared container and checkout. The host master uses `spawn` to launch the
container/coordinator, then supervises it through `agent_cmd`. The coordinator
creates the run before spawning local workers. Never reset a run or spawn nested
containers to evade its limit. See `docs {"name":"subagents"}` for launching.

## Which container

- **The swarm runs in the container the coordinator was spawned into.** There
  is no swarm image or swarm config: the host master picks a `container_configs`
  entry — `agent_cmd {"agent_id":"*","command":"get_container_configs"}` lists
  the names (the spawn description's roster line shows the same set) — and
  names it
  explicitly: `spawn {"agent_id":"coordinator","task":"…","container":{"mode":"new","container_config":"<name>"}}`
  (`"container": true` = this repo's `standard` entry when one exists — no
  global default overrides it; run `quecto container init` first if the
  roster shows none — else the labelled default). The result's
  `container_config=<name>` confirms the choice; a new container is a fresh
  clone of that config's `--repo`, not the master's working tree.
- **Workers are spawned with `container` omitted** (a local spawn inside the
  shared container); any other `container` value from a swarm member is
  refused. They inherit the container's identity through their environment.
- **Only the official isolated-PID adapter (`scripts/container-runtime/docker`)
  can host a swarm.** The host-local reference scripts
  (`scripts/container-runtime/*.sh`) cannot host a swarm, and neither can the
  host: `swarm` there fails with `swarm is container-only`. A config on the
  wrong adapter fails at `op=create`, not at spawn — check the config's
  `create` argv first (`quecto config get --effective container_configs`).
- **Setup (preconditions → verify).** The standard container written by
  `quecto container init` is that adapter (its `create.sh` sets
  `QUECTO_SWARM_CONTAINER=isolated-pid-v1` and the checkout): follow
  `docs {"name": "container-runtime"}` until `quecto container doctor` exits 0
  and `quecto container status` reports the image present (its last line is
  `ready:`), then spawn the
  coordinator with `"container":{"mode":"new","container_config":"standard"}`.
  Verify: the spawn result names `container_config=standard`; the
  coordinator's `swarm op=create` succeeds; `agent_cmd get_containers` lists
  the environment `running`. Rollback: `kill_container` or `quecto container kill <ref|name>`
  (until the supervisor closes the run its container is `retained` when the
  coordinator goes, and needs one of them;
  `quecto container gc` sweeps exited leftovers), then the container-runtime
  rollback if the repo should lose its config.

## Bash for commands, board ops for coordination

Use the **bash** tool for Git, builds, tests, scripts and any other command or
computation, subject to its configured policy. Read the resulting artifacts
with normal file tools and record their references on the board with the
board ops below. Only the integrator may change branches, commit or
integrate. Do not raise limits or route around policy from agent code. An
optional observed-token budget is described below.

## Create once

Call `swarm` with `op=create` and these fields:

| Field | Type and constraints |
|---|---|
| `goal` | Nonempty string |
| `constraints` | Optional list of strings; omitted or `null` is an empty list |
| `criteria` | Nonempty list of objects: `id`, `description`, `kind` (`command` or `review`) |
| `member_limit` | Integer 1–25; coordinator and idle/reserved workers count |
| `deadline_in_seconds` | Seconds from now, at most seven days; use this instead of `deadline` when you do not know the current Unix time |
| `deadline` | Unix timestamp in seconds, in the future and within seven days (`date +%s` gives now) |

Example criteria: `[{"id":"tests","kind":"command","description":"Acceptance tests pass"},{"id":"review","kind":"review","description":"Independent reviewer accepts the final revision"}]`.

Example: `swarm {"op":"create","goal":"Review PR 1400","constraints":["read-only"],"criteria":[{"id":"tests","kind":"command","description":"Acceptance tests pass"}],"member_limit":3,"deadline_in_seconds":3600}`.
Until a run is created it is in `setup`, and every board op is refused.

## Board ops

The board is a SQLite store the harness serves in-process. Every board call
is one `swarm` call: `{"op":"<name>", ...fields}`, with the op's arguments as
named JSON fields. The answer is the op's JSON result (`null` for an op that
returns nothing). Durable state lives on the board, not in your context.

- **Send the schema's types.** Task and message ids, `offset`, `limit` and
  `supersedes` are JSON integers (`true` and `3.0` are not). `passed`,
  `release_files` and `include_consumed` are booleans, and only `true`
  counts as true. `acceptance`, `constraints` and `paths` are
  lists of strings; `dependencies` is a list of task ids. `evidence` is a
  nonempty list of `{"artifact":"tests.log","revision":"<commit>"}` objects.
  Member ids and tokens are the strings the ops return.
- **Fields bind by name.** A missing required field is refused
  (`claim: missing required argument task_id`), and so is a field the op
  does not take (`claim: unexpected argument id`). A field with a default
  may be left out.
- **Values the board cannot hold exactly are refused up front.** An
  integer outside i64 and u64 (such as `18446744073709551616`), a number
  with no finite value (`NaN`, `Infinity`, or `1e400`, which overflows) and a
  string holding a lone surrogate (`"\ud800"`) are refused with
  `arguments: not representable as a serde_json value: <why>`, and the
  board never sees the call. `-0` is the integer 0.
- **Board ops need a running run within its deadline.** While the run is in
  `setup`, paused, ended or past its deadline, every board op (reads, `inbox`
  and `ack` included) is refused with a message naming what is allowed now:
  `summary`, `events` and `usage` always are.
- **Refusals keep the board's text**, as `swarm: "<message>"`.
- After an op that changed the board, the harness sends the wake hints (or
  settles a run that ended). A hint that could not be delivered is added to
  the answer as `notification_warnings`, and a failure of that step as
  `coordination_error`; the op itself was done. When the answer is not an
  object, it moves under `result` beside them.

Any member:

| Example | Behavior |
|---|---|
| `{"op":"task","task_id":3}` | Read one task. A claimed, blocked or submitted row (also from `tasks` and the summary) adds `owner_last_activity` (seconds since the owner's last board event) and `owner_state`: `active` or `idle` (live member; idle = no board event for 300 s, a prompt to look, not proof of a stall), `reserved` (never launched), `lost` (harness loss recorded; resume, then revoke), `dead` (exit confirmed by its launcher's harness) or `unknown`. An active or idle owner is named in `contact` (send it an `op=send` with that id as `recipient`); otherwise `contact` is null and `recovery` names the coordinator's move. A provider suspension is not visible on the board: use `agent_cmd status` |
| `{"op":"tasks","offset":0,"limit":50}` | Page the tasks; `offset` integer ≥0 (default 0), `limit` integer 1–100 (default 50) |
| `{"op":"file_owners","offset":0,"limit":50}` | Page the file reservations; same paging |
| `{"op":"task_create","request":"implement-v1","title":"Implement behavior","acceptance":["Acceptance tests pass"],"dependencies":[]}` | Stable request string, title string, **nonempty list of strings** as `acceptance`, optional list of task ids as `dependencies` (default `null`, none); answers the task. A retry needs the same request **and** payload |
| `{"op":"dependencies","task_id":3,"dependencies":[1,2]}` | Replace a task's dependencies before it is claimed; missing, self and cyclic dependencies are refused |
| `{"op":"claim","task_id":3}` | Answers the task you now own, with a new claim `token`; unmet dependencies refuse |
| `{"op":"block","task_id":3,"token":"<token>","reason":"needs the schema decision"}` | Nonempty string reason |
| `{"op":"unblock","task_id":3,"token":"<token>","reason":"schema decided"}` | Resume your blocked claim, keeping token and reservations; submitted work cannot be reopened |
| `{"op":"release","task_id":3,"token":"<token>"}` | Release your claim and its files; a later claim gets a new token |
| `{"op":"submit","task_id":3,"token":"<token>","evidence":[{"artifact":"tests.log","revision":"<commit>"}]}` | Submit evidence for coordinator verification; submission is not completion |
| `{"op":"reserve","task_id":3,"token":"<token>","paths":["src/example.rs"]}` | 1–100 checkout-contained paths, all or nothing; answers `{token, paths}` with the reservation token |
| `{"op":"release_files","task_id":3,"token":"<token>","reservation":"<reservation token>"}` | Release that reservation of this claim |
| `{"op":"send","request":"schema-question-v1","recipient":"<member id>","body":"Which schema version?","revision":null,"supersedes":null}` | Any member may message any other member directly; a question about a task you depend on belongs with that task's owner (`contact` on its row). Stable request string, member id, **string** body ≤8192 UTF-8 bytes; optional `revision` the message is about; optional `supersedes`, the id of your own earlier unread message to the same recipient, which becomes `superseded` in the same transaction. Answers `{id, status}`; a retry needs the same request and payload |
| `{"op":"withdraw","message_id":7}` | Withdraw your own unread message; it leaves the recipient's inbox and wake path and stays in the audit as `withdrawn` |
| `{"op":"inbox","include_consumed":false}` | At most 100 of your unread messages (`revision`, `supersedes`, `superseded_by` included); `true` adds consumed, superseded and withdrawn history |
| `{"op":"ack","message_id":7}` | Acknowledge a message after reading it |
| `{"op":"evidence","criterion":"tests","artifact":"tests.log","revision":"<commit>","kind":"command","passed":true}` | `kind` is `command` or `review`. A worker's call records a proposal with **accepted=0**; only the coordinator's `passed: true` accepts evidence |
| `{"op":"usage_report"}` | Per-member usage totals and recent request diagnostics (the same report as `op=usage`) |

The harness's own ops: `{"op":"summary"}` (goal, members, counts of task
statuses plus `members_without_claim` and `members_dead`, the first 50
tasks/files and evidence; add `"since":<event_cursor>` for a compact
`unchanged` answer, which an owner turning idle by the clock alone since that
cursor turns into a full summary again, and `next_liveness_check_at` says
when the next one would), `{"op":"events","after":0,"limit":25}` (explicit
chronological history; limit 1–100), `{"op":"usage"}`, `op=reconcile` and
`op=cancel_run`.

A working sequence (each line is one call; later calls reuse the ids and
tokens earlier answers returned):

```json
{"op":"task_create","request":"implement-v1","title":"Implement behavior","acceptance":["Acceptance tests pass"],"dependencies":[]}
{"op":"claim","task_id":1}
{"op":"reserve","task_id":1,"token":"<claim token>","paths":["src/example.rs"]}
{"op":"submit","task_id":1,"token":"<claim token>","evidence":[{"artifact":"tests.log","revision":"<commit>"}]}
```

Edit and test with the normal edit and bash tools between the `reserve` and
the `submit`. In a later turn, read the task again for its claim token. A
string `acceptance` such as `"tests pass"` is invalid; use `["tests pass"]`.
An object `body` is invalid; serialize the message to a string. Do not
reserve `/tmp` or paths outside the checkout. Ownership is cooperative, not a
filesystem lock. Stale tokens cannot mutate someone else's work. Store large
content in artifacts, not messages or evidence fields.

## Coordinator verification and control

| Example | Behavior |
|---|---|
| `{"op":"verify_task","task_id":3,"token":"<claim token>","revision":"<commit>"}` | Accepts the submitted task's evidence at that revision |
| `{"op":"revalidate_task","task_id":3,"revision":"<final commit>","evidence":[{"artifact":"tests-final.log","revision":"<final commit>"}]}` | Needs a completed task and fresh nonempty evidence matching the final revision, after later dependent work changed it |
| `{"op":"amend","goal":"...","constraints":["read-only"],"criteria":[{"id":"tests","kind":"command","description":"Acceptance tests pass"}],"reason":"scope narrowed"}` | Updates the contract with a string reason and invalidates prior overall evidence |
| `{"op":"complete","revision":"<commit>"}` | Needs every criterion accepted and every task verified at that revision, with no outstanding work or file reservations |
| `{"op":"stop","status":"blocked","reason":"..."}` | `status` is `blocked`, `failed`, `cancelled` or `budget-exhausted`; use `op=cancel_run` for parent cancellation |
| `{"op":"revoke","task_id":3,"reason":"member silent"}` | Takes a claim back (see below); answers the task |
| `{"op":"recover","task_id":3,"release_files":false}` | Reopens work whose owner's death was confirmed (see below) |

- `revoke` takes a claim back from a member that will not finish
  (suspended, hung, silent), alive or not: the task returns to `ready` with no
  owner, token, blocker or evidence, its file reservations go, the audit records
  the reason and previous owner, and the previous owner is messaged. Its stale
  token then fails with `stale or unowned claim`. A repeat on an unowned task is
  a no-op.
- `recover` reopens work whose owner's death the
  harness confirmed: a member you launched that exited (on its own or by your
  `agent_cmd kill`) is marked dead by your harness, its tasks block with
  `worker death confirmed; coordinator recovery required`, and the run keeps
  running. An orderly end (exit code, protocol shutdown, delegated kill)
  releases its reservations; an abrupt one (a signal nobody here sent, an
  unobservable exit) retains them because Bash tool children in their own
  process groups may still be writing those paths — the tasks say
  `reservations retained`, and only `revoke` or `recover` with
  `"release_files":true` frees them; both need a running run
  (resume first). A vanished harness seen by `op=reconcile` is not a confirmed
  death: only the member's launcher (or anyone, once that launcher is dead)
  records the loss, after a ten-second grace in which the launcher's reaper
  normally confirms the death instead; a recorded loss pauses the run holding
  `failed`, once per member, and ownership is retained until the master
  resumes and you `revoke`. Other members' reconciles record nothing.

Actually inspect command results and independent review before accepting them.
Worker proposals, an empty queue or a message acknowledgment do not prove done.
Only the coordinator may amend, verify, revalidate, complete, stop, revoke or recover.
Workers may call `evidence`; their proposals never authorize completion.

## Wakeups, terminal inspection and artifacts

Wake hints are coalesced from actionable changes, not read/ack traffic. They are
best-effort; durable inbox/task state is authoritative. New ready work wakes only
members free to take it: while you hold a claimed, blocked or submitted task you
are woken by messages to you and contract amendments, not by others' work
(if you only wait for review and no worker is free, you are woken). After you submit, check `op=summary` and claim
ready work if there is any: you are not woken for work that became ready while
you held your claim. Otherwise yield. On a file
reservation conflict, release the task and yield instead of holding it. **First use op=summary**
when receiving a hint. If running, inspect inbox/ready tasks and acknowledge read
messages. If the run is no longer running, board ops (`inbox` and `ack`
included) are refused: do not retry them. Report the final summary and remain available for supervisor requests,
with export handled through the supervisor channel. A queued hint can arrive after completion.
Do not repeatedly poll, sleep or acknowledge an empty inbox.

`op=summary` remains available after completion. An external master retrieves the
coordinator's latest report with `agent_cmd.get_report`; add `export_raw:true`
to export retained records. Export is explicit, not automatic. Preserve evidence before environment disposal.

The SQLite board is in the checkout's git directory
(`.git/quecto/swarm.sqlite`; `.quecto/swarm.sqlite` only where there is no git
directory), out of reach of git commands. Copy important evidence to durable
report files before cleanup. Never delete the board or its directory: the run
cannot continue, and every call fails with `coordination store missing at <path>`.

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
with `{"op":"block","task_id":3,"token":"<claim token>","reason":"<the question>"}`,
report the exact question to the master and yield the turn. Do not sleep/poll
or use `{"op":"stop","status":"blocked","reason":"..."}` to wait for one task: a blocked **run** is a pause holding `blocked` that only
the master can resume or close. A blocked **task** retains its claim and can be
unblocked (`op=unblock`) and submitted after the answer arrives and work is
completed. The original deadline
continues to apply.

The master sends the answer with `agent_cmd` `prompt` when idle, or `steer` when
it must interrupt a busy coordinator. A queued `follow_up` waits for the current
turn to finish. The coordinator should explicitly acknowledge the answer and
apply it to the task before optional inbox work. Transport acceptance means the
command was queued, not that the model read or acted on it; retrieve the report
with `get_messages` to verify handling. A full/closed dispatch queue returns an
explicit failure instead of falsely accepting the clarification. A run the
coordinator ended (`stop` or `complete`) is a pause holding its outcome: the
master resumes it with `swarm_control resume` to continue the same run, or
closes it with `swarm_control close`; only a closed or cancelled run is
terminal and cannot be revived.

Ordinary Bash build/test commands strip swarm launch-context variables, so their test runtimes do not enroll in this live pool. Launch participating agents with managed `spawn`. Wake delivery checks current unread messages and claimable tasks, but queued hints can become stale; inspect the board before acting. Steering takes priority over buffered idle work. Forwarded controls return `data.status: "accepted"` on queue admission and retain that response ID through dispatch; inspect the report for actual acknowledgment and results.

## Durable supervisor controls and reports

The master can call `agent_cmd` with `command:"swarm_control"` and
`action:"pause"`, `"resume"`, `"close"`, `"extend"` (with `deadline_seconds`)
or `"status"`. Pause needs no model turn and bypasses the prompt queue. It
preserves claims, artifacts and agents, suspends current execution,
suppresses automatic wakes and freezes the wall-clock deadline until resume.
Every end of a run is the same kind of pause holding an outcome: the
coordinator's `stop("blocked"|"failed"|"budget-exhausted", ...)`, its
`complete(...)` (`succeeded`), a spent token budget and a passed deadline.
`status` reports `outcome` and `reason`. Only the master resumes (`resume`
continues the same run with everything intact) or closes (`close` makes the
held outcome terminal and settles the members). A run holding
`budget-exhausted` resumes only after `extend` or a raised `usage_budget`.
The coordinator may `swarm {"op":"pause","reason":"..."}`; `swarm
{"op":"resume"}` is refused for every member, and `cancel_run` is the only
immediate terminal transition. `status` also reports `resume_blockers`: what a
resume would refuse on right now (a passed deadline, a spent budget, or a lost
coordinator).

A swarm container lives as long as its swarm. The swarm ends when the
supervisor `close`s it, or when its owner deletes all sub-agents or moves to
another session (`/new`, a `/resume` that succeeds): the container, board and
checkout are then removed once its coordinator is gone. Nothing else ends
it — a coordinator that exits, crashes, is `kill`ed or cancels its own run,
a refused `/resume`, and the master shutting down (a crash, a lost client — anything but
an ordinary TUI exit, which announces itself and removes the container like `close` does),
leave the environment `retained` (`get_containers` lists it) so the run can
be resumed; `kill_container` removes it. `metadata.retained` reads `run ended: <outcome>; ...`
after an orderly end you have not closed yet (the run is untouched), or names the lost coordinator
when the socket closed on a running (or outcome-less paused) run: that run
is paused holding `failed` and `resume_blockers` names the coordinator to
relaunch. Read the members' harness logs with
`journalctl --user CONTAINER_NAME=quecto-env-<id>` (rootless Podman,
journald driver) or `docker logs quecto-env-<id>` (Docker); the name is the row's
`metadata.container` in `get_containers`.

After sending an answer, inspect `agent_cmd get_state` → `controlReceipts` by
command ID. `queued`, `started`, `completed`, `failed`, `cancelled`, and `rejected`
describe handling. These receipts retain the latest 64 commands in the live
session. `completed` means the turn completed; verify the coordinator's explicit
acknowledgment and resulting work in `get_report`.

`agent_cmd {"agent_id":"...","command":"get_report"}` returns the latest
substantive assistant report independently of unread transcript backlog, in
full up to 64 KiB (a longer one is cut with a notice). Add `export_raw:true`
to write retained message/spill JSONL and a checksum manifest to artifacts; the
response contains paths, not the raw transcript. Paths belong to the target
agent's filesystem. Exports cannot reconstruct data already cleared or evicted.
Unpersisted messages have a null ordinal; do not invent cursors from list indices.

## Observed usage budget

The coordinator uses `swarm {"op":"usage"}` for per-member totals and recent
request diagnostics. Configure `swarm {"op":"usage_budget","token_limit":1000000,
"strict_unknown":true}` or master `agent_cmd` `swarm_control` with
`action:"usage_budget"` and the same fields. Explicit `token_limit:null` disables
it; budgets are disabled by default. An 80% warning is recorded once per budget
configuration. Reaching the limit durably pauses the run. Strict mode also pauses
when attempted requests have no usage receipt. Raise/disable the budget before
resuming, or admission will pause again.

The limit counts reported context-input plus output tokens. It is an observed
usage guard, not a provider quota or billing limit: in-flight requests can finish
and exceed it, and unavailable usage is not zero. Retry counters measure
instrumented call attempts, cache-prefix hashes compare logical system/tool
prefixes, and reported costs are estimates. `get_session_stats` includes request
diagnostics and runtime identity; unknown build revision/digest stays unknown.
Quota failures suspend automatic turns. Send an explicit prompt, follow_up or
steer after resolving the cause: any explicit instruction re-arms the member
once the run admits it (a parent's `agent_cmd prompt` to an idle member
arrives as a follow-up and executes; a paused run keeps it queued), while
buffered automatic notifications wait until it is re-armed.
Do not repeatedly wake or retry ahead of a provider's reset horizon.

Coordinators of an ended or terminal run remain available for reports. Their
tool execution is restricted to native read-only swarm `summary`, `events`, and
`usage`; Bash, spawning and board ops are unavailable until the
master resumes the run. Export through the supervisor channel and preserve the
container until the user authorizes teardown.

Paused instructions remain queued; resume restores admission and the deadline.
A resume also wakes every live member, and a member whose automatic turns were
suspended by a provider failure re-arms on it and continues its work without a
prompt or steer. Do not pause a run because one member failed: resume the run
and, only if the member is still stuck, steer it. For a member that holds a
claim it will not finish, no process is prescribed: an explicit `agent_cmd`
`steer`/`follow_up` re-arms a provider-suspended member (#1712), `agent_cmd`
`set_model` moves it off a failing provider, `op=revoke` reassigns its
work, and `op=recover` applies once its death is confirmed.
Terminal completion notices do not trigger automatic report turns.
