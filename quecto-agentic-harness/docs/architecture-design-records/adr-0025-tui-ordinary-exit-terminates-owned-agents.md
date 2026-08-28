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
