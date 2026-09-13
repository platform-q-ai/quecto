# ADR-0015 — Subagent Lifecycle Is an Explicit State Machine

**Status:** Proposed.

**Implementation status:** Not started.

## Context

Subagents are a defining capability of the harness. A parent agent can spawn a
child harness process, send it prompts, steer or follow up, observe passive completion,
query state, retrieve messages, receive passive completion notes, and forward
selected UDS commands to the child.

This lifecycle crosses several boundaries: tool execution, process launch,
UDS readiness, session identity, command forwarding, history retrieval,
monitoring, cancellation, completion coalescing, and parent notification.

The implementation has many targeted tests, but lifecycle state is still easy
to reason about incorrectly because process state, child UDS state, parent
registry state, and user-visible state are related but not identical.

## Decision

Model subagent lifecycle as an explicit state machine and make lifecycle events
first-class application/infrastructure vocabulary.

The target lifecycle states are:

```rust
enum SubagentState {
    Launching,
    SocketReady,
    Idle,
    Busy,
    AwaitingCompletion,
    Exited,
    Failed,
    Killed,
}
```

The exact enum names and layering may differ, but the implementation should make
state transitions explicit and testable. Representative lifecycle events:

```rust
enum SubagentLifecycleEvent {
    SpawnRequested,
    ProcessStarted,
    SocketDiscovered,
    InitialPromptSent,
    TurnStarted,
    TurnEnded,
    CompletionNoted,
    Exited,
    Killed,
    Failed,
}
```

The state machine should distinguish:

- process lifecycle from agent-run lifecycle;
- parent registry metadata from child-reported state;
- passive completion notes from transcript inspection results;
- local parent history from child history resolved over forwarded commands.

This decision does not require changing the public subagent tool schema or UDS
wire events immediately. Public shape changes require separate protocol work.

## Consequences

- Race-sensitive behaviour such as completion notes, kill, abort, and
  child message retrieval becomes easier to specify.
- Tests can assert legal transitions and idempotency/coalescing rules.
- Parent-facing status can be derived from lifecycle state rather than scattered
  booleans and snapshots.
- Some infrastructure code may need adapters to report events into the lifecycle
  model.
- The state machine must preserve current compatibility for existing subagent
  commands and TUI/API consumers.

## Alternatives considered

- **Keep lifecycle implicit in monitor/registry code.** Rejected: the number of
  lifecycle edges already justifies a named model.
- **Use OS process state as the lifecycle.** Rejected: an alive process can be
  busy, idle, socket-not-ready, or unreachable; process state is necessary but
  insufficient.
- **Push subagent orchestration into an external tool.** Rejected for this scope:
  ADR-0006 keeps larger taskgraph orchestration external, but the kernel owns
  the composable unit contract and child-agent lifecycle semantics.

## Delta — cross-process liveness dimension (#1460 / epic #1467)

ADR-0023 makes the TUI a multiplexer of replicant agent *processes* that
outlive it (`--persist` detach). That adds a liveness dimension **orthogonal**
to the in-process lifecycle states above:

```text
live      — the process answers a connect probe on its socket
detached  — the process is presumed running but no client is attached
dead      — the socket refuses (or the stamped pid is gone); safe to reap
```

`SubagentState` describes what an agent is doing; the liveness dimension
describes whether anyone can still reach it. A `Busy` agent can be `detached`
(TUI closed), and an `Exited` one leaves a `dead` socket behind.

The roster of known agents (pid + socket registry sidecar) is persisted in
the session store so a restarted TUI can re-derive `live | detached | dead`
by probing, rather than trusting stale files. Tracked by #1461.

### Lifetime correction — #1937 (epic #1929)

The paragraph above no longer describes the launcher's children. Under #1935
a launcher-created subagent is **launch-bound**: it is started without
`--persist`, ignores client churn, and runs the common shutdown when its
bound parent connection is lost, so it cannot outlive the harness that
launched it. The persisted roster is therefore history only: it carries no
`socketPath` or `pid` (a legacy record's are ignored when read and dropped on
the next save), and a restarting harness probes nothing and readopts nothing
— it resets the operational roster to empty and the master re-spawns the
workers it needs with a fresh identity and launch generation. On a session
transition the departing session's live children are released through parent
loss (interim until #1938). The `live | detached | dead` liveness dimension
survives only for a **top-level** `quecto agent --persist` process, which the
TUI still multiplexes and reattaches to; it is never re-derived for a
launcher's children. See ADR-0025's #1937 correction for the roster policy.
