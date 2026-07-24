# ADR-0015 — Subagent Lifecycle Is an Explicit State Machine

**Status:** Proposed.

**Implementation status:** Not started.

## Context

Subagents are a defining capability of the harness. A parent agent can spawn a
child harness process, send it prompts, steer or follow up, await completion,
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
    AwaitStarted,
    AwaitTimedOut,
    Exited,
    Killed,
    Failed,
}
```

The state machine should distinguish:

- process lifecycle from agent-run lifecycle;
- parent registry metadata from child-reported state;
- passive completion notes from explicit `await` results;
- local parent history from child history resolved over forwarded commands.

This decision does not require changing the public subagent tool schema or UDS
wire events immediately. Public shape changes require separate protocol work.

## Consequences

- Race-sensitive behaviour such as completion notes, `await`, kill, abort, and
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
