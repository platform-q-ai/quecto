# ADR-0027 — The TUI Is a Single-Connection Client

**Status:** Accepted.

**Supersedes:** ADR-0023 and ADR-0025 where they assume a process-per-tab
multiplexer, tab-agent registry, or workspace manifest. ADR-0025's corrected
ordinary-exit teardown semantics remain in force for the one TUI-owned agent.

## Context

ADR-0023 designed the TUI as a multiplexer: each tab would own an agent process
and connection, with a registry and workspace manifest providing discovery and
restore. The shipped product never exposed a way to create another tab. That
left substantial routing, persistence, and lifecycle machinery serving an
unreachable topology.

The first part of #2044 removed tab switching, the workspace registry and
manifest restore, and tab open/close lifecycle. The application now has one
connection state. Per-tab request-id prefixes do not establish ownership in a
single-connection client; exact, process-unique pending request ids do.

ADR-0025 corrected ordinary exit so the TUI asks its owned harness to perform
its authoritative fleet teardown. That behavior is valuable independently of
multi-tab presentation and must remain.

## Decision

`quecto-tui` is a single-connection client. One TUI instance owns one agent
connection and presents one session at a time.

- The TUI has no tab multiplexer, tab-agent registry, or workspace manifest.
  `/new` and `/resume` change the session through the existing connection.
- Request ids are opaque and carry no tab namespace; the harness continues to
  treat them as opaque. A request whose answer belongs to the asking client
  (history and resume fetches, searches, the exit persist) is minted
  process-unique and matched by exact pending-id equality. Requests for shared
  harness state (model, effort, stats, roster refresh) keep fixed ids on
  purpose: every attached client applies the same answer from one ordered
  stream, so no correlation machinery is added for them.
- Ctrl-D, `/exit`, and `/quit` retain the common ordinary-exit path established
  by ADR-0025's later lifecycle corrections: persist the current state, ask the
  one owned harness leader to shut down, and use its bounded settlement
  protocol. Externally attached agents remain unowned and are not terminated.
- Subagent focus and roster presentation remain views over the connected
  harness. They do not create additional top-level TUI connections.

## Consequences

- Connection ownership and response routing have one authority instead of a
  dormant tab layer.
- Multiple independent TUI processes may attach to a multi-client harness.
  Their process-unique request ids prevent one client from consuming another's
  solicited response; broadcast events remain broadcast by protocol design.
- Session resume preserves the single-session behavior and does not restore a
  workspace of tabs.
- Users who need concurrent top-level views run independent TUI instances.
- ADR-0023 remains historical evidence for the rejected topology. ADR-0025
  remains historical evidence for the exit decision and its corrections; only
  its tab/manifest framing is superseded.

## Alternatives considered

- **Retain dormant multi-tab abstractions.** Rejected: they increase routing and
  lifecycle complexity without a reachable user behavior.
- **Keep a tab prefix on request ids.** Rejected: it is not an ownership
  boundary across processes, whereas process-unique ids plus exact pending-id
  matching are.
- **Discard ADR-0025 ordinary-exit behavior with the tabs.** Rejected: the TUI
  still owns one harness, and orderly bounded teardown prevents leaked agents.
