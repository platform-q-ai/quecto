# Harness clean-architecture review

## Scope and method

Target: `quecto-agentic-harness`. Review architecture, not cosmetic style; no implementation changes are part of this review.

Review one logical area at a time, with at most one active review agent. All agents use low effort. This document accumulates findings; unreviewed areas are not implicitly approved.

For each confirmed finding record evidence (`file:line`), the concrete architectural cost, a practical improvement, trade-offs, and a verification approach. Separate open questions and cross-area follow-ups from confirmed findings. Prefer improvements that reduce coupling or clarify policy ownership over abstraction for its own sake.

## Review sequence

Paths below are relative to `quecto-agentic-harness`.

| # | Area | Scope | Status |
|---|---|---|---|
| 1 | Domain model and core ports | `src/domain/**`, relevant domain and contract tests | Reviewed |
| 2 | Agent execution and context management | Application agent loop, context management, agent usage | Reviewed |
| 3 | Supporting application services | Catalogue, provider runtime, subagents, environments, extension tools, application ports | Reviewed |
| 4 | Provider and data adapters | Infrastructure providers, configuration, authentication, catalogue/model registry, persistence, reload, logging, time | Reviewed |
| 5 | Tool and execution adapters | Infrastructure tools, extensions, security, subagent process/container runtime | Reviewed |
| 6 | CLI/REPL composition | Main/lib entry points, CLI excluding UDS/protocol details, REPL, shared/catalogue/tool runtime wiring | Reviewed |
| 7 | UDS and long-lived sessions | Interface CLI protocol/UDS modules, UDS contracts and BDD tests, line-IO integration | Reviewed |
| 8 | Architecture enforcement and test quality | Architecture tests, contract-test enforcement, relevant lint/style configuration and documented architectural rules | Reviewed |

Area 5 assesses mechanics and enforcement rather than re-reviewing area 3's application policies. Area 6 assesses composition; area 7 owns detailed protocol/session behavior.

Order rationale: establish inner vocabulary and contracts, examine their application consumers, then assess adapters and the composition boundary against those contracts. Finally, review the enforcement suite itself against the observed architecture.

Area 8 will assess whether checks enforce actual dependency boundaries rather than names; bypasses via aliases, re-exports, macros and module placement; false positives and refactoring brittleness; behavioral substance of contract tests versus mere existence checks; gaps between documented principles and enforced rules; and separation of architecture, style/lint and behavioral checks. Use concrete bypass/false-positive examples where feasible, prioritizing useful safeguards over blanket restrictions. Carry A3-01's alias bypass into this dedicated review.

## Findings

### Area 1 — Domain model and core ports

Baseline: `cf861cad`. Static review of domain contracts and selected consumers/tests; no tests executed. Citations below are crate-relative. Priorities describe architectural improvement value, not demonstrated production failures.

#### A1-01 — Move finalization orchestration out of domain (priority: high)

**Evidence:** `src/domain/environment_finalization.rs:1-6` explicitly identifies itself as an application use case. `EnvironmentFinalizationUseCase` coordinates inspection, membership removal, cleanup/kill, and rollback (`:40-120`). Infrastructure constructs and executes it directly in `src/infrastructure/tools/subagent_cleanup.rs:101-114`. `src/domain/mod.rs:5-8` even attaches application-located tests to this domain module.

**Cost:** The dependency graph looks inward while infrastructure actually invokes application transaction policy concealed in the domain namespace. Policy ownership is misleading, weakening the value of layer-import checks.

**Improvement:** Move transaction orchestration to application; keep atomic membership/claim invariants in the domain registry. Inject an application-owned finalization capability into infrastructure cleanup callers, with its contract exposed through `application::ports` and its implementation wired at composition. Do **not** simply let infrastructure import the relocated concrete use case: `src/application/ports.rs:1-7` explicitly excludes use cases from that dependency surface. Separate the script-execution outbound port from the finalization capability consumed by callers.

**Trade-off:** Moderate wiring and test churn; relocation alone does not improve runtime behavior. Preserve current ownership, blocking execution, and exactly-once cleanup semantics.

**Verification:** Run existing application finalization tests, `tests/contracts/environment_finalization_port.rs`, cleanup/rollback tests, and architecture checks. Verify policy ownership and permitted imports rather than adding a brittle ban on names ending in `UseCase`.

#### A1-02 — Launch contracts expose adapter mechanics (priority: high)

**Evidence:** `src/domain/subagent_launch.rs:8-10,27-42,56-70` exposes OS arguments, executable paths, Tokio deadlines, monitor registration, and socket addressing. Runtime records also expose PID/socket details (`:95-105`). `src/application/subagent_launch.rs:33-48` sequences these operations. This conflicts with the pure-vocabulary intent of `docs/architecture-design-records/adr-0021-script-managed-subagent-launch.md:11-14`.

**Cost:** Application launch orchestration knows how the local executable is assembled, not just the launch transaction. Contract doubles must reproduce process-shaped operations; alternative adapters inherit that vocabulary even when it does not fit.

**Improvement:** First establish application ownership for orchestration contracts, exposing adapter-facing contracts through `application::ports`. Incrementally absorb `build_cli_args` and `resolve_binary` into adapter preparation; keep prepare/readiness/register/prompt/rollback policy explicit in application. Hide process-shaped prepared state behind adapter-owned handles where practical. Reassess deadline representation alongside retry policy rather than inventing a clock abstraction solely to remove an import.

**Trade-off:** Launch is lifecycle-sensitive. Split ownership correction from port redesign; do not flatten transactional stages into an opaque launch call or change rollback ordering. Existing direct/proxy endpoint requirements may justify some explicit endpoint vocabulary.

**Verification:** Preserve `tests/contracts/subagent_launch_ports.rs` success/failure and exactly-once rollback coverage. Add an in-memory preparation adapter that does not fabricate binary paths or argv. Review application retry and runtime ownership details in area 3 before fixing the full port shape.

#### A1-03 — Extension delivery envelope is classified as domain (priority: medium)

**Evidence:** `src/domain/extension_tool.rs:9-17` combines tool-call values with `tokio::sync::oneshot::Sender<ToolResult>`. `src/infrastructure/extensions/uds_tool.rs:13-24,67-75` creates and queues it for delivery. `src/application/extension_tool.rs:1-6` explicitly explains the domain placement as a way for infrastructure and interface to share the type without crossing boundaries.

**Cost:** An asynchronous delivery mechanism becomes core vocabulary to satisfy import direction. This obscures which layer owns extension delivery and requires consumers of the envelope to use Tokio's reply mechanism. This is not evidence that all Tokio or serialization usage in domain is inherently wrong.

**Improvement:** Move the delivery envelope to an explicit application-owned extension-delivery boundary exposed through the permitted ports surface. Keep tool arguments, identity, and results as domain values. Retain the existing channel initially; introduce a transport-independent reply contract only if an actual consumer needs it.

**Trade-off:** Mostly ownership clarity initially. Confirm composition and cancellation ownership in areas 5 and 7 before changing delivery mechanics.

**Verification:** Run extension dispatch, timeout, disconnect, and concurrent-call tests; check permitted application-port imports and ensure no reply ownership changes.

#### A1-O1 — Consider typed environment identities (opportunity, not a confirmed defect)

`src/domain/environment_registry.rs:64-71` distinguishes session refs, runtime IDs, and hidden environment UUIDs semantically but represents all as `String`; claims also retain string refs (`:125-140`). `src/infrastructure/tools/spawn_container.rs:375-377` populates all three together. No incorrect identity substitution was demonstrated.

Consider migrating session refs and environment UUIDs to distinct newtypes, following `src/domain/ids.rs`, if lifecycle API changes are undertaken. This prevents accidental interchange but causes broad signature/conversion churn; do not prioritize a standalone rewrite without additional misuse evidence. Preserve serialized forms and lookup behavior; verify identity separation with type-checking tests and existing registry contracts.

#### Strengths and limits

The registry's exclusive kill/inspection claims are useful domain invariants and should remain inward. Existing architecture and behavioral contract suites provide a foundation for safe changes. Findings concern semantic ownership that import direction alone cannot establish; they are not a recommendation to replace the architecture wholesale.

This was not an exhaustive semantic audit of every provider, message, session, tool, or workflow invariant. Only selected outer consumers were inspected to substantiate domain findings. Adapter behavior, complete application transactions, protocol compatibility, and test execution remain outside this pass.

### Area 2 — Agent execution and context management

Static review of orchestration/context code and selected consumers; no tests run.

#### A2-01 — Turn activity is not cancellation-safe (priority: medium; reusable-API hardening)

**Evidence:** `src/application/agent_loop.rs:543-545` marks a turn active before awaiting. Clearing is performed by explicit policy-drain branches (`src/application/agent_loop_policy.rs:343-364`), while `ImmediateIfIdle` consults that flag (`:249-258`). The loop's normal terminal paths call drains, but dropping its pending future does not execute those paths.

**Cost:** If a caller drops a pending process future and retains/reuses the agent, subsequent idle policy requests can be queued as if execution were still active. This is a code-supported cancellation hazard, not a reproduced production incident; whether outer callers reuse the agent after cancellation remains an area 7 follow-up.

**Improvement:** Represent turn activity with a cancellation-safe owned guard/lifecycle object whose drop clears activity. Keep policy draining explicit at cooperative boundaries: dropping a future should not implicitly perform asynchronous persistence or event emission. Avoid a guard borrowing all of the mutable agent.

**Trade-off/verification:** Small lifecycle refactor, but distinguish flag reset from pending-request reconciliation. Poll execution into a pending fake provider, drop the future while retaining the agent, and assert an idle mutation applies immediately. Retain normal boundary/drain tests and test subsequent queued-request handling.

#### A2-02 — Lifecycle vocabulary does not govern execution (priority: medium)

**Evidence:** `src/application/agent_loop_turn.rs:8-18` names states, but `src/application/agent_loop.rs:577,590,612-628` records them in unused `_state` locals. Actual transitions and per-turn counters/ledgers remain in `run_loop` (`:543-720`), including separate terminal cleanup branches.

**Cost:** Named states imply stronger lifecycle structure than they enforce. Adding a terminal branch still requires remembering cleanup manually. Existing classifiers are useful, but do not establish complete lifecycle handling. This overlaps A2-01 and is not a separate demonstrated failure.

**Improvement:** First centralize terminal outcomes and their shared cleanup, and remove misleading state annotations or make them operational. Extract a small turn context/transition dispatcher only where it reduces duplicated lifecycle decisions; a large typestate rewrite is not necessary.

**Trade-off/verification:** Moderate churn if generalized too far. Characterize observable event/message ordering first; retain malformed-response retries, tool continuation, final response, provider failure, iteration-limit, and cancellation tests. Prioritize the focused A2-01 fix before broader state restructuring.

#### A2-03 — Session/context lifecycle is coordinated across layers (priority: medium)

**Evidence:** Loop and context manager copy spill-store/session configuration (`src/application/agent_loop.rs:102-105,143-162`; `src/application/context.rs:81-94`). `src/application/agent_loop/agent_loop_session.rs:4-8` manually synchronizes session identity. Interface code resets history/snapshot namespaces and directly clears the exposed spill port (`src/interface/cli/uds_dispatch_session.rs:305-329,473-486`); snapshot refresh injects store/key separately (`src/interface/cli/uds_snapshots.rs:481-485`).

**Cost:** Cross-resource session consistency depends on interface handlers remembering the full operation sequence. Direct store access bypasses a cohesive application lifecycle boundary. Existing snapshot reset explicitly protects history/key consistency; no actual wrong-session recall was found.

**Improvement:** Introduce application-owned session/context lifecycle operations for switching and clearing, plus a narrow snapshot/recall capability. Delegate initially to existing persistence implementations and retain explicit failure reporting. Consolidate authoritative session identity where feasible rather than merely wrapping each current field setter.

**Trade-off/verification:** Requires interface/composition changes, not a new storage format. Do not promise atomicity across resources without defining failure semantics. Contract-test session switching, old-key isolation, clear failures, and snapshot consistency. Areas 3 and 7 should settle service ownership and error behavior before implementation.

**Strengths/limits:** Context invariants and pruning/spill regression tests provide a useful base; gauge reconciliation and saturating usage accumulation are already encapsulated. Outer consumers were sampled for boundary evidence, not comprehensively reviewed.

### Area 3 — Supporting application services

Static review; no tests run. Existing structured outcomes, retained-state behavior, transaction ordering and contract tests are useful foundations.

#### A3-01 — Root aliases bypass the ports-only dependency boundary (priority: high; extends A1-01/A1-02)

**Evidence:** `src/lib.rs:5-6` re-exports application modules as root aliases. `src/infrastructure/tools/environment_kill.rs:8` imports through one; `src/infrastructure/tools/agent_cmd.rs:31-53` stores and accepts the concrete `EnvironmentControlUseCase`. This conflicts with `src/application/ports.rs:1-7`, which permits contracts rather than concrete use cases.

**Cost:** Infrastructure is coupled directly to orchestration, while textual checks for `crate::application::` can miss the alias path. The issue is architectural ownership, not the alias spelling itself.

**Improvement:** Expose outbound environment ports through `application::ports`; inject a separate narrow environment-control capability into tools from composition. Remove internal reliance on aliases without casually breaking public compatibility. Strengthen architecture regression coverage for re-export/alias paths; merely changing the searched string is not a durable enforcement strategy.

**Trade-off/verification:** Moderate wiring churn. Preserve environment kill and launch exactly-once/rollback contracts. This extends the existing ownership findings rather than adding a separate cleanup redesign.

#### A3-02 — Publication guarantee exceeds the atomic boundary (priority: medium)

**Evidence:** `src/application/provider_runtime.rs:98-108` exposes two stores and promises atomic catalogue/runtime publication. `:137-149` publishes the catalogue during resolution, then publishes the runtime aggregate separately.

**Cost:** The runtime snapshot itself is coherent, but two independent read paths cannot share the documented atomic visibility guarantee. A concurrent reader of both stores can encounter different generations. No user-visible mismatch was reproduced; consumer reliance needs confirmation in areas 4/6.

**Improvement:** Prefer one authoritative aggregate with catalogue views derived from it. Alternatively explicitly scope coherence to runtime-aggregate readers and prevent decisions from combining independently read generations. If consolidating, resolve without publishing and publish only the completed aggregate.

**Trade-off/verification:** Consumer migration may outweigh the benefit if all consequential reads already use the aggregate. Add a controlled interleaving test for the supported read contract and retain failed-composition/last-good-state tests.

#### A3-Q1 — Clarify refresh timeout responsibility (open question, not a confirmed adapter defect)

`src/application/catalogue_refresh.rs:38-59,88-94,233-262` supplies cooperative cancellation and bounds to a synchronous adapter call; elapsed-time checks happen after it returns. The application cannot forcibly interrupt an uncooperative adapter. However, `src/infrastructure/catalogue_discovery.rs:304` does configure a client timeout, so the synchronous signature alone does not establish an actual unbounded production request.

Area 4 should establish whether all adapters honor the intended bound, including retries, body reading and persistence, and whether documentation promises interruption or merely cooperative limits. If strict application-level termination is required, consider an async or bounded execution port with explicit late-side-effect semantics. A timeout wrapper alone cannot stop blocking work or undo persisted updates. Test timeout/cancellation and completed-over-budget updates before changing the contract.

**Limits/follow-ups:** Selected consumers only. Area 7 remains responsible for A2-03 session lifecycle failure semantics. Deeper adapter and composition review will determine the practical reach of A3-02/A3-Q1.

### Area 4 — Provider and data adapters

Fresh static review at `43dfad9c` after moving to the chore branch; no tests run. Earlier areas retain their original baseline citations. This broad pass sampled boundaries and consequential consumers, not every provider/configuration/authentication implementation.

#### A4-01 — Model limits can diverge from selected runtime (priority: high; strengthens A3-02)

**Evidence:** `src/interface/catalogue_runtime.rs:71-77` selects against the runtime aggregate, while `:93-109` independently reloads/publishes catalogue sources when reading model limits. These are distinct generation paths, not merely two names for the same aggregate. Actual consumers include startup agent construction (`src/interface/cli/agent.rs:402-407`) and model switching (`src/interface/cli/uds_dispatch_runtime.rs:33-54`). In the latter, catalogue selection is a status verdict rather than the operation that installs the provider; the independently refreshed limits still apply to the current agent runtime.

**Cost:** A disk edit between composition and limit lookup can supply newer context/output limits alongside an older selected runtime. The earlier publication concern therefore has consequential consumers, though no execution failure was reproduced.

**Improvement:** Return selected model limits with the runtime/model selection snapshot, making execution use one generation. Keep fresh catalogue browsing separate and explicitly identified, rather than publishing as a side effect of an execution-limit read.

**Trade-off/verification:** Interface migration is needed; no new storage format. Test selection followed by an on-disk edit and limit lookup, plus controlled concurrent publication. Preserve intentional fresh-on-disk catalogue browsing without mixing it into an existing execution generation. Track as one consolidated publication improvement with A3-02.

#### A4-02 — Reload observation and successful application share one checkpoint (priority: medium)

**Evidence:** `src/infrastructure/reload.rs:80-90` advances observed fingerprints before rebuilding. `src/interface/cli/provider_reload.rs:90-97` retains the last-good runtime on rebuild failure but returns `Unchanged`; subsequent identical source observations do not trigger a rebuild.

**Cost:** A transient rebuild failure is not automatically retried until another detected edit or explicit forced reload. Last-good retention is sound, but observation is being treated as successful application. This is not evidence of data loss or permanent inability to reload.

**Improvement:** Distinguish observed and applied revisions, or retain pending-rebuild state. Define retry/backoff policy at the application lifecycle boundary instead of retrying every poll indiscriminately. Expose failure/retained state where callers need diagnostics rather than collapsing everything into unchanged.

**Trade-off/verification:** Invalid persistent inputs must not generate unbounded warnings or rebuild work. Test transient failure then success with unchanged bytes, persistent invalid input/backoff, and force reload while preserving last-good behavior.

#### A3-Q1 resolution — HTTP timeout is not an end-to-end refresh deadline

`src/infrastructure/catalogue_discovery.rs:301-320` configures reqwest's blocking request timeout and caps response reads. Cache persistence follows separately (`:354-366`); `src/application/catalogue_refresh.rs:253-266` deliberately preserves successful updates that finish over budget. Cancellation is cooperative and cannot interrupt this adapter's current network/write operation.

Document bounds as an HTTP/cooperative budget unless strict end-to-end interruption is actually required. If it is, redesign execution and late-write semantics together; an async timeout wrapper alone will not terminate blocking work. Verify slow body, slow persistence and cancellation cases. This remains a contract-clarity improvement, not a claim that production HTTP is unbounded.

**Strengths:** Atomic cache replacement, capped discovery reads, last-good runtime retention and same-read registry composition are useful safeguards worth preserving.

### Area 5 — Tool and execution adapters

Static, sampled review of tools, extensions, trust/security and subagent execution mechanics; no tests run. Existing strict JSON/endpoint validation, argv-based execution, content-hash trust and process ownership handling are useful foundations. Prior launch/finalization ownership issues are not counted again.

#### A5-01 — Extension timeout excludes admission to the delivery queue (priority: high)

**Evidence:** `src/infrastructure/extensions/uds_tool.rs:77-91` awaits channel send before applying the response timeout.

**Cost:** A full undrained channel can stall a call indefinitely despite its configured timeout. This extends A1-03 with a concrete delivery-lifecycle gap rather than a second ownership complaint.

**Improvement:** Give delivery one deadline covering queue admission and response, with distinct failure diagnostics. Retain transport mechanics in the adapter while making the timeout contract explicit at the delivery boundary.

**Trade-off/verification:** Total-budget semantics reduce response time after slow admission; document that change. Test a full undrained queue, disconnect, cancellation and normal replies. Define whether already-delivered timed-out work can still finish; cancellation of the waiter does not undo external execution.

#### A5-02 — Lifecycle script execution lacks an explicit bounded-execution contract (priority: high)

**Evidence:** `src/infrastructure/tools/spawn_container.rs:142-150,354-362` awaits exec/create output without a local deadline; `:78-82` similarly awaits cleanup completion.

**Cost:** Hanging configured scripts can prevent launch or rollback from completing. Trusted configuration does not guarantee terminating execution. This is a liveness concern, not proof that cleanup runs more than once or that configured scripts are a security sandbox.

**Improvement:** Centralize script execution mechanics with operation-specific deadlines, owned-process termination/reaping and explicit uncertain/failed-cleanup outcomes. Keep transaction ordering in application and preserve retained argv. Check outer cancellation guarantees before selecting timeout defaults.

**Trade-off/verification:** Killing a local script does not necessarily destroy an external environment it created. Preserve an actionable recovery path. Test hanging create/exec/cleanup, descendant termination, invalid-output rollback and late external side effects; do not claim exactly-once external cleanup from a timeout alone.

#### A5-03 — Trust persistence failures cannot be reported by the approval contract (priority: medium)

**Evidence:** `src/infrastructure/repo_local_container_config.rs:79-96` separates approval from a non-fallible recording operation and discards write errors. `:207-211` maps unreadable/corrupt stores to empty; `write_store` writes directly rather than atomically replacing the file (`:225-230`).

**Cost:** A user-approved invocation may proceed without remembered approval, causing repeated prompts and hiding storage problems. No unapproved execution bypass is demonstrated: the user has approved the content, and unreadable stored approval falls back conservatively.

**Improvement:** Return recording errors and distinguish approve-once from persist-approval decisions. Let the application/caller choose whether failed persistence blocks execution or proceeds with an explicit warning. Use atomic replacement and define concurrent-update handling rather than silently discarding approvals.

**Trade-off/verification:** Mandatory durable approval would change behavior for read-only installations and should not be imposed without a policy decision. Test corrupt/unwritable stores, concurrent approvals, approve-once and content-hash changes.

### Area 6 — CLI/REPL composition and runtime wiring

Static review; no tests run. Main entry-point delegation is thin and shared tool-runtime construction reduces entry-point drift. Detailed UDS/protocol behavior and enforcement are deferred to areas 7/8.

#### A6-01 — REPL message persistence resets unrelated metadata (priority: high; extends A2-03)

**Evidence:** `src/interface/repl/mod.rs:156-165,236-245` reconstructs a persisted `Session` for clear and exit with `workflow_run: None` and an empty subagent roster.

**Cost:** Saving a previously populated named session can replace workflow/roster metadata even when the operation concerns only conversation messages. Parent verification confirmed loading retains only messages (`src/interface/repl/mod.rs:520-533`), while persistence records metadata removal (`src/infrastructure/persistence/session_store.rs:509-519`); an all-empty save deletes the session file (`:120-125`). End-to-end REPL reproduction remains to be tested.

**Improvement:** Give application session updates explicit field-preservation semantics, or retain the loaded aggregate when replacing messages. Define clear-history's intended workflow behavior separately; incidental metadata loss should not follow from a handler rebuilding a DTO.

**Trade-off/verification:** Blind load/merge/save can introduce races, so respect existing session locking/version semantics. Test preloaded workflow/roster metadata across REPL exit and clear, plus deliberate resets. Consolidate with A2-03 rather than creating a second session service.

#### A6-02 — Workflow handoff input is deleted before validation (priority: medium)

**Evidence:** `src/interface/tool_runtime.rs:436-448` reads then removes a workflow spec before JSON parsing. `:337-354` reports load failure and proceeds without starting that workflow.

**Cost:** Malformed or partial handoff input is removed before successful acceptance, preventing inspection/retry. Destructive handoff policy is embedded in composition helpers.

**Improvement:** Define handoff ownership explicitly: validate and bind before acknowledgement/deletion, or quarantine rejected input if consume-on-read is intentional. Preserve refusal to start an invalid workflow; do not silently fall back to another template.

**Trade-off/verification:** Retained specs may contain sensitive data, so retention needs bounded cleanup and access controls. Test valid, malformed, oversized and bind-failure inputs with explicit retention/removal expectations. Oversized input is already rejected before deletion.

#### A6-03 — Shared interface utilities own credential lifecycle policy (priority: medium)

**Evidence:** `src/interface/shared.rs:216-291,383-452` orchestrates provider-specific refresh; `:301-347` chooses credential persistence-failure fallback; `:455-497` implements optional runtime-manager synchronization directly.

**Cost:** Shared CLI wiring doubles as credential application service and deployment adapter, tying reuse/testing of refresh and persistence policy to interface code.

**Improvement:** Extract an application-owned credential-refresh capability with explicit rotated-token persistence and fallback outcomes. Keep OAuth/provider parsing, credential storage and manager HTTP synchronization in adapters, wired at composition. Preserve optional best-effort sync as a stated policy, not an accidental swallowed error.

**Trade-off/verification:** Avoid a generic credential framework; migrate the existing paths together to avoid subtly different refresh rules. Contract-test rotation races, omitted refresh tokens, persistence failure, unsupported providers and optional-sync failure.

### Area 7 — UDS and long-lived sessions

Static review of selected dispatch, cancellation, emission and lifecycle paths; no tests run. Shared line-IO framing and snapshot namespace resets are useful foundations.

#### A7-01 — Notification broadcasting bypasses the bounded emitter (priority: high)

**Evidence:** `src/interface/cli/uds_cancel.rs:633-660` serializes and broadcasts notification/state events directly. The common `EventSink::emit_serialized` enforces the JSON event budget (`:210-229`), but this path does not use it.

**Cost:** An over-cap notification/state event can enter broadcast delivery despite the shared outbound size contract. The downstream writer **does** enforce its cap (`src/interface/cli/uds_wire.rs:56-65`), and the broadcast writer exits on write error (`src/interface/cli/uds_multi.rs:594-599`). Thus the supported risk is avoidable delivery failure/client disconnection, not an oversized frame escaping wire enforcement. The forwarding function also returns true without establishing successful bounded delivery. Normal completion notes are short; error/reason strings and aggregate roster size are the relevant growth paths (`src/infrastructure/tools/subagent_registry.rs:684-704`).

**Improvement:** Route broadcasts through one capped emission capability and explicitly distinguish accepted, oversized and disconnected outcomes where delivery drives subsequent LLM injection. Prefer bounded/reference-based notification content when dropping would lose important information.

**Trade-off/verification:** Centralizing rejection can change formerly attempted delivery into explicit drops; define that behavior. Test over-cap and normal notifications, ordering/deduplication and line-IO framing. Extend area 8 enforcement to prohibit bypassing the authoritative outbound boundary without requiring a particular helper name.

#### A7-02 — Pending command admission silently drops acknowledged input (priority: high)

**Evidence:** `src/interface/cli/uds_session.rs:316-333` enqueues/prepends only below the 64-item limit and returns no result. `src/interface/cli/uds_dispatch.rs:326-367` acknowledges steer/follow-up success regardless and clears the steer gate.

**Cost:** Saturated queues can discard user instructions while reporting success. Queue capacity is a sensible protection, but admission and interruption policy have no explicit contract.

**Improvement:** Return typed admission outcomes and acknowledge only accepted work; report queue-full otherwise. If steer deserves reserved capacity or replacement semantics, put that decision in the session/application policy rather than an incidental deque helper. Couple gate changes to the chosen admission outcome.

**Trade-off/verification:** Clients must handle an explicit rejection instead of assuming success. Test full queues for steer and follow-up, cancellation interactions, ordering, gate behavior and bounded memory.

#### A7-03 — Clear-history failure exposes a partial lifecycle commit (priority: high; extends A2-03/A6-01)

**Evidence:** `src/interface/cli/uds_dispatch_session.rs:473-489` clears memory, snapshot, usage, pending input and spill data before saving the durable session. Save failure reports an error without restoring the old state.

**Cost:** Live history may be cleared and spills removed while persisted history remains old; restart can reveal stale history whose recall data is gone. This is a concrete reason for application-owned lifecycle/failure semantics, not merely a large-handler complaint.

**Improvement:** Prepare and durably commit the intended session aggregate before irreversible cleanup, then publish live state and reconcile spill cleanup with explicit degraded outcomes. Define concurrency and recovery behavior; do not promise cross-store atomic rollback without supporting mechanisms.

**Trade-off/verification:** Persistence-first sequencing still needs crash and cleanup recovery semantics. Inject save and spill failures and test live reuse/restart, snapshot consistency and retry behavior. Consolidate with the existing session lifecycle findings.

#### A2-01 follow-up — Current UDS cancellation performs explicit reconciliation

`src/interface/cli/uds_cancel.rs:548-552` owns the process future in the cancellation loop; after unwinding, `:391` drains policy mutations at the boundary before reuse. This mitigates the reported activity-flag hazard on the inspected production path. Retain A2-01 as cancellation-safety debt in the reusable application API, not a demonstrated UDS defect; prioritize queue admission and session partial commits above it.

### Area 8 — Architecture enforcement and test quality

Agent mapping was followed by parent-only source verification and execution of the architecture/contract suites. Findings below are deliberately narrower than claims of complete compiler-level enforcement or absent behavioral coverage.

#### A8-01 — Lexical dependency checks miss actual dependency paths (priority: high; extends A3-01)

**Evidence:** `tests/architecture.rs:175-213` recognizes application dependencies by a literal `crate::application::` substring. Root application aliases (`src/lib.rs:5-6`) and concrete-use-case usage (`src/infrastructure/tools/agent_cmd.rs:31-53`) pass the check. The parent verified that exact production bypass and ran the suite successfully. Both scanners also stop at the first standalone `#[cfg(test)]` (`tests/architecture.rs:58-61,196-200`), although `src/domain/mod.rs:6-9` already contains production items after such an attribute.

**Cost:** A passing layer test is weaker evidence than its name suggests. Module layout and reference spelling affect enforcement independently of dependency direction. The environment handler guard (`tests/architecture.rs:265-298`) even recommends concrete-use-case delegation while the ports-only rule forbids direct infrastructure coupling to use cases.

**Improvement:** First reconcile the intended rule and protect the known alias/cfg-cutoff bypass with checker fixtures. Parse Rust items/imports and track relevant re-exports, or enforce visibility/crate boundaries where practical. Resolved dependency tooling offers stronger coverage but requires explicit cfg/macro/toolchain policy. AST parsing alone is not full name resolution.

**Trade-off/verification:** Do not replace a cheap useful ratchet with unreliable tooling or split crates solely for aesthetics. Checker tests should cover ordinary valid ports, forbidden aliases/re-exports, relative imports, multiline imports, comments/string literals and production items after test modules. Macro behavior needs explicit scope rather than an unverified claim that every macro bypasses checks.

#### A8-02 — Wire-cap enforcement and delivery tests cover different boundaries (priority: medium; extends A7-01)

**Evidence:** The common event emitter and notification broadcast path differ (`src/interface/cli/uds_cancel.rs:210-229,633-660`), while the final writer still caps payloads (`src/interface/cli/uds_wire.rs:56-65`). Existing architecture checks assert shared reader use, not universal capped broadcast admission. ADR-0008 explicitly describes bounded events as only substantially implemented, not a completed universal construction guarantee.

**Cost:** Existing wire protections do not ensure graceful event rejection, correct delivery accounting or continued connection health on every emitter path.

**Improvement:** Hide raw outbound broadcast access behind a narrow bounded capability where feasible. Add focused over-cap notification/roster tests covering writer survival and injection/admission semantics, not just frame size. Structural tests should reinforce that capability boundary rather than demand a helper spelling or ban unrelated `Sender<String>` channels.

**Trade-off/verification:** Shared framing is already valuable and must remain the final defense. Review sampled tests did not establish exhaustive absence of notification-size coverage; the recommendation is to make this specific behavior explicit, not to claim no related tests exist.

#### A8-03 — Contract discovery is an inventory check, not behavioral conformance (priority: medium)

**Evidence:** `tests/architecture.rs:301-369` extracts single-line `pub trait` declarations and checks matching registered module paths exist. It does not inspect assertions or implementation coverage; an empty registered module satisfies that inventory condition. Inline test layout (`:87-116`) is also checked in the architecture suite, but is a repository-style rule rather than dependency enforcement.

**Cost:** The test name can be mistaken for assurance that every real adapter meets a port's behavior. Some declaration spellings are outside the text scanner's scope, and module naming becomes part of coverage bookkeeping.

**Improvement:** Label and separate dependency, shape/style and behavioral checks clearly. Preserve the inventory ratchet, but pair important multi-adapter ports with reusable conformance assertions and an explicit implementation-to-contract map. Do not require network-backed tests for every vendor on every commit; use fixtures/local adapters and deliberate integration coverage.

**Trade-off/verification:** Current contracts are not empty or universally superficial: launch contracts exercise real local/script adapters (`tests/contracts/subagent_launch_ports.rs:247-383`), while finalization tests exercise claimed cleanup. Parent execution passed all 77 contract tests. Keep these strengths; add checker fixtures for detection limits and test relevant behaviors rather than assertion counts. CI and pre-push explicitly run both suites (`.github/workflows/ci.yml:67-68`; `scripts/pre-push.sh:99-100`).

## Consolidated priorities and remaining design questions

1. **Session lifecycle integrity:** A2-03/A6-01/A7-03. Preserve unrelated metadata and define persistence/cleanup commit semantics in one application boundary.
2. **Truthful admission and bounded execution:** A7-02, A5-01/A5-02 and A7-01. Acknowledge only accepted work, budget queueing as well as execution, and reject oversize events before damaging connection health.
3. **Honest dependency ownership and enforcement:** A1-01/A1-02/A3-01/A8-01. Move orchestration to application, inject capabilities rather than concrete use cases, and close demonstrated checker bypasses.
4. **Coherent runtime generation:** A3-02/A4-01. Couple execution limits to the runtime/model generation used by the agent.
5. **Failure-aware supporting services:** A4-02, A5-03, A6-02/A6-03. Distinguish observed/applied reload state, approval/durable approval, workflow validation/consumption, and credential policy/adapter mechanics.
6. **Targeted architectural hardening:** A1-03, A2-01/A2-02, A8-02/A8-03. Clarify delivery ownership and lifecycle contracts without a wholesale framework rewrite. Typed environment IDs (A1-O1) remain optional.

Open implementation decisions: strict versus cooperative refresh deadlines; late script/extension side effects after timeout; approve-once versus mandatory durable trust; workflow-spec failure retention; session crash recovery and partial cleanup; the practical scope of Rust-aware dependency resolution. Recommendations are designs to validate, not already-proven fixes.

## Parent-only final verification

Final verification baseline: `43dfad9c`, branch `chore/harness-clean-architecture-review`. All retained finding claims were checked personally against current source and consequential call sites; no agents were used for this verification phase. Areas 1–3 initially used `cf861cad`; comparing the source revisions showed only Codex-provider changes between those baselines, outside their cited findings. Area notes saying “no tests run” describe their initial review pass, not this final verification.

| Claims checked | Parent verification and disposition |
|---|---|
| A1-01/A3-01/A8-01 | Confirmed misplaced finalization orchestration, concrete-use-case aliases, ports-only intent and scanner bypass. Corrected recommendations to capability injection rather than merely relocating imports. |
| A1-02/A1-03/A1-O1 | Confirmed process-shaped launch signatures, Tokio reply envelope and untyped environment identities. Corrected runtime-record citation; identity newtypes remain an opportunity, not an observed bug. |
| A2-01/A2-02 | Confirmed manual activity lifecycle and unused state annotations. Followed UDS cancellation through its boundary drain and reduced A2-01 priority; no demonstrated UDS cancellation failure claimed. |
| A2-03/A6-01/A7-03 | Confirmed duplicated session coordination, message-only REPL load, metadata-clearing persistence and clear-before-save ordering. Failure consequences are source-supported, not fault-injection reproductions. |
| A3-02/A4-01 | Confirmed separate publication/read paths and actual limit consumers; clarified that UDS selection verdict itself does not install the provider. |
| A3-Q1/A4-02 | Confirmed bounded HTTP versus later persistence and observation-before-rebuild checkpointing. Removed any implication of unbounded production HTTP or inability to force a retry. |
| A5-01/A5-02/A5-03 | Confirmed send-before-timeout, unbounded local script waits and discarded trust write failures. No sandbox bypass, unauthorized approval or guaranteed external cleanup failure asserted. |
| A6-02/A6-03 | Confirmed delete-before-parse and credential refresh/persistence/sync decisions in interface helpers. Retention and service extraction remain design choices. |
| A7-01/A7-02/A8-02 | Confirmed direct broadcast bypass, final wire-cap enforcement/writer exit and silent full-queue admission failure. Corrected oversized-frame claim to rejection/disconnection risk. |
| A8-03 | Confirmed textual trait/module inventory, substantive representative contracts and CI/hook integration. No blanket assertion that existing contracts lack behavioral value. |

Commands personally executed:

- `cargo test -p quecto-agentic-harness --test architecture` — **46 passed**, 0 failed.
- `cargo test -p quecto-agentic-harness --test contracts` — **77 passed**, 0 failed.
- Read-only source assertions confirmed the existing root-alias escape and illustrated the scanner's first-`cfg(test)` cutoff without modifying repository code.
- `git diff --stat cf861cad HEAD -- quecto-agentic-harness/src quecto-agentic-harness/tests/architecture.rs` — checked baseline differences.

Limits: no new production code or regression tests were added; no end-to-end failure injection, provider-network tests or exhaustive review of every implementation. Passing existing suites establishes the current baseline, not reproduction or disproof of the listed failure modes. This document is a completed scoped architecture review, not a certification that all invariants are correct.
