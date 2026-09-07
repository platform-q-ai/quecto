# Packaged cooperative swarm spike

`swarm.py` is a single standard-library-only helper suitable for Rust
`include_str!` + execution in a fresh namespace inside `python -I`. It does not
require imports from this directory, a pip package, a service, or subprocesses.
Load the source with `exec(source, namespace)` and use `namespace['Swarm']` and
`namespace['demo']`. No work happens at module load.

```python
s = Swarm.create('/tmp/new-swarm.sqlite', goal='Compute a result',
                 done='Coordinator checks result', deadline=time.time() + 60,
                 coordinator='lead', members=['lead', 'worker'], token_budget=100)
s.add_task('lead', 'calculate', 'Compute 2+2', token_limit=20)
c = s.claim('worker', 'calculate')  # None if unavailable, blocked, or over budget
s.finish('worker', 'calculate', c['claim_token'],
         evidence=[{'expression': '2+2', 'result': 4}], tokens_used=8)
s.complete('lead', 'Independently checked 2+2 == 4')
print(s.summary())
```

Actual provider instructions can use the member-bound convenience API (no
background loop, subprocess, or thread is started):

```python
w = worker('/tmp/new-swarm.sqlite', 'worker')  # also accepts a Swarm instance
c = w.claim()  # inspect c['description']; None means no eligible work
if c is not None:
    # Perform the assigned work through existing harness tools; do not exec task text.
    w.message('lead', 'Claimed ' + c['task_id'])
    # After doing and checking the work, report real evidence and measured usage:
    w.finish(c['task_id'], c['claim_token'], evidence=[{'result': 4}], tokens_used=8)
```

`worker(board, member)` returns `Worker` with `claim`, `finish`, `message`,
`messages`, `reserve`, `release`, and `summary`, matching `Swarm` methods but
omitting the actor argument. It validates membership, not provider identity.
Coordinator-only operations remain on `Swarm`. No CLI is required; the runner
can call these functions directly from the loaded module namespace.

Reconnect with `Swarm(path)`. All mutating calls are serialized SQLite
transactions (`BEGIN IMMEDIATE`, ten-second lock timeout); methods open/close
connections individually so instances do not share connection/thread state.
Creation refuses an existing file and creates the database with mode 0600.

API:

- `create(path, *, goal, done, deadline, coordinator, members, token_budget)`:
  absolute Unix deadline; immutable, unique membership of 1–10 including lead.
- `add_task(actor, task_id, description, *, dependencies=(), token_limit=0)`:
  coordinator only; dependencies must already exist, preventing cycles.
- `claim(actor, task_id=None)`: atomically claims a ready task and reserves its
  full token limit. Automatic selection is lexical task-ID order among eligible
  tasks. Returns task fields including an opaque claim token, or `None`.
- `finish(actor, task_id, claim_token, *, evidence, tokens_used)`: owner/token
  match required; nonempty finite JSON evidence list; reported usage cannot
  exceed reservation. Finishing releases reservation and charges actual usage.
- `message(actor, recipient, body)` → monotonic ID; `None` means broadcast.
  `messages(actor, *, after=0)` returns recipient/broadcast messages in ID order.
- `reserve(actor, path)` → bool; `release(actor, path)`: exact normalized
  workspace-relative POSIX path ownership. Same owner reserve is idempotent.
- `complete(actor, evidence)`: coordinator attestation string, nonzero tasks,
  all finished. Marks terminal and releases file reservations.
- `cancel(actor, reason)`: coordinator only; works after deadline, cancels
  unfinished tasks, clears claim handles/reservations, marks terminal.
- `summary()`: coherent snapshot of metadata, tasks, dependencies, messages,
  evidence, reservations, usage, and deadline-exceeded flag. Omits claim handles.

Only reads and cancellation are possible after deadline. Terminal state rejects
all mutations. Invalid requests raise `SwarmError`; filesystem/SQLite operational
errors are propagated. The optional `clock` argument on construction supports
deterministic tests.

## Safe fake demo and tests

`demo('/tmp/new-demo.sqlite')` runs **two explicitly fake arithmetic workers** in
concurrent threads with separate connections and a rendezvous barrier. Workers
read assignments via messages, record squares, and message the coordinator. The
coordinator independently verifies every input/square, then records total 55 and
completes the swarm. No LLM, shell, network, repository edits, or external agents.
The only persistent effect is creating the requested SQLite database. Existing
paths are never overwritten. No subprocess is used anywhere in the helper.
However, Linux PID/process limits can count threads: the concurrent demo needs
room for the Python main thread plus two worker threads. PythonLab's default
`max_processes=1` may therefore block it; configure a sufficiently higher limit
(at least 3 for this interpreter, plus any runner overhead) when running `demo`.
Ordinary `Swarm`/`worker` calls remain single-threaded and need no increase.
The embedding runner owns this configuration; the helper never changes limits.
Worker/evidence arrival order is intentionally
unspecified; arithmetic and verification are deterministic.

```sh
python3 -m unittest discover -s quecto-agentic-harness/src/infrastructure/tools/swarm_python
```

Development: wrote contract tests first; initial run failed with missing `swarm`
module (RED). Implemented helper; all 10 tests pass (GREEN), including competing
claims across connections, dependencies, budgets, ownership, terminal/deadline
checks, reservations, messages, and independently verified fake demo.

## Deliberate limits

This is a bounded coordination helper, **not an orchestration platform**.
Membership/actor names are cooperative caller assertions, not authentication or
harness-enforced isolation. Anyone with DB access can bypass every rule; summary
is not a private mailbox. Harness capabilities/process boundaries must enforce
real membership and permissions. Token usage is self-reported, not metered LLM
usage. Evidence is data, never executed; only the demo verifies arithmetic—the
coordinator must check real evidence against the human-readable done criterion.

Reservations do not lock OS files, resolve symlinks/case aliases, cover directory
subtrees, or prevent noncooperative edits. No crash recovery, lease/heartbeat,
reclaim, retries, dynamic membership, task deletion, distributed filesystem
support, background deadline watchdog, or automatic cancellation. A crashed
claim holds its budget until coordinator cancellation. Wall-clock checks occur
at transaction authorization, not mid-operation; expiration alone does not
mutate stored status. Cancellation does not terminate running agents or refund
real usage; unfinished usage is unknown. Cancellation and completion clear file
reservations. Tasks/messages/evidence have no size/count quota; caller/harness
must bound payloads and database growth. Intended for a trusted local filesystem
and a short-lived small swarm.
