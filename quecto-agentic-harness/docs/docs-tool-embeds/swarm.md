# Swarm workbench

`swarm` coordinates one fixed pool of 1–10 agents, including its coordinator,
in a shared container and checkout. The host master uses `spawn` to launch the
container/coordinator, then supervises it through `agent_cmd`. The coordinator
creates the run before spawning local workers. Never reset a run or spawn nested
containers to evade its limit. See `docs {"name":"subagents"}` for launching.

## Python versus external commands

The default Python process limit is **RLIMIT_NPROC=1**. `swarm` Python is for
in-process computation and `from swarm import board` coordination. Subprocesses
such as `subprocess.run(['git', ...])`, grep, or Bash may fail with
`BlockingIOError: [Errno 11] Resource temporarily unavailable` even when the
container has available capacity. This is a configured execution restriction,
not proof that the container is full. Privileged processes may be exempt.

Use the existing **bash** tool for Git, tests and external commands, subject to
its configured policy. Read the resulting artifacts with normal file tools and
record their references on the board. Only the integrator may change branches,
commit or integrate. `tools.swarm.max_processes` is operator configuration; do
not raise limits or route around policy from agent code. An optional observed-token budget is described below.

## Create once

Call `swarm` with `op=create` and these fields:

| Field | Type and constraints |
|---|---|
| `goal` | Nonempty string |
| `constraints` | `list[str]` |
| `criteria` | Nonempty list of objects: `id`, `description`, `kind` (`command` or `review`) |
| `member_limit` | Integer 1–10; coordinator and idle/reserved workers count |
| `deadline` | Unix timestamp in seconds, in the future and within seven days |

Example criteria: `[{"id":"tests","kind":"command","description":"Acceptance tests pass"},{"id":"review","kind":"review","description":"Independent reviewer accepts the final revision"}]`.

## Python calls

Use `swarm {"op":"run","code":"from swarm import board; print(board.summary())"}`.
Each invocation is a fresh Python process; store durable work on the board.
Exactly one of `code: str` or `path: str` is required. Optional execution fields:
`args: list[str]`, `stdin: str`, `timeout_seconds: int`, `max_output_bytes: int`,
and `background: bool`. A background launch returns `job_id`; use `op=status`,
`op=output` (optional byte `offset`/`limit`) or `op=cancel` with that ID.

In the signatures below, IDs are task/message integers; member IDs and tokens
are strings returned by the API. `Evidence` is a nonempty list of
`{"artifact": "tests.log", "revision": "actual-commit-id"}` objects. Store large
content in artifacts, not messages or evidence fields.

| Call | Input and behavior |
|---|---|
| `summary(since=None)` | Current goal, members, counts, first 50 tasks/files and evidence; no history. Reuse `event_cursor` as `since` for a compact unchanged response |
| `events(after=0, limit=25)` | Explicit chronological history; limit 1–100 |
| `tasks(offset=0, limit=50)` / `file_owners(offset=0, limit=50)` | Integer offset ≥0; integer limit 1–100 |
| `task_create(request, title, acceptance, dependencies=None)` | Stable request string, title string, **nonempty `list[str]` acceptance**, optional `list[int]` dependency IDs; returns a task |
| `task(id)` / `dependencies(id, ids)` | Read task; change dependency `list[int]` before claiming |
| `claim(id)` | Returns owned task with a new claim `token`; unmet dependencies reject |
| `block(id, token, reason)` | Nonempty string reason |
| `unblock(id, token, reason)` | Resume your blocked claim, preserving token and reservations; submitted work cannot be reopened |
| `release(id, token)` | Release own claim/files; future claim gets a new token |
| `submit(id, token, evidence)` | Submit `Evidence` for coordinator verification; submission is not completion |
| `reserve(id, token, paths)` | `list[str]`, 1–100 checkout-contained paths, all-or-nothing; returns reservation token |
| `release_files(id, token, reservation)` | Release that reservation token for this claim |
| `send(request, recipient, body)` | Stable request string, member ID, **string** body ≤8192 UTF-8 bytes; returns message ID/status |
| `inbox(include_consumed=False)` / `ack(message_id)` | Read at most 100 own messages; acknowledge after reading |
| `evidence(criterion, artifact, revision, kind, passed)` | Strings plus `passed: bool`; workers record proposals with **accepted=0**. Only coordinator calls with `passed=True` accept evidence |

For example:

```python
from swarm import board
work = board.task_create('implement-v1', 'Implement behavior', ['Acceptance tests pass'], [])
claim = board.claim(work['id'])
board.reserve(work['id'], claim['token'], ['src/example.rs'])
# Use normal edit/bash tools to perform work and run checks.
# In a later invocation, retrieve the task and its claim token again.
board.submit(work['id'], claim['token'], [{'artifact': 'tests.log', 'revision': 'actual-commit-id'}])
```

A string such as `acceptance='tests pass'` is invalid; use `['tests pass']`.
Dictionary message bodies are invalid; serialize the intended message to a string.
Do not reserve `/tmp` or paths outside the checkout. Ownership is cooperative,
not a filesystem lock. Stale tokens cannot mutate someone else's work.
Retrying `task_create` or `send` requires the same request ID **and** payload.

## Coordinator verification and control

- `verify_task(id, token, revision)` accepts submitted task evidence at that revision.
- `revalidate_task(id, revision, fresh_evidence)` requires a completed task and new
  `Evidence` matching the final revision after later dependent work changes it.
- `amend(goal, constraints, criteria, reason)` updates scope with a string reason
  and invalidates prior overall evidence.
- `complete(revision)` requires every criterion accepted and every task verified
  at that revision, with no outstanding work or file reservations.
- `stop(status, reason)` accepts `blocked`, `failed`, `cancelled`, or
  `budget-exhausted`; use tool `op=cancel_run` for parent cancellation.
- `recover(id)` requires proof the entire former owner's execution scope stopped.
  The current adapter cannot establish that from harness death alone: it fails
  the run and retains ownership. Stop/discard that environment; do not reassign.

Actually inspect command results and independent review before accepting them.
Worker proposals, an empty queue or a message acknowledgment do not prove done.
Only the coordinator may amend, verify, revalidate, complete, stop or recover.
Workers may call `evidence`; their proposals never authorize completion.

## Wakeups, terminal inspection and artifacts

Wake hints are coalesced from actionable changes, not read/ack traffic. They are
best-effort; durable inbox/task state is authoritative. **First use op=summary**
when receiving a hint. If running, inspect inbox/ready tasks and acknowledge read
messages. If terminal, do not call `op=run` for inbox/ack: Python execution is
closed. Report the final summary and remain available for supervisor requests,
with export handled through the supervisor channel. A queued hint can arrive after completion.
Do not repeatedly poll, sleep or acknowledge an empty inbox.

`op=summary` remains available after completion. An external master retrieves the
coordinator's latest report with `agent_cmd.get_report`; add `export_raw:true`
to export retained records. Export is explicit, not automatic. Preserve evidence before environment disposal.

Returned artifact paths use **workspace-relative** references. `artifact_base`
names the execution workspace inside the container; join it with each
`artifact_paths` entry, e.g. `.quecto/swarm/<execution_id>/stdout.txt`. Status,
output paging and synchronous spills use the same namespace. These are not host
paths. The SQLite board is `.quecto/swarm.sqlite`; old execution directories may
be pruned, so copy important evidence to durable report files before cleanup.

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
master and yield the turn. Do not sleep/poll or use `board.stop('blocked', ...)`
as a pause: a blocked **run** is terminal, closes Python execution and settles
workers. A blocked **task** retains its claim and can be submitted after the
answer arrives and work is completed. The original deadline continues to apply.

The master sends the answer with `agent_cmd` `prompt` when idle, or `steer` when
it must interrupt a busy coordinator. A queued `follow_up` waits for the current
turn to finish. The coordinator should explicitly acknowledge the answer and
apply it to the task before optional inbox work. Transport acceptance means the
command was queued, not that the model read or acted on it; retrieve the report
with `get_messages` to verify handling. A full/closed dispatch queue returns an
explicit failure instead of falsely accepting the clarification. A terminal run
cannot be revived by steering: preserve its report and start a fresh environment
when further implementation is authorized.

Ordinary Bash build/test commands strip swarm launch-context variables, so their test runtimes do not enroll in this live pool. Launch participating agents with managed `spawn`. Wake delivery checks current unread messages and claimable tasks, but queued hints can become stale; inspect the board before acting. Steering takes priority over buffered idle work. Forwarded controls return `data.status: "accepted"` on queue admission and retain that response ID through dispatch; inspect the report for actual acknowledgment and results.

## Durable supervisor controls and reports

The master can call `agent_cmd` with `command:"swarm_control"` and
`action:"pause"`, `"resume"`, or `"status"`. Pause needs no model turn and bypasses
the prompt queue. It preserves claims, artifacts and agents, cancels current
execution, suppresses automatic wakes and freezes the wall-clock deadline until
resume. Use this for a whole-run approval wait. `stop("blocked", ...)` remains
terminal. Native coordinator operations `swarm {"op":"pause","reason":"..."}`
and `swarm {"op":"resume"}` offer the same durable state transition.

After sending an answer, inspect `agent_cmd get_state` → `controlReceipts` by
command ID. `queued`, `started`, `completed`, `failed`, `cancelled`, and `rejected`
describe handling. These receipts retain the latest 64 commands in the live
session. `completed` means the turn completed; verify the coordinator's explicit
acknowledgment and resulting work in `get_report`.

`agent_cmd {"agent_id":"...","command":"get_report"}` returns the latest
substantive assistant report independently of unread transcript backlog. Long
reports include stable `get_message` recovery parameters. Add `export_raw:true`
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
Quota failures suspend automatic turns. Send an explicit prompt after resolving
the cause. Do not repeatedly wake or retry ahead of a provider's reset horizon.

Terminal coordinators remain available for reports. Their tool execution is
restricted to native read-only swarm `summary`, `events`, and `usage`; Python,
Bash, spawning and board mutations are unavailable. Export through the supervisor
channel and preserve the container until the user authorizes teardown.

Paused instructions remain queued; resume restores admission and the deadline.
A resume also wakes every live member, and a member whose automatic turns were
suspended by a provider failure re-arms on it and continues its work without a
prompt or steer. Do not pause a run because one member failed: resume the run
and, only if the member is still stuck, steer it.
Terminal completion notices do not trigger automatic report turns.
