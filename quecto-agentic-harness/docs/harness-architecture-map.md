# Harness Architecture Map

This map gives contributors a short orientation to the main
`quecto-agentic-harness` orchestration surfaces. It is intentionally descriptive:
Phase 0 of the architecture-hardening PRD does not change runtime behaviour.

For the hardening plan, see
[PRD: Agentic Harness Architecture Hardening](prd-harness-architecture-hardening.md).
For the related decisions, see the [ADR index](architecture-design-records/README.md).

## Turn execution

**Primary code:** `src/application/agent_loop.rs` and sibling modules such as
`agent_loop_pruning.rs`, `agent_loop_spill.rs`, `agent_loop_tools.rs`,
`agent_loop_model_limits.rs`, and `agent_loop_context_gauge.rs`.

`AgentLoopImpl` is the application coordinator for a user turn. It receives the
conversation request through the `AgentLoop` port, prepares provider messages,
applies context-budget decisions, invokes the configured provider, executes tool
calls, records assistant/tool messages, emits progress/audit events, and updates
usage/context gauges.

Important invariants before Phase 2:

- the public `AgentLoop` port remains the external application boundary;
- provider wire protocols stay in infrastructure;
- tool execution is mediated by tool-registry ports rather than provider code;
- context pruning/spill decisions preserve tool-call/tool-result coherence; and
- cancellation/abort handling must leave session history consistent with what
  clients have observed.

## Context management

**Primary code:** `src/application/context_pruning.rs`,
`context_pruning_messages.rs`, `agent_loop_pruning.rs`, `agent_loop_spill.rs`,
`agent_loop_context_gauge.rs`, and related context/spill tests.

Context management already exists as a conceptual subsystem. It plans what can
be sent to a provider within the model budget, protects pinned recent turns,
keeps tool calls coherent with their tool results, spills older content when
needed, reconciles local estimates with provider-reported token truth, and marks
durable prefixes dirty when session state changes.

Important invariants before Phase 1:

- recent user/assistant/tool turns that must remain visible are pinned;
- tool-call and tool-result messages are never separated by pruning;
- collapsed/spilled content remains recoverable through documented recall paths;
- provider-truth token observations supersede local estimates when available;
  and
- dirty-prefix tracking is durable-session bookkeeping, not a pruning policy.

## UDS dispatch

**Primary code:** `src/interface/cli/uds.rs`, `uds_dispatch.rs`,
`uds_reader.rs`, `uds_responses.rs`, `protocol.rs`, and UDS regression tests.

UDS mode is the stable protocol boundary for TUI/API clients, subagents, and
external automation. The top-level dispatcher accepts framed JSON commands,
preserves correlation ids, selects local handling or subagent forwarding, emits
bounded responses/events, and coordinates broadcast versus direct writer output.

Important invariants before Phase 3:

- length-prefixed JSON frames are the protocol baseline;
- legacy newline-delimited JSON is accepted only for compatibility;
- response events echo command correlation ids when provided;
- command ordering and single-run semantics are preserved;
- bounded end-of-turn events carry `messageRefs` rather than full histories; and
- child-targeted history/sync commands are answered by the child, not by the
  parent session.

See also the [UDS protocol reference](uds-protocol.md) and
[protocol capability matrix](protocol-capability-matrix.md).

## Subagent lifecycle

**Primary code:** `src/infrastructure/tools/spawn.rs`,
`spawn_binary.rs`, `subagent_registry.rs`, `subagent_monitor.rs`,
`subagent_monitor_stall.rs`, `subagent_monitor_merge.rs`,
`subagent_await_result.rs`, and `agent_cmd.rs`.

Subagents are spawned harness processes supervised by the parent. The parent
tracks process launch, socket connection/readiness, forwarded child events,
message-history retrieval, busy/idle snapshots, terminal exit/failure, passive
completion notifications, and explicit `agent_cmd await` calls.

Important invariants before Phase 4:

- spawning returns quickly while monitoring continues asynchronously;
- child commands are routed over the child's UDS socket when possible;
- passive completion notes are coalesced and do not duplicate explicit awaits;
- `get_subagents` reports enough identity/state to rebuild the unit tree; and
- exited children remain inspectable long enough for result recovery.

## Persistence and session recovery

**Primary code:** session ports in `src/domain`/`src/application`, persistence
adapters in `src/infrastructure`, and UDS/session recovery paths in
`src/interface/cli`.

Session persistence stores conversation messages, tool-call identity, durable
context bookkeeping, workflow state, and enough metadata to resume or inspect a
session after reconnect/restart. UDS clients use paged history and stable message
ids to re-sync after disconnects or dropped broadcast events.

Important invariants before later phases:

- no Phase 0 hardening changes the session file format;
- stable message/tool ids remain string-compatible at serialization boundaries;
- paged `get_messages` is the supported re-sync path, not unbounded history;
- `get_message` resolves stable ids, including spilled/collapsed content where
  supported; and
- workflow state is session metadata and must recover to a valid template/mode
  if the configured workflow library changes.

## Baseline subsystem checks

Use these focused checks while hardening the architecture. The full pre-push gate
remains authoritative before a PR is handed off.

| Subsystem | Focused check |
|---|---|
| Repo docs / Phase 0 links | `cargo test -p quecto-agentic-harness --test repo_docs` |
| Architecture boundaries | `cargo test -p quecto-agentic-harness --test architecture` |
| Context management | `cargo test -p quecto-agentic-harness --lib context_pruning` |
| Agent loop | `cargo test -p quecto-agentic-harness --lib agent_loop` |
| UDS protocol/dispatch | `cargo test -p quecto-agentic-harness --lib uds` |
| Subagent lifecycle | `cargo test -p quecto-agentic-harness --lib subagent` |
| Workflow/session recovery | `cargo test -p quecto-agentic-harness --lib workflow` |

## Baseline longest files

Generated on 2026-07-25 from `*.rs` under `quecto-agentic-harness` with a simple
line count. The production source cap remains 750 lines; large BDD step files are
listed because they are important hardening hotspots even when exempt from that
cap.

| Lines | File |
|---:|---|
| 4150 | `tests/bdd/uds_steps.rs` |
| 4062 | `tests/bdd/provider_steps.rs` |
| 3755 | `tests/bdd/e2e_steps.rs` |
| 2240 | `tests/bdd/context_pruning_steps.rs` |
| 1844 | `tests/bdd/uds_bounded_events_steps.rs` |
| 1468 | `tests/bdd/main.rs` |
| 1380 | `tests/bdd/auth_steps.rs` |
| 1227 | `tests/bdd/tui_architecture_steps.rs` |
| 1099 | `tests/workflow_config_template.rs` |
| 1032 | `tests/bdd/uds_paged_history_steps.rs` |
| 894 | `tests/architecture.rs` |
| 859 | `tests/bdd/subagent_monitor_steps.rs` |
| 831 | `tests/bdd/repl_steps.rs` |
| 803 | `tests/bdd/agent_cmd_tool_steps.rs` |
| 763 | `tests/bdd/audit_log_steps.rs` |

Current largest production/library files are at the 750-line cap, including
`src/interface/cli/uds_snapshots.rs`, `src/interface/cli/uds_dispatch.rs`,
`src/interface/cli/uds.rs`, `src/interface/cli/protocol.rs`,
`src/infrastructure/tools/subagent_monitor.rs`, and
`src/application/agent_loop.rs`.
