# ADR-0012 — Explicit Agent Turn State Machine

**Status:** Proposed.

**Implementation status:** Not started.

## Context

`quecto-agentic-harness` is intentionally the kernel for the agent loop: it owns
provider calls, tool execution, streaming progress, audit events, context
pruning/spilling, usage accounting, session reconciliation, cancellation, and
final turn results.

The current implementation is well tested and already decomposes some concerns
into helper modules, but the main loop still acts as a broad coordinator. The
important behavioural states of a turn are partly implicit in control flow and
message mutations rather than named as a first-class application concept. That
makes future changes risky because features such as streaming, malformed model
recovery, tool retries, context collapse, cancellation, and durable persistence
all interact during one turn.

The risk is not that the current code lacks tests; it is that the mental model
for a turn is spread across the loop, helper modules, progress events, audit
writes, and persistence side effects.

## Decision

Represent agent turn execution as an explicit application-level state machine.

The agent loop remains the top-level coordinator, but it should advance through
named turn states and delegate each state transition to focused components. The
state machine should be introduced incrementally behind the existing public
`AgentLoop` port.

The target conceptual model is:

```text
PrepareTurn
  -> BuildProviderRequest
  -> AwaitProviderResponse | StreamProviderResponse
  -> ClassifyProviderResponse
  -> ExecuteToolCalls
  -> AppendToolResults
  -> RetryWithModelFeedback
  -> FinalizeAssistantResponse
  -> ReconcilePersistenceAndUsage
  -> CompleteTurn
```

The exact Rust names may differ, but each transition should have a clear owner,
inputs, outputs, and tests. The state machine is an internal application detail;
it does not change provider adapters, tool implementations, UDS protocol shapes,
or persisted session format by itself.

## Consequences

- The main `AgentLoopImpl::process` path should become easier to read as a
  sequence of turn-state transitions.
- Tests can target specific transition invariants instead of only full-loop
  behaviour.
- New features should identify which turn state they extend, reducing accidental
  cross-cutting changes.
- There is short-term duplication risk while extracting states from existing
  logic; implementation should use small behaviour-preserving refactors.
- Public compatibility is preserved because the domain `AgentLoop` trait and
  observable events remain stable unless changed by a later ADR.

## Alternatives considered

- **Leave the loop as the only state machine.** Rejected: the implicit state
  machine already exists, but only as control flow. Naming it lowers change risk.
- **Rewrite the entire loop around a new enum in one change.** Rejected: too much
  risk for a critical subsystem. This must be phased and test-preserving.
- **Move turn orchestration into domain.** Rejected: provider/tool execution,
  audit, cancellation, and persistence coordination are application concerns;
  the domain should retain vocabulary and ports, not runtime orchestration.
