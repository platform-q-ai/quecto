# ADR-0014 — Context Management Is a First-Class Application Subsystem

**Status:** Proposed.

**Implementation status:** Not started.

## Context

Context management is central to the harness. It determines what messages are
sent to providers, how large tool results are collapsed, when history is spilled
or recalled, how provider-reported token truth is reconciled with local
estimates, and when durable persisted history must be reconciled after pruning.

Today these behaviours are covered by tests and distributed across several
modules. That distribution reflects real complexity, but the subsystem's
invariants are more important than any single implementation file:

- recent turns may be pinned;
- tool call/result relationships must remain coherent;
- collapsed or spilled content must remain recoverable where promised;
- local token estimates and provider-truth gauges must not be confused;
- pruning that mutates persisted history must mark the durable prefix dirty;
- frame-size workarounds should not leak into application-level context policy.

As context windows, model metadata, message paging, subagents, and recovery
surfaces evolve, context management needs a clearer boundary.

## Decision

Treat context management as a named application subsystem with explicit policy,
planning, application, and reconciliation phases.

The target shape is an internal module namespace such as:

```text
application/context/
  budget.rs
  estimate.rs
  gauge.rs
  plan.rs
  pruning.rs
  spill.rs
  collapse.rs
  durable_prefix.rs
```

The exact layout may differ, but callers should interact with higher-level
operations rather than directly coordinating scattered pruning/spill/gauge
functions:

```rust
let plan = context_manager.plan_before_provider_call(messages, model_context);
context_manager.apply_plan(messages, plan).await?;
context_manager.reconcile_after_provider_usage(messages, usage);
context_manager.reconcile_after_turn(messages, turn_outcome).await?;
```

Context management remains in the application layer. Persistence stores and
spill stores remain ports/adapters. Provider-specific token reporting remains
behind provider/domain types.

## Consequences

- Context-related invariants become easier to document and test in one place.
- The agent loop can delegate context policy rather than mixing it into turn
  orchestration.
- Future work such as paged single-message recovery, provider-specific context
  windows, and durable prefix reconciliation has a natural home.
- Implementation must avoid changing observable pruning behaviour accidentally;
  extract existing logic behind regression tests before changing policy.
- This ADR does not change wire protocol payloads or session file formats.

## Alternatives considered

- **Keep context helpers as agent-loop internals.** Rejected: context policy is a
  core application concern and already large enough to merit its own boundary.
- **Move context management to infrastructure.** Rejected: infrastructure stores
  bytes and files; it should not own provider-message selection policy.
- **Move context management to domain.** Rejected: domain should define messages,
  errors, and ports. Runtime budgeting, spilling, and reconciliation need
  application services and async ports.
