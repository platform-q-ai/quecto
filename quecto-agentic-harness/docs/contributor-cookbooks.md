# Contributor Cookbooks

These cookbooks map common `quecto-agentic-harness` changes to the production
files, tests, docs, and compatibility checks that normally matter. They are a
starting point, not a substitute for reading the local code and preserving the
Clean Architecture dependency rule.

For a subsystem overview, start with the
[Harness Architecture Map](harness-architecture-map.md). For the hardening plan,
see the [architecture-hardening PRD](prd-harness-architecture-hardening.md) and
[ADR-0018](architecture-design-records/adr-0018-contributor-change-cookbooks.md).

## Local check command index

Run the narrowest check that exercises the touched subsystem before committing.
The pre-push hook remains the fast local integration gate, and authoritative CI
runs after `merge-requested` is applied.

| Subsystem | Focused local command |
|---|---|
| Agent loop | `cargo test -p quecto-agentic-harness --lib agent_loop` |
| Context management | `cargo test -p quecto-agentic-harness --lib context_pruning` |
| UDS protocol/dispatch | `cargo test -p quecto-agentic-harness --lib uds` |
| Subagents | `cargo test -p quecto-agentic-harness --lib subagent` |
| Protocol docs / repo docs | `cargo test -p quecto-agentic-harness --test repo_docs` |
| Architecture boundaries | `cargo test -p quecto-agentic-harness --test architecture` |
| Domain/application contracts | `cargo test -p quecto-agentic-harness --test contracts` |
| Workflow configuration/docs | `cargo test -p quecto-agentic-harness --test workflow_config_template` and `cargo test -p quecto-agentic-harness --test workflow_docs` |

Use BDD tags or shards only when the change touches scenario-level behaviour.
Do not run live provider lanes unless the task explicitly requires them.

## Add a built-in tool

**Start in:** `domain::tool` contracts and infrastructure tool adapters.

**Production files usually involved:**

- `src/domain/tool.rs` for trait or schema vocabulary changes, if needed.
- `src/infrastructure/tools/<tool>.rs` or `src/infrastructure/tools/<tool>/` for
  the concrete tool.
- `src/infrastructure/tools/mod.rs` and `src/infrastructure/tools/registry.rs`
  for registration and guard composition.
- `src/interface/cli/agent/agent_tool_registry.rs` when CLI/agent construction
  needs to expose the tool.
- `src/infrastructure/security/sandbox.rs` for path or command safety rules.

**Tests to add/update:**

- Tool unit tests beside the implementation, for example
  `src/infrastructure/tools/<tool>_tests.rs`.
- Registry/schema tests when the tool definition changes.
- BDD feature coverage under `tests/features/*_tool.feature` only for
  user-visible tool behaviour.
- Architecture tests if a new module risks crossing layer boundaries.

**Docs and compatibility:**

- Update `README.md`, `docs/extensions.md`, or `docs/disable-tools.md` when the
  user-facing tool surface changes.
- Keep tool names stable once shipped; changing a name can break prompts,
  extensions, and recorded sessions.
- Avoid production code paths that exist only for tests. Prefer injected test
  fixtures or existing `test_support` helpers.

**Common pitfalls:**

- Do not let infrastructure tools import application/interface modules.
- Do not bypass `Sandbox::validate_path` for filesystem access.
- Keep `bash`-like behaviour explicit: command execution is not a filesystem
  sandbox.

## Add or change a UDS command

**Start in:** the protocol type, then route through the UDS dispatcher family
that owns the command.

**Production files usually involved:**

- `src/interface/cli/protocol.rs` for `AgentCommand` and `AgentEvent` shapes.
- `src/interface/cli/uds_dispatch.rs` and focused dispatch modules such as
  `uds_dispatch_query.rs`, `uds_dispatch_session.rs`, `uds_dispatch_runtime.rs`,
  `uds_dispatch_forwarding.rs`, or `uds_dispatch_sync_forward.rs`.
- `src/interface/cli/uds_responses.rs`, `uds_session.rs`, and `uds_snapshots.rs`
  when response or snapshot JSON changes.
- `src/infrastructure/tools/agent_cmd.rs` when the spawned-agent tool exposes
  the command.

**Tests to add/update:**

- Protocol shape tests in `src/interface/cli/protocol*_tests.rs`.
- Dispatcher family tests next to the touched module.
- Regression tests for correlation ids, bounded responses, and child-targeted
  forwarding when relevant.
- BDD under `tests/features/uds_*.feature` for client-observable behaviour.

**Docs and compatibility:**

- Update `docs/uds-protocol.md` and, for protocol-affecting work,
  `docs/protocol-capability-matrix.md`.
- Keep JSON wire shapes string-compatible unless a separate ADR/PRD approves a
  breaking change.
- Preserve legacy aliases during documented deprecation windows.

**Common pitfalls:**

- Child-targeted history/sync commands must be handled by the forwarding
  pre-router and must not fall through to parent-local history.
- Always echo request correlation ids in direct responses.
- Keep frame-size and bounded-event invariants in mind; do not reintroduce
  unbounded history payloads.

## Add provider/model runtime capability

**Start in:** provider configuration/model registry, then provider adapters.

**Production files usually involved:**

- `src/infrastructure/model_registry.rs` and `src/infrastructure/config.rs` for
  model metadata, effort levels, or config defaults.
- `src/infrastructure/providers/*` for provider-specific request/response
  handling.
- `src/domain/provider.rs` only for provider-agnostic vocabulary that the
  application genuinely needs.
- `src/interface/cli/models.rs`, `agent_provider.rs`, or UDS runtime/model
  dispatch modules for user-facing selection/reload behaviour.

**Tests to add/update:**

- Provider unit tests and parser tests beside the adapter.
- Config/model registry tests for discovery, defaults, pricing, effort, or
  validation behaviour.
- UDS/model tests if runtime selection or hot reload changes.
- Provider smoke BDD only for tiny credential/provider-availability checks.

**Docs and compatibility:**

- Update `docs/runtime-models-providers.md` and embedded capability docs when
  user configuration changes.
- Do not print API keys, OAuth tokens, or raw credential payloads in failures.
- Keep provider wire protocols in infrastructure; domain/application should use
  provider-neutral request/response vocabulary.

**Common pitfalls:**

- Validate custom provider URLs and keep loopback/test exceptions explicit.
- Treat effort vocabulary as provider-specific and validate it before use.
- Keep live-provider tests opt-in and deterministic unit coverage local.

## Add a progress or audit event

**Start in:** domain event vocabulary for application events, then interface
serialization for UDS-visible events.

**Production files usually involved:**

- `src/domain/agent.rs` for `AgentProgressEvent` changes.
- `src/application/agent_loop*.rs` where progress is emitted.
- `src/interface/cli/uds_cancel.rs`, `uds_snapshots.rs`, or `protocol.rs` for
  UDS event conversion.
- `src/infrastructure/logging.rs` or audit-specific modules for persistent audit
  behaviour.

**Tests to add/update:**

- Agent-loop unit tests for emission timing and cancellation paths.
- UDS event-shape tests for broadcast/direct-client behaviour.
- BDD under `tests/features/observability.feature`, `audit_log.feature`, or
  related UDS features when clients observe the event.

**Docs and compatibility:**

- Update `docs/uds-protocol.md` for new UDS events or fields.
- Add matrix notes when event compatibility or client recovery semantics change.
- Keep existing event names and fields stable; prefer additive fields.

**Common pitfalls:**

- Do not leak secrets in event payloads.
- Do not emit high-volume unbounded content; use references, summaries, or
  bounded previews.
- Ensure cancellation/abort does not leave clients with impossible state.

## Change session persistence safely

**Start in:** domain session vocabulary, then persistence adapters and recovery
paths.

**Production files usually involved:**

- `src/domain/session.rs` and `src/domain/message.rs` for persisted concepts.
- `src/infrastructure/persistence/*` for JSON file serialization.
- `src/application/reload.rs`, context modules, or agent loop finalization when
  persistence state is updated.
- `src/interface/cli/uds_session*.rs` for paged history, snapshots, and resume.

**Tests to add/update:**

- Persistence round-trip tests for old and new fields.
- Session/reload tests for recovery behaviour.
- UDS paged-history or resume tests when clients observe the field.
- Repo-doc or protocol tests if documented session contracts change.

**Docs and compatibility:**

- Update `docs/sessions.md` and `docs/uds-protocol.md` for observable changes.
- Preserve existing session file compatibility. New fields should be optional or
  defaultable unless a separate migration plan is approved.
- Stable message ids and tool-call ids must remain string-compatible at JSON
  boundaries.

**Common pitfalls:**

- Do not persist provider-specific wire payloads in domain types unless they are
  already part of the domain contract.
- Keep tool-call/tool-result pairs coherent across pruning, reload, and resume.
- Avoid unbounded history reads; use paged history and `get_message` recovery.

## Add subagent behaviour

**Start in:** subagent lifecycle/registry/monitor code, then the UDS forwarding
or `agent_cmd` surface that exposes it.

**Production files usually involved:**

- `src/domain/subagent.rs` for shared validation or vocabulary.
- `src/infrastructure/tools/spawn*.rs`, `subagent_registry.rs`,
  `subagent_monitor*.rs`, `subagent_lifecycle.rs`, and
  `subagent_await_result.rs` for process/run state.
- `src/infrastructure/tools/agent_cmd*.rs` for parent-to-child commands.
- `src/interface/cli/uds_dispatch_forwarding.rs`,
  `uds_dispatch_get_message_forward.rs`, and `uds_control_forward.rs` for UDS
  forwarding behaviour.

**Tests to add/update:**

- Lifecycle transition tests for launch, socket readiness, busy/idle, await,
  timeout, completion notes, exit/failure, and kill.
- Forwarding tests for child-targeted commands and result recovery.
- BDD under `tests/features/subagent*.feature` only for user-observable flows.

**Docs and compatibility:**

- Update `docs/subagents.md` and `docs/uds-protocol.md` when commands, events,
  or notification semantics change.
- Preserve passive completion-note and explicit-await deduplication semantics.
- `get_subagents` must continue to expose enough identity/state for the unit
  tree.

**Common pitfalls:**

- Read-only spawned reviewers still have `bash`; treat `read_only` as a guard
  against accidental writes, not a sandbox.
- Preserve result recovery after child exit long enough for inspection.
- Test races explicitly: completion before await, await before completion, kill
  during busy, and exit before socket ready.

## Change context policy

**Start in:** the application context subsystem and its invariants.

**Production files usually involved:**

- `src/application/context_pruning.rs` and
  `src/application/context_pruning_messages.rs`.
- `src/application/agent_loop_pruning.rs`, `agent_loop_spill.rs`, and
  `agent_loop_context_gauge.rs`.
- `src/domain/session.rs` for spill index, dirty-prefix, or message metadata.
- `src/infrastructure/persistence/*` when spill/session storage changes.

**Tests to add/update:**

- Context pruning unit tests for pinned recent turns, tool-call/tool-result
  coherence, spill/recall promises, dirty-prefix marking, and gauge
  reconciliation.
- Agent-loop context tests when provider request construction changes.
- BDD under `tests/features/context_pruning.feature` for externally visible
  pruning/spill behaviour.

**Docs and compatibility:**

- Update `docs/sessions.md`, `docs/harness-architecture-map.md`, and the PRD/ADR
  links when policy or invariants change.
- Preserve session file format unless a separate ADR/PRD approves migration.
- Recall IDs and collapsed content promises are user-visible contracts.

**Common pitfalls:**

- Do not separate a tool result from its tool call.
- Provider-truth token counts should supersede local estimates when available.
- Do not make dirty-prefix bookkeeping double as a pruning policy.
