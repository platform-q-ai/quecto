# ADR-0013 — UDS Command Dispatch Uses Command-Family Routers

**Status:** Proposed.

**Implementation status:** Not started.

## Context

The UDS interface is the harness's primary integration boundary for the TUI,
API gateway, subagents, external clients, and extension-related workflows. It
has grown to cover prompting, steering, history retrieval, message paging,
subagent forwarding, model/effort changes, sessions, workflow controls,
extensions, and lifecycle operations.

The current dispatcher is correct but increasingly central. Special cases such
as child-targeted history/message/sync forwarding must run before local fast
paths, and many command variants share concerns such as correlation ids,
response framing, broadcast behaviour, busy handling, and frame-size guards.

A single dispatcher file is therefore becoming a feature accretion point.
Adding a command requires understanding unrelated command families and the
ordering of special cases.

## Decision

Split UDS command handling into command-family routers while preserving the
existing wire protocol.

The top-level dispatcher should become a small orchestration layer:

1. parse/receive an `AgentCommand`,
2. run pre-routing rules that must happen before local handling, especially
   subagent forwarding,
3. route the command to a focused command-family handler,
4. emit the resulting event/response through the existing framing and broadcast
   helpers.

Target command families:

- conversation: `prompt`, `steer`, `follow_up`, `abort`, cancellation-sensitive
  controls;
- history: `get_messages`, `get_messages_tail`, `get_message`, paging,
  rewind/resume history operations;
- subagents: spawn, `agent_cmd`, lifecycle queries, forwarded child commands;
- session: clear history, session state, stats, resume/persistence controls;
- model/runtime: set model, set effort, model/provider reload surfaces;
- workflow: workflow automation, nudges, workflow-state commands;
- extensions: extension listing/reload/registration commands;
- lifecycle/query: health/state/fieldless low-cost queries.

The refactor is internal. Existing command names, JSON shapes, response shapes,
correlation-id behaviour, and legacy compatibility rules remain unchanged unless
a later ADR explicitly changes them.

## Consequences

- Each command family can have narrower tests and fixtures.
- Subagent forwarding rules become explicit pre-routing behaviour rather than
  ad hoc cases embedded in the main match.
- Adding a new UDS command should require choosing a family and updating only
  that family plus protocol tests.
- The top-level dispatcher remains responsible for cross-cutting invariants:
  frame limits, correlation ids, broadcast/writer selection, and command-family
  routing.
- There is some risk of over-fragmentation; command families should be split by
  behaviour and invariants, not one file per trivial command.

## Alternatives considered

- **Keep a single match forever.** Rejected: simple initially, but the protocol
  already has enough families and forwarding rules to justify routing structure.
- **Build a dynamic command-handler registry.** Rejected for now: Rust enum
  matching is clear, fast, and compile-time checked. A static family router is
  enough.
- **Move UDS command semantics into application.** Rejected: UDS command shapes,
  correlation ids, and protocol compatibility are interface concerns. The
  application layer should expose use cases/ports, not wire protocol details.
