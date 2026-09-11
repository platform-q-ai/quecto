# Issue #1925 verification

Registry teardown must only signal pids this harness launched itself.

## Mechanism

- `ProcessOwnership` (`quecto-agentic-harness/src/infrastructure/tools/process_ownership.rs`)
  now starts **unowned**. `SubagentEntry::new` / `with_identity` use
  `ProcessOwnership::unowned()`, so snapshot-merged descendants
  (`subagent_monitor_merge.rs`) and persisted-session restores
  (`uds_dispatch_session.rs`) inherit no signal authority.
- Only `SpawnLaunchPorts::register` (`spawn_launch_ports.rs`) claims authority,
  via `ProcessOwnership::launched(&child)`, which requires the spawned
  `tokio::process::Child` handle rather than a bare pid. The same lease is
  handed to the reaper, which retires it on reap (unchanged).
- `signal()` returns whether a signal was dispatched; `shutdown_all` logs
  "sent termination" only when it actually did. `terminate_removed_entry`
  (cascade + `agent_cmd kill` + reaper) goes through the same lease.
- `spawn_container::rollback_once` signals `child.id()` from its own held
  child handle, so it is owned by construction and unchanged.

## Fixtures

Every literal live pid in `tests/bdd/*_steps.rs` (1, 2, 3, 4, 42) and in
non-asserting `src/**/*_tests.rs` fixtures was replaced with 0. Fixtures where
the pid value is itself projected/asserted keep it; they are unowned by
construction.

## Commands and results (2026-09-11, worktree on origin/master 742f05fc)

| Command | Result |
| --- | --- |
| `cargo test -p quecto-agentic-harness --lib` | ok, 4375 passed, 0 failed |
| `cargo test -p quecto-agentic-harness --features test-support --test bdd </dev/null` | 1483 scenarios (1483 passed), 7223 steps (7223 passed), exit 0; no pre-existing failures observed |
| `cargo test --test architecture` (in quecto-agentic-harness) | ok, 49 passed |
| `cargo fmt --all` | clean |
| `cargo clippy --workspace --all-targets --features quecto-agentic-harness/test-support -- -D warnings -W clippy::cognitive_complexity -W clippy::too_many_arguments -W clippy::too_many_lines` | Finished, no warnings |

### Regression tests (`spawn_registry_ownership_tests.rs`, `agent_cmd_kill_tests.rs`)

- `unowned_entry_carrying_our_own_pid_is_never_signalled`: installs a real
  counting SIGTERM handler (positive control: a self-directed `kill` is
  observed), then runs `terminate_removed_entry` (DirectPid and
  LocalProcessGroup) and `shutdown_all` on entries with `pid = std::process::id()`.
  Zero deliveries.
- `owned_launched_child_is_still_terminated_on_shutdown`: spawns `sleep 30`,
  claims `launched(&child)`, `shutdown_all`, waits: exit by SIGTERM.
- `entry_constructors_default_to_unowned`.
- `kill_parent_sigterms_owned_process_and_skips_unowned_descendant_pid`: owned
  parent dies by SIGTERM; unowned descendant pid is left untouched.

### Mutation check

Committed, then flipped `unowned()` to grant authority:
`cargo test -p quecto-agentic-harness --lib -- ownership_tests kill_tests`
-> 3 failed (`unowned_entry_carrying_our_own_pid_is_never_signalled`,
`entry_constructors_default_to_unowned`,
`kill_parent_sigterms_owned_process_and_skips_unowned_descendant_pid`).
Reverted.

## Adversarial self-review

- (a) Remaining signal paths: `process_tree::*` is only reachable through
  `ProcessOwnership::dispatch` or `spawn_container::rollback_once` (own child
  handle). `swarm_process.rs` and `bash/mod.rs` kill pids from their own
  `Child` handles / process groups. `session_ownership.rs` uses `kill(pid, 0)`
  as a liveness probe only.
- (b) Leaks: every real local launch registers through `SpawnLaunchPorts::register`
  with `prepared.child` present, which is exactly where `launched(&child)` is
  claimed; the stub path (`spawn.rs`) registers pid 0. Persisted-session
  restores and snapshot-merged descendants were never this process's children;
  the child harness that launched them tears them down on its own SIGTERM.
- (c) Restore: `uds_dispatch_session.rs` builds via `with_identity` -> unowned.

## Not done

- Item 4 of the issue (re-run the suite inside a swarm container) was not
  performed from this worktree.
