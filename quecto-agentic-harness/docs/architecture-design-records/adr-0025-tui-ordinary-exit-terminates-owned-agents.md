# ADR-0025 — TUI Ordinary Exit Terminates Owned Agents After Durable Roster Capture

**Status:** Proposed.

**Supersedes:** ADR-0023 lifecycle inversion only. ADR-0023's process-per-tab topology and shared-state invariants still stand.

**Implementation status:** Planned by #1586.

## Context

ADR-0023 chose the process-per-tab TUI topology: one replicant `quecto agent --mode uds --persist` process per tab, with the TUI acting as a multiplexer. That decision remains sound.

ADR-0023 also chose a lifecycle inversion: ordinary TUI exit does not kill tab agents. The reason was resume safety: if the TUI closes, it can later reattach to the same live sockets and continue the same sessions.

Operational experience showed a cost to that lifecycle rule. Ctrl-D, `/exit`, and `/quit` close the TUI but leave TUI-owned parent agents and subagents alive as sleeping processes. Repeated ordinary exits can accumulate idle `quecto agent` processes and consume resources. Issue #1584 reported the leak; #1585 investigated the resume dependency; #1586 is the canonical implementation issue.

The key finding is that agent death does not have to mean session loss. Persistent session storage and workspace manifests can restore conversations and tabs. Durable subagent roster snapshots can restore the roster as historical, non-live rows when the previous processes were intentionally terminated. Live socket reattach remains useful for agents that are verifiably still alive, but ordinary TUI exit does not need to preserve processes solely to make `/resume` work.

## Decision

Ordinary TUI exit terminates TUI-owned agents after durable capture.

Ctrl-D, `/exit`, and `/quit` are one semantic operation: ordinary TUI exit. They must share one exit request/finalization path.

The finalization path is ordered:

1. Capture the visible per-tab roster state for all tabs in durable storage.
2. Persist workspace/session durability, including tab mapping and roster snapshots.
3. Ask owned live agent rosters to shut down where a parent-mediated graceful path is available.
4. Wait only within a bounded grace period.
5. Terminate remaining TUI-owned parent agent processes tracked by TUI child watches.
6. Exit the TUI.

`/resume` restores from durable state, not from a requirement that old processes still exist.

Restored roster rows are classified by liveness:

- **Live reattach:** a persisted entry whose socket/process is verifiably live and still owns the expected session may be restored as live and sendable.
- **Historical non-live:** an entry that was killed by ordinary exit, already dead, unreachable, or previously detached-but-gone is restored visibly as historical/non-live. It preserves identity, display metadata, last known status, parent/read-only/backend/environment metadata, and relevant tool/error display state when available.
- **Drop invalid:** malformed rows without sufficient stable identity may be ignored for compatibility and safety.

Historical non-live rows are not sendable, not running, not counted as active work, and never reattach through stale sockets. Selecting or sending to them must produce a stable non-live/undeliverable UX.

## Consequences

- Ordinary TUI exit no longer leaks TUI-owned `quecto agent` processes.
- `/resume` is a durability feature, not a live-process preservation feature.
- Agent process death is acceptable because conversations, workspace tab layout, and roster identity are recoverable from durable state.
- The TUI must keep liveness separate from activity/status. A row can preserve last known `running` or `idle` display context while being classified as historical/non-live for routing.
- Roster persistence must happen before teardown. Incidental final manifest flushes are not sufficient unless they explicitly include the full visible roster and complete before any process termination.
- Existing behavior for unrelated/global agents is unchanged. The TUI may terminate only processes it owns or can prove belong to its owned roster.
- Closing an individual tab remains the explicit terminate action for that tab's agent unless a future ADR changes tab-close semantics.
- The process-per-tab topology from ADR-0023 remains in force.

## Alternatives considered

- **Keep ADR-0023 detach-on-exit.** Rejected for ordinary TUI exit because it leaks idle TUI-owned agents and subagents over time.
- **Make `/resume` require old sockets.** Rejected because it makes resource cleanup incompatible with resume and fails when processes crash or are killed externally.
- **Silently drop killed roster rows on resume.** Rejected because it loses user-visible roster context and contradicts the desired `/resume` UX in #1586.
- **Restart killed subagents automatically.** Rejected for now. Restarting could consume resources or resume work unexpectedly. Historical rows may support explicit future restart, but ordinary `/resume` must not make killed rows live.
- **Kill all discovered agents.** Rejected. Ordinary TUI exit is scoped to TUI-owned parent agents and their owned rosters only.

## Lifecycle correction — #1608

This correction supersedes the historical-roster restoration decision above and
its workspace/tab-layout assumptions. Ordinary **killing** exit preserves session
transcripts, not stopped children in the operational agent registry. Historical
conversation messages (including spawn/tool results and child session histories)
remain in session storage; neither transcript loading nor history browsing depends
on recreating a dead `SubagentEntry`.

The supported contracts are distinct:

| Lifecycle | Operational roster policy |
| --- | --- |
| Ordinary exit of a TUI-owned agent with killing enabled (default) | Persist the transcript with an empty child roster before cleanup. Old `ordinary_tui_exit_stopped` records never synthesize Detached/Idle rows; like ordinary recovery records they require verified survival (old clients used this marker for detach too). |
| `--detach-on-exit`, or exit from an externally attached agent | Persist the ordinary live-recovery snapshot without a killing marker; do not terminate unowned processes. |
| Reconnect to a still-running harness | Use the harness's existing in-memory registry. A client disconnect/reconnect is not a session restore or a reason to clear live children. |
| Recovery after a crash / session resume in a new harness | Restore only records with a reachable socket that proves the expected session identity. Dead, unreachable, explicitly killed and unknown-reason records do not restore. |

Full compact roster replies and authoritative live broadcasts expose operational
membership only. Cursor deltas may still carry terminal notifications to remove a
previously live row; those are not historical full-roster membership. New/restarted
agents enter through normal registration, without historical UI tombstones here.

### Persistence audit

- Removed the ordinary-exit synthetic entry classifier/reconstruction and the
  compact historical-row exception.
- Removed the final-save loader that copied an earlier historical exit-barrier
  roster over the current registry snapshot. A session-local killing latch keeps
  routine and final saves empty after the barrier; explicit detach persistence or
  switching sessions resets that intent without altering the live registry.
- Retained the session roster schema and identity-verification path: supported
  detach and crash recovery still need them. Legacy reason values remain readable
  for safe migration (unverified stopped records and explicitly killed records are rejected).
- Workspace manifests are no longer written by ordinary-exit fan-out. Legacy
  workspace resume and the tab-agent sidecar still have consumers for live attach;
  removing those separately requires changing those supported contracts. They are
  not a source of synthetic harness operational entries.

Cleanup is an independent process-ownership concern. A saved empty roster does
not prove process termination, and a visible historical tool result does not prove
that a child is still alive. Persistence failures and cleanup timeouts remain
reported rather than silently treated as successful exit durability/termination.

## Lifetime correction — #1937 (epic #1929)

This correction supersedes the **live reattach** classification above and the
"Recovery after a crash / session resume in a new harness" row of the #1608
table. Under #1935 a launcher-created child is lifetime-scoped to the harness
that launched it: it is started without `--persist`, ignores ordinary client
churn because it is launch-bound, and runs the common shutdown when its bound
parent connection is lost. A restored session therefore cannot describe a
live child of the restoring harness, and restore no longer tries to find one:

| Lifecycle | Operational roster policy |
| --- | --- |
| Session resume (`resume_session`) or a new harness loading a saved session | Restore the transcript, workflow run and past child messages. Reset the operational roster to empty. Persisted rows of every liveness/reason — live, detached, dead, killed, unknown, malformed — are history only: no socket probe, no pid compare, no monitor, no child row re-created. The master re-spawns needed workers with a fresh identity and launch generation. |
| Reconnect to a still-running harness | Unchanged: the harness's in-memory registry. |
| Top-level `quecto agent --persist` | Unchanged and still supported. |

The session roster schema keeps identity, display, status, parent/read-only
metadata and undelivered report bookkeeping as history; the child's
`socketPath` and `pid` are no longer written and are ignored when read from a
legacy record (migrated on the next save). Restore performs no identity
verification and no process check of any kind. Historical rows are never
synthesised into the operational roster and nothing restarts automatically.

### Session transitions tear the departing session's children down — #1938

Because a launched child belongs to the harness that launched it and to no
later session, a row merely dropped from the
operational roster on `new_session` or `resume_session` would strand a live
child no teardown path can reach until the master exits. The #1937 interim
released each departing row through parent loss (aborting its monitor task
so the child observed the loss of its bound connection). #1938 replaces that
release with the acknowledged, application-owned **fleet teardown**
(`TerminateAllDelegatedAgents`): before the departing session is saved and
its roster replaced, every direct child is claimed, asked to shut down over
its edge, concluded through the supervised owned handle and compensated,
under a concurrency bound; exited tombstones are pruned. The same use case is
the direct-children step of every common shutdown (`shutdown` command,
parent loss, bind deadline, SIGTERM/SIGINT, the last client of the default
lifetime) and of `delete_all_subagents`, so the harness has one owner of
whole-fleet ends. A child that cannot be settled within its budget refuses
the transition explicitly and keeps the current session; the process-exit
paths report it and exit anyway. A spawn racing an admitted shutdown is
either registered before the fleet is claimed (and torn down with it) or
refused, because registration reads the frozen lifecycle inside the
registry's critical section.

## Final model — #1940 (epic #1929 closed)

The epic closes with one teardown model, ratcheted by
`tests/architecture/teardown_authority.rs`:

- A harness owns only the children it launched, as handles held by the
  `OwnedChildSupervisor`; every other row — a merged grandchild, a restored
  record, a script or container member, a swarm member — is ended over the
  protocol (`shutdown`, `terminate_delegated_agent` routed one edge at a
  time) and its end is observed, never forced. A merged descendant is stored
  with its uuid, launch generation and parent, and **no pid**.
- A launched child is lifetime-bound to its launcher: loss of the bound
  parent control connection, or the bind deadline, runs its own common
  shutdown, so a subtree falls with any ancestor without pid knowledge above.
- Restore is history only plus explicit re-spawn; the retained swarm
  environment on spontaneous coordinator loss (#1924) is the one exception
  to "environments end with their members", and only `kill_container` ends it.
- The process effects that remain are enumerated by file: the supervisor's
  owned-handle TERM/KILL, bash invocation containment, the Python
  `ExecutionScope` job containment, signal-0 observation, the
  retained-environment command adapter and tool-child containment. Nothing
  else in the harness crate may signal, and no `/proc` traversal grants
  authority.
- Kill latency: ACK 5 s, exit after ACK 10 s, TERM 2 s, KILL 2 s (≈ 19 s
  worst case for an owned child); 25 s compensation wait for an unowned
  child; 30 s per remaining hop for a nested target; 45 s before a repeated
  OS signal forces the harness out.

**TUI ordinary exit — #1956.** The TUI's ordinary exit (`Ctrl+D`, `/quit`,
`/exit`) and its other owned-harness ends (tab close, `/new` workspace reset,
startup-failure cleanup) signal the harness **leader only**: after the
snapshots are persisted, SIGTERM to that one pid (`kill(pid)`, never
`kill(-pgid)`, never a descendant); a wait for that process to exit within a
*settle* budget derived from the fleet teardown above — `ceil(n / 8)` batches
(`DEFAULT_SETTLEMENT_BOUND`) × 25 s (`DEFAULT_COMPENSATION_WAIT`) + 5 s
persist slack for the `n` subagents the tab's roster last showed, capped at
the 3-pass (`MAX_PASSES`) worst case of 80 s and used in full when the roster
is unknown; then a **second** SIGTERM — a repeated signal is what arms the
harness's own 45 s `FORCE_EXIT_AFTER`, a single one never does — and a wait of
those 45 s; and only then SIGKILL of that one pid. A "waiting for the agent to
settle its subagents…" notice appears past ~1 s. The former process-group
SIGTERM, `/proc` descendant sweep, 1.5 s grace and "leader exit is not proof
of cleanup" verifier are gone; in their place a read-only post-exit canary
reads `/proc` once and reports — never signals — any process still naming the
old leader as parent or process group, skipped when the pid's kernel start
time shows it has been recycled. Under the lifetime binding that report is
always empty; a non-empty one is the evidence. Swarm members and bash tool
children in their own process group are outside its view by design. The
harness's fleet teardown on SIGTERM is thus the only subagent-ending
authority, with the TUI a plain SIGTERM sender whose waits are the harness's
own numbers.
