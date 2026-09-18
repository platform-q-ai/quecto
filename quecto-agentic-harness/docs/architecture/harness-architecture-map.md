# Harness Architecture Map

This map gives contributors a short orientation to the main
`quecto-agentic-harness` orchestration surfaces. It is intentionally descriptive:
Phase 0 of the architecture-hardening PRD does not change runtime behaviour.

For the hardening plan, see
[PRD: Agentic Harness Architecture Hardening](../prd/prd-harness-architecture-hardening.md).
For the related decisions, see the [ADR index](../architecture-design-records/README.md).

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

See also the [UDS protocol reference](../uds-protocol.md) and
[protocol capability matrix](protocol-capability-matrix.md).

## Subagent lifecycle

**Primary code:** `src/infrastructure/tools/spawn.rs`,
`spawn_binary.rs`, `subagent_registry.rs`, `subagent_monitor.rs`,
`subagent_monitor_stall.rs`, `subagent_monitor_merge.rs`,
and `agent_cmd.rs`.

Subagents are spawned harness processes supervised by the parent. The parent
tracks process launch, socket connection/readiness, forwarded child events,
message-history retrieval, busy/idle snapshots, terminal exit/failure, passive
completion notifications, and `agent_cmd get_messages` transcript reads.

Important invariants before Phase 4:

- spawning returns quickly while monitoring continues asynchronously;
- child commands are routed over the child's UDS socket when possible;
- passive completion notes are coalesced and pair with `agent_cmd get_messages` for result recovery;
- `get_subagents` reports enough identity/state to rebuild the unit tree; and
- exited children remain inspectable long enough for result recovery.

Container config selection (#2024 S4a) is launch policy, not tool plumbing:
`src/application/subagents/use_cases/select_container_config.rs` picks the
named or labelled-default entry (enumerating the live names on every refusal
and rejecting unrunnable argv) from the set its `EffectiveContainerConfigs`
port returns — the configuration capability's *effective* configuration for
the launching agent's checkout (trusted overlay merged entry-wise, untrusted
overlay reported and not applied — withholding the implicit default only
when it is refused, unparseable or failing the trust checks, or declares `container_configs`, judged in
`composition/container_configs.rs` on the resolver's parsed top-level keys)
or an explicit spawn `config` file, which
replaces the layers. `src/infrastructure/config/container_configs.rs` adapts
the port; `src/composition/container_configs.rs` binds it to the run's own
configuration selection, and the spawn tool only holds and invokes the
composed handle (a launcher composed without one refuses new containers).

Container failures are diagnosable (#2024 S4b). Every container script run
(create/exec in `spawn_container.rs`, the retained inspect/kill/cleanup in
`environment_commands.rs`) goes through
`src/infrastructure/processes/containers/script_stderr.rs`, which keeps a
bounded, sanitised tail of the script's stderr and appends it to the
`script-managed <op> failed with status …` error (also echoed on the
harness stderr). `quecto container doctor` is the environments
capability's `application/environments/use_cases/diagnose_container_runtime.rs`
over two capability-local ports: `ContainerConfigLookup` (adapted in
`src/infrastructure/config/container_config_lookup.rs` over the launch
policy's `SelectContainerConfig`, so the doctor and `spawn` agree on the
effective config) and `ContainerRuntimePreflight` (adapted in
`src/infrastructure/processes/containers/preflight.rs`, which runs the
config's create argv with `--preflight-only` and parses its
status/check/detail/remedy lines — the checks live in the script, one list
for the create and the doctor). `composition/environments.rs` builds the
doctor; `interface/cli/container.rs` parses, invokes it and presents.

## Persistence and session recovery

**Primary code:** session vocabulary in `src/domain/session.rs`,
`src/domain/session_identity.rs`, `src/domain/conversation_view.rs` and
`src/domain/conversation_edit.rs`; the sessions capability in
`src/application/sessions/` (`use_cases/`, `ports.rs` + `ports/`, `dto/`,
`active_session.rs`, `conversation_ledger.rs`, `session_home.rs`); persistence
adapters in `src/infrastructure/persistence/` and
`src/infrastructure/session_export.rs`; workspace discovery adapters in
`src/infrastructure/workspace/` (`git_scope_discovery.rs`,
`filesystem_scope.rs` — the only production spawns of `git`, resolved on PATH
once and run off the async executor); the graph in
`src/composition/{sessions,session_home,active_session,session_report,
retention,fleet_settlement}.rs`; the wire edge in `src/interface/uds/sessions/`
and the session modules of `src/interface/cli/`.

Sessions capability, final shape (epic #1968, closed by #1979): fourteen use
cases own every session transaction and query — `ListSessions` (#1861),
`ReadHistory` (#1856), `RecoverMessage` (#1858), `SynchronizeTranscript`
(#1857), `ExportSessionReport` (#1859), `SaveSession` (#1860),
`ClearConversation` (#1864), `RewindConversation` (#1865),
`StartFreshConversation` and its `DepartingChildren` collaborator (#1862),
`ResumeSavedSession` (#1863, also the startup open), and the retained-context
owners `RecallContext`, `RetainContext` and `ListRetainedContext` (#1866).
Twelve ports are declared under the capability's `ports` files and contracted
under `tests/contracts/`; DTOs are domain values. Every port operation is keyed
by the typed `SessionIdentity` (the existing raw key only); the flat
`<base>/sessions/` projection (`.json`, `.owner`, `spill.jsonl`) is owned by
`FlatSessionLayout` in `src/infrastructure/persistence/session_layout.rs`
alone. Composition is the only constructor of a use case, a controller, a
store or the one application-owned `ActiveSessionState` (typed identity plus
the `ConversationLedger` read model — published transcript, bounded full-copy
ledger, sync frontier, retention backstop — and the persistence state), and
hands the interface plain handles through `CliComposition`. The interface
parses, maps and presents: the idle dispatch loop, the busy reader task and
the connect-time snapshot read through the same owners, the `sessionKey` every
presenter reports is the active session's identity, and no interface module
reaches a store method, names an adapter or converts a raw key. Sessions alone
owns durable retained context; the context-pruning policy (`src/application/
context*.rs`, the agent loop) alone decides what to retain and consumes the
narrow `RetainContext`/`ListRetainedContext` handles. `tests/architecture/
sessions_capability.rs`, `sessions_epic_close.rs` and
`sessions_epic_close_retirement.rs` pin these rules as exact, decrease-only
inventories, and `docs/sessions.md` names every use case and port.

The wire protocol was unchanged by epic #1968. Folder-aware discovery
(#2009, parent #2001) extends the existing sessions query/save/resume owners;
it does not change opaque `SessionIdentity` or the global `FlatSessionLayout`.
Home metadata is separate from identity. Git-reported common-dir/worktree
relationships and canonical paths define discovery grouping; the one
eligibility rule is the domain's `SessionHome::admission`, and identical group
membership does not authorize restore in another execution directory. The
home context is composed once per loop over the one file store and is
mandatory for resume. Cross-folder executors and metadata search are later
slices, not #2009.

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

## Provider runtime composition

**Primary code:** the compose-provider-runtime use case in
`src/application/provider_runtime.rs`; the concrete factories in
`src/infrastructure/provider_runtime.rs` and
`src/infrastructure/provider_runtime_admission.rs`; the OAuth refresh wiring
in `src/infrastructure/providers/refresh_wiring.rs` and the refreshed-credential
persistence in `src/infrastructure/auth/token_refresh.rs`; the composition in
`src/composition/runtime.rs` (`compose_and_publish_runtime`,
`build_agent_provider`).

`main` hands composition's `build_agent_provider` to the CLI through
`CliComposition` as the one `ProviderRuntimeBuilder` type
(`src/infrastructure/runtime_configuration.rs`, aliased by the CLI); agent
startup (`src/interface/cli/agent.rs`) calls the injected builder, never
constructs provider state itself (#1849 PR 1), and threads the same builder
into the reload inputs. Reloading a running session is the
reload-runtime-configuration use case
(`src/application/catalogue/use_cases/reload_runtime_configuration.rs`,
#1849 PR 2): forced by the UDS `reload` command, polled before every prompt
and `set_model`, over the `RuntimeConfigurationSource` port that
`src/infrastructure/runtime_configuration.rs` implements (the ADR-0002 gate
in `src/infrastructure/reload.rs`, one `Config` read, the injected builder)
and the `ReloadRuntime` port the agent loop implements. The use case is two
scheduler-free phases — `rebuild`/`rebuild_if_changed` yielding a
`ReloadStep`, then `apply` — and `src/interface/cli/uds_dispatch_reload.rs`
runs the rebuild under `tokio::task::spawn_blocking` so the current-thread
UDS runtime keeps serving while it is in flight; composition builds the use
case into `CatalogueHandles.reload` and the presenter in
`src/interface/uds/catalogue/reload_presenter.rs` renders the outcome. The
durable tool-policy persistence hook is its own composition seam
(`src/composition/tool_policy.rs`, `CliComposition.tool_policy_persistence`),
installed on the loop by the agent build; it is a caller of the configuration
writer below, patching `tools.policy.entries` only. The catalogue's
default-persistence ports (`ModelDefaultPersistence`,
`EffortDefaultPersistence`, #2024 S2 — `set_model`/`set_effort … persist`)
are served by a mapping in `src/composition/catalogue_defaults.rs` onto the
configuration capability's `PatchConfiguration` use case, in the manner of
`fleet_settlement.rs`: it touches no file or lock itself, so it is
composition, not infrastructure, and the infrastructure ratchet stays exact. The
composition publishes the routing provider and the catalogue as one
generation into the per-directory stores; a failed composition retains the
previously published generation.

## Configuration: selection, overlay, safe writer

The configuration capability (`src/application/configuration/`, #1966,
#2024) owns which files a run loads and the one write path every change goes
through. `use_cases/select_config.rs` turns the CLI's inputs into a
`ConfigSelection` (`dto/config_selection.rs`): an explicit `--config` file, or
the global `<base_dir>/config.json` layered with the working directory's
`.quecto/config.json` overlay candidate (and the retired `<cwd>/config.json`
location, reported if present, never loaded).
`use_cases/resolve_effective_config.rs` reads both layers through the
`ConfigDocumentStore` port (the overlay through its `read_overlay`, whose
typed `OverlayDocument::Refused` carries the one overlay policy — no
symbolic link below the working directory — as a reason the report,
`config trust` and `config set` print), gates the overlay through the `OverlayTrustStore`
port (untrusted: reported in the `ConfigSources` layer report, not applied),
refuses the global-only sections (`providers`, `admission` — the rule lives in
the use case), validates each layer and the merge through the `ConfigValidator`
port, and merges by the pure section-wise policy in `overlay_policy.rs`.
`use_cases/patch_configuration.rs` is the safe writer: a dotted-path patch of
the JSON document (never of the `Config` struct, so unknown keys and order
survive), global-only keys and whatever the store refuses (a symbolic link
on the way to the overlay — nothing is written through it) refused for the
overlay, an untrusted overlay never patched, the result validated before a byte is written
— the layer alone, then the merge through `ResolveEffectiveConfig::preview`
(an intra-capability use-case call; cross-capability calls go through ports)
— the whole cycle under the writer port's exclusive hold (`DocumentLock`), the
write through the `ConfigDocumentWriter` port, and trust recorded for the
bytes the writer returned. `use_cases/read_configuration.rs` (secret-shaped
leaves redacted unless revealed; `redaction.rs`) and
`use_cases/trust_config_overlay.rs` serve `quecto config get` and
`quecto config trust`.

Infrastructure lives under `src/infrastructure/config/`: `loaders.rs` (the
filesystem document store — a present-but-broken entry is an error, never
absence; `read_overlay` refuses a symbolic link at `.quecto` or at the file,
stated once), `mapping.rs` (document → `Config` with every load-time validation,
the env overrides, the validator adapter, `realize_config`),
`persistence.rs` (the overlay trust record `<base_dir>/config-overlay-trust.json`,
canonical path + sha256 — since #2024 S4a the one record that also gates the
`container_configs` a spawn selects from; the interactive prompt shows the
document, bounded), `container_configs.rs` (the subagent capability's
`EffectiveContainerConfigs` port over the effective configuration, bound by
`composition/container_configs.rs` to the launching agent's own selection),
and `writer/` (the JSON document writer: existing
indentation kept, tmp + fsync + rename via `atomic_write`, an exclusive
`flock` on `<base_dir>/locks/<sha256 of the document's canonical path>.lock`
— never beside the document, so a refused write of a not-yet-existing
overlay leaves the checkout clean; plus the tool-policy persistence hook as
a caller of it, under the same lock). The runtime-configuration source
(`src/infrastructure/runtime_configuration.rs`) rebuilds through a
composition-supplied `ConfigLoader` and watches the overlay as well as the
base file, and the trust record while the overlay exists
(`ReloadSource::while_present`). Composition (`src/composition/configuration.rs`) builds the
`ConfigurationHandles` (`src/interface/cli/configuration_handles.rs`,
including the `realize` document → `Config` handle) `main` hands the CLI
through `CliComposition.configuration`, and the loader for reloads (which
watches the trust record only when an overlay existed at seed time); the
interface (`src/interface/cli/{config_cmd.rs, config_loading.rs,
commands.rs}`) parses `quecto config get|set|trust`, loads through the
handles and presents the layer report in `quecto status`.

## Baseline subsystem checks

Use these focused checks while hardening the architecture. The full pre-push gate
remains authoritative before a PR is handed off.

| Subsystem | Focused check |
|---|---|
| Repo docs / Phase 0 links | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --test docs repo_docs::` |
| Architecture boundaries | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --test architecture` |
| Context management | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib context_pruning` |
| Agent loop | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib agent_loop` |
| UDS protocol/dispatch | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib uds` |
| Subagent lifecycle | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib subagent` |
| Workflow/session recovery | `cargo test --workspace --features quecto-agentic-harness/test-support --bins --lib workflow` |

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
| 1099 | `tests/docs/workflow_config_template.rs` |
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
