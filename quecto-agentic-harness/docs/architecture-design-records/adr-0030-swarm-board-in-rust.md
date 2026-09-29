# ADR-0030: The swarm board runs in Rust, on the same SQLite file

**Status:** Accepted
**Date:** 2026-09-29
**Scope:** Epic #2265 (E1). Recorded by #2278 (E1-S13), the slice that
switches the harness to the Rust board.

## Context

The swarm coordination board was about 1,670 lines of Python: the policy
in `swarm_policy.py`, the use cases in `swarm_use_cases.py`, and SQL spread
over `swarm_store.py`, `swarm_repository.py`, `swarm_tasks.py` and
`swarm.py`. The harness reached it through a persistent `python3 -I`
interpreter per (checkout, database, member), and members reached it
through `op=run` Python programs. That meant two languages for one set of
invariants, a Python runtime on every host and in every container, a second
execution sandbox, and a typed API that weaker members only reached through
code strings.

Slices S1–S12 ported the board to Rust (domain policy, a Python-compatible
JSON codec, a rusqlite store, one use case per board method, and a
method-name dispatcher), each proven identical to Python by a differential
harness. S13 switches the callers.

## Decision

Owner decisions (overridable by the owner only):

- **D1 — Python is dropped entirely.** The `op=run` workbench (inline and
  file Python, background jobs, the `status`/`output`/`cancel` job ops) is
  removed (#2282). Members use the `bash` tool for computation, Git and
  tests.
- **D2 — No new schema version and no migration.** Rust uses exactly the
  existing board schema: the same tables, columns and column order,
  indexes, constraints, `sqlite_master` DDL text, lazily created tables and
  in-place `ADDED_COLUMNS` upgrade; the same pragmas, journal mode,
  transaction and locking behaviour, token and claim semantics, timestamp
  and id formats, and error conditions and messages wherever they are
  observable. Boards created by Python open and keep working. During the
  transition Python and Rust operate on the same file concurrently.
- **D3 — SQLite stays.** `rusqlite` with the `bundled` feature is only the
  Rust driver for the same file format. SQLite is compiled into the
  `quecto` binary: no Python and no system `libsqlite3` on hosts or in
  containers.
- **D4 — Structured ops are 1:1.** Each member-facing `board.<method>`
  becomes a `swarm` op with the same name, the same arguments (names, JSON
  types, defaults) and the same result shape. Only the calling syntax
  changes (#2279).

Planning decisions (recorded for the owner to overrule):

- **P1.** Where a board method and an existing harness op share a name
  (`summary`, `pause`, `resume`, `events`, `usage_budget`), the existing
  op is kept. `create` stays the harness op (`board.create`, `_activate`
  and supervision, with `deadline_in_seconds`); the raw `board.create` is
  never exposed.
- **P2.** Argument-binding errors (a missing or unknown argument) are
  calling syntax and carry a Rust message; every error the board raises
  keeps Python's exact text.
- **P3.** Behaviour on well-typed inputs, and on the loosely typed inputs
  the board accepts today, matches. Inputs Python rejects with exceptions
  other than `SwarmError` may give different text but are still errors; the
  differential suite lists each permitted divergence by name.
- **P4.** Superseded by the owner (2026-09-28): structured ops keep
  today's `op=run` gate, so board calls are refused unless the run is
  running.
- **P5.** After a structured op that moved the board's event cursor, the
  tool runs the same post-call lifecycle `op=run` ran: notify on a cursor
  change, settle when the run is no longer running.
- **P6.** `python3` stays in this repository's own dev container (its
  scripts need it) and leaves the starter Containerfile and the harness
  image.

### What "identical" means

For every operation sequence, with an injected clock and id source:

1. **Results.** Equal JSON values (integer and float distinct); errors
   compare by exact text at the tool boundary, `swarm: "<text>"`.
2. **Database.** Identical `sqlite_master` rows and an identical logical
   dump of every table by rowid (`files` by path, since Python reserves in
   hash order). Page layout, the change counter and free pages are not
   compared.
3. **Connection behaviour.** One connection per transaction, each begun
   with `BEGIN IMMEDIATE` (reads included); `PRAGMA foreign_keys=ON`;
   500 ms busy timeout; the default rollback journal, never WAL; `mode=rw`,
   or `mode=rwc` only on create and bootstrap; a missing store refused
   before opening (#2145); the expiry transaction commits even when the
   mutation after it is refused.
4. **Ids and time.** `uuid4().hex` ids, REAL Unix-second event times, and
   the same order of id draws and clock reads within an operation.
5. **Stored JSON.** Columns written by `encode()` and by plain
   `json.dumps` are written byte for byte, Python float `repr` included.
6. **Mixed writers.** Python and Rust clients take turns on one file and
   see each other's writes; under contention both answer
   `coordination store unavailable or contended: database is locked`.

### The switch (S13)

`SwarmContext` and `HostedStore` call the Rust dispatcher in-process,
through `SwarmBoard`: composition's handles builder
(`composition::swarm::build_swarm_board_handles`), carried from `main`
through `CliComposition` and `CliContext`, bound once per process by the
agent's admission and given to the host's `HostedStoreObservation` by
`composition::environments` (the process's board when admission bound
one, so the host's reads are recorded too). The handles are built once
per board file, and each call is recorded in the session's event log
(`swarm_op`, #2303) once the log is open and switched on. Every board
call runs off the async workers (a debug build asserts it in
`SwarmBoard::call`): on the blocking pool, or on a thread outside any
runtime. Board refusals keep their wrapper,
`swarm: "<text>"`. The persistent interpreter (`swarm_board_worker.rs`)
and its teardown-authority entry are deleted: no child process remains
to leak.

Two recording gaps were known at the switch, and #2313 closed them:

- **Admission's own calls.** The admission's `_status`, `_bootstrap` and
  `_activate` run before the session's log is attached
  (`event_log::attach`). The event-log switch is now decided before
  admission, from the configuration the build then loads (owner decision
  T1 holds: with it off, nothing is measured or written), and the calls
  are held until the log opens, then written there first. A
  `--backend claude-code` member never attaches a log, so its admission's
  calls still leave `tracing` records only (E2).
- **One log per process.** A board recorded in the first log it was given.
  It now records in the current session's log: a session switch rebinds
  the board's log (`SwarmBoard::follow_session`), so in multi-session UDS
  mode each session's board calls go to its own log.

Python remains only for `op=run` member programs until #2282, and those
programs use the Python board over the same file as the harness's Rust
calls (`tests/integration/swarm_board_mixed.rs` proves the two writers
interleave and contend without a difference from a single writer, and
`swarm_board_mixed_harness.rs` that the harness's calls and a member's
programs leave the pure-Python board's file).

*Note (#2282):* #2282 removed `op=run` and its Python workbench (jobs,
interpreter sandbox, the `tools.swarm` limits); members compute through
`bash`, and every board call is a structured op on the Rust board. The
paragraph above records the state before it.

## Consequences

- No `python3` is started by the harness for its own board calls; each
  board call is a direct function call on the caller's (blocking) thread.
- Timing shifts slightly: the interpreter's start-up (about 100 ms) is
  gone.
- The differential suite stays until #2283 deletes the Python board and
  freezes it into golden fixtures.
