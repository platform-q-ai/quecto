# PRD: Agentic Harness Architecture Hardening

**Status:** Draft for review
**Scope:** `quecto-agentic-harness`
**Related ADRs:**

- [ADR-0012 — Explicit Agent Turn State Machine](../architecture-design-records/adr-0012-explicit-agent-turn-state-machine.md)
- [ADR-0013 — UDS Command Dispatch Uses Command-Family Routers](../architecture-design-records/adr-0013-uds-command-family-router.md)
- [ADR-0014 — Context Management Is a First-Class Application Subsystem](../architecture-design-records/adr-0014-context-management-is-a-first-class-application-subsystem.md)
- [ADR-0015 — Subagent Lifecycle Is an Explicit State Machine](../architecture-design-records/adr-0015-subagent-lifecycle-state-machine.md)
- [ADR-0016 — Typed Identifiers for Protocol and Session Boundaries](../architecture-design-records/adr-0016-typed-identifiers-for-protocol-and-session-boundaries.md)
- [ADR-0017 — Protocol Evolution Is Tracked by a Capability Matrix](../architecture-design-records/adr-0017-protocol-evolution-matrix.md)
- [ADR-0018 — Contributor Change Cookbooks for Common Harness Extensions](../architecture-design-records/adr-0018-contributor-change-cookbooks.md)
- [ADR-0019 — Domain Ports Are Segregated by Role When They Grow](../architecture-design-records/adr-0019-role-segregated-domain-ports.md)

---

## 1. Problem

`quecto-agentic-harness` is well structured and heavily tested, but it now owns
several complex orchestration surfaces:

- agent turn execution;
- context pruning, spilling, gauges, and durable reconciliation;
- UDS protocol dispatch and compatibility;
- subagent lifecycle and forwarding;
- session/message/tool-call identity;
- extension and tool registry lifecycle;
- workflow, provider, audit, and progress integration.

The current architecture is sound, but several key behaviours are still implicit
in large coordination modules. That makes the next wave of work riskier than it
needs to be. The goal of this PRD is to harden the harness architecture without
rewriting it or changing public behaviour unnecessarily.

---

## 2. Goals

- **G1:** Make agent turn execution understandable as an explicit state machine.
- **G2:** Make context management a named application subsystem with documented
  invariants.
- **G3:** Reduce UDS dispatcher centrality by routing commands through focused
  command-family handlers.
- **G4:** Make subagent lifecycle transitions explicit and testable.
- **G5:** Reduce identifier ambiguity at protocol/session/subagent boundaries.
- **G6:** Improve protocol compatibility visibility through a capability matrix.
- **G7:** Improve contributor/agent ergonomics with change cookbooks.
- **G8:** Keep domain ports role-focused as they grow.
- **G9:** Preserve current external behaviour unless a later protocol-specific
  ADR/PRD says otherwise.

---

## 3. Non-goals

- No big-bang rewrite of the harness.
- No public UDS protocol shape changes as part of this PRD.
- No session file format changes unless separately approved.
- No replacement of the existing Clean Architecture boundary tests.
- No move of provider wire protocols, tool implementations, or persistence
  adapters into the domain/application layers.
- No speculative framework for future features that do not exist.

---

## 4. Users and stakeholders

- Core maintainers changing the harness safely.
- Agents working in the repo that need predictable implementation paths.
- TUI/API/subagent consumers depending on stable UDS behaviour.
- Future contributors adding tools, commands, providers, workflow features, or
  context-management behaviour.

---

## 5. Success metrics

- `AgentLoopImpl::process` and related code read as a sequence of named turn
  states rather than mixed orchestration.
- UDS dispatch top-level file routes by command family and contains minimal
  feature-specific logic.
- Context management has a named module boundary and invariant tests.
- Subagent lifecycle tests assert legal state transitions and important races.
- High-risk functions that previously accepted multiple `String` ids use typed
  identifiers or documented conversion points.
- Protocol capability matrix exists and is linked from UDS docs/ADR index.
- At least six change cookbooks exist and are linked from developer docs.
- Existing unit, architecture, contract, BDD, and repo-doc tests pass.

---

## 6. AST validation pass

A follow-up Rust syntax-graph pass (`rust_ast_graph`) was run after drafting the
ADRs/PRD to validate that the proposed phases match the code's current structure.
The pass was syntax/text-derived, not compiler type resolution, but it confirmed
the main architectural pressure points.

### 6.1 Agent loop integration point

`AgentLoopImpl` is implemented across `agent_loop.rs` plus several sibling
inherent impl modules, including clamp/model limits, effort, pruning, spill,
tool execution, and session-key handling. Its imports and fields connect provider
calls, tool execution, context pruning/spill stores, progress events, audit
sinks, usage accounting, model limits, and provider error classification.

**PRD implication:** Phase 2 should not invent a new top-level owner. It should
make the existing `AgentLoopImpl` orchestration read as named turn-state
transitions while preserving the public `AgentLoop` port.

### 6.2 Context subsystem already exists conceptually

The syntax graph showed a substantial context-management cluster:

- `application::context_pruning`
- `context_pruning_messages`
- context pruning/spill/unspilled tests
- `agent_loop_context_gauge`
- `agent_loop_context_tokens_tests`
- `agent_loop_ctx_mgmt_tests`
- `agent_loop_spill`
- `agent_loop_1072_tests`

It also showed a dedicated `ContextGaugeCalibration` type with methods such as
`reconcile_before_call`, `observe_provider_truth`, and `observe_estimate_only`.

**PRD implication:** Phase 1 is a consolidation/boundary-setting phase, not a
policy rewrite. Existing concepts and regression tests should move behind a
clearer context boundary before any pruning/spilling semantics change.

### 6.3 UDS dispatch is a tested central boundary

The graph located the central dispatcher at
`src/interface/cli/uds_dispatch.rs` and many direct test call sites across UDS
regression modules, including dispatch coverage, subagent forwarding, masked
pruning, resume/persist, effort/model, and abort/steer tests.

**PRD implication:** Phase 3 should treat the current dispatcher as a valuable
compatibility boundary. Split routing internally, but preserve direct behavioural
tests and add command-family tests rather than replacing the boundary with a
large dynamic framework.

### 6.4 Subagent lifecycle is a real event-processing cluster

Subagent monitor functions include event application, parsed-event handling,
exit marking, monitor task spawning, monitor loops, bounded message reads,
line/event handling, workflow/message forwarding, terminal-completion gating,
notification dispatch, and socket connection retry.

**PRD implication:** Phase 4 should define lifecycle states/events around the
existing monitor/registry/notification behaviours. Process state alone is
insufficient; the lifecycle model must distinguish socket readiness, child-run
state, registry state, passive completion notes, and forwarded child history.

### 6.5 Identifier ambiguity is visible at protocol boundaries

The syntax graph showed protocol/session/subagent public API surfaces using raw
strings for correlation ids, agent ids, parent ids, session keys, message/tool
ids, and related response fields.

**PRD implication:** Phase 5 should start with high-risk boundaries where several
id kinds coexist in one function: UDS forwarding, child history retrieval,
session message lookup, and event/audit/progress correlation. Serialization must
remain string-compatible.

### 6.6 Role-segregated ports remain conditional

The code-reading pass identified broad tool-registry responsibilities: catalog,
execution, extension lifecycle, and session-key propagation. The AST public API
query reinforced that domain ports are key architecture seams, but the split
should still be driven by caller pressure.

**PRD implication:** Phase 6 stays deliberately conditional: split only when a
trait's independent roles are causing mock/test/caller friction.

---

## 7. Phased implementation plan

### Phase 0 — Baseline and documentation scaffolding

**Purpose:** Create the documentation and measurement baseline before code moves.

**Deliverables:**

1. Add this PRD and ADR-0012 through ADR-0019.
2. Update the ADR index with the new proposed ADRs.
3. Add a protocol capability matrix document and link it from `uds-protocol.md`
   and the ADR index.
4. Add a short harness architecture map covering:
   - turn execution;
   - context management;
   - UDS dispatch;
   - subagent lifecycle;
   - persistence/session recovery.
5. Record baseline longest files and subsystem test commands in the PR or issue.

**Acceptance criteria:**

- Repo-doc tests pass.
- No production behaviour changes.
- New docs link cleanly.

---

### Phase 1 — Context subsystem boundary

**Purpose:** Move context management behind a clearer application boundary before
changing turn orchestration.

**Deliverables:**

1. Create an `application/context` namespace or equivalent module grouping.
2. Move or wrap existing context pruning/spill/gauge functionality behind a
   `ContextManager` or similarly named facade.
3. Document invariants in module docs:
   - pinned recent turns;
   - tool-call/tool-result coherence;
   - spill/recall promises;
   - provider-truth vs local-estimate gauges;
   - durable prefix dirty semantics.
4. Add focused tests for context plans and reconciliation outcomes.
5. Keep existing public behaviour and persistence format unchanged.

**Acceptance criteria:**

- Existing context pruning/spill tests pass.
- New tests cover at least pinned turns, dirty-prefix marking, and provider-gauge
  reconciliation.
- `AgentLoopImpl` delegates context decisions through the new boundary.

---

### Phase 2 — Agent turn state machine extraction

**Purpose:** Make the core turn lifecycle explicit while preserving the
`AgentLoop` public port.

**Deliverables:**

1. Introduce internal turn-state vocabulary.
2. Extract provider request construction and response classification into named
   transition functions or services.
3. Extract malformed-response recovery into a named transition.
4. Extract finalization/reconciliation into a named transition.
5. Keep tool execution initially compatible with existing helper modules; do not
   rewrite all tool handling in this phase unless necessary.
6. Add tests for state transitions and failure/cancellation paths.

**Acceptance criteria:**

- `AgentLoopImpl::process` is shorter and primarily coordinates named states.
- Existing agent-loop tests pass.
- New tests identify at least these outcomes:
  - final assistant response;
  - tool-call continuation;
  - malformed model retry;
  - provider failure classification;
  - cancellation or abort behaviour.

---

### Phase 3 — UDS command-family routing

**Purpose:** Reduce dispatcher centrality and isolate command-family semantics.

**Deliverables:**

1. Introduce a pre-router for subagent-targeted forwarding that must occur
   before local fast paths.
2. Split command handling into command-family modules:
   - conversation;
   - history;
   - subagents;
   - session;
   - model/runtime;
   - workflow;
   - extensions;
   - lifecycle/query.
3. Keep top-level dispatch responsible for cross-cutting protocol invariants:
   correlation id handling, response emission, frame guards, and broadcast/writer
   selection.
4. Add or update tests for each command family.

**Acceptance criteria:**

- Existing UDS tests pass.
- Top-level dispatcher contains little feature-specific command logic.
- Child-targeted `get_messages`, `get_message`, and `sync` are handled by the
  forwarding pre-router and cannot fall through to parent-local history.

---

### Phase 4 — Subagent lifecycle state machine

**Purpose:** Make subagent lifecycle transitions explicit and race-resistant.

**Deliverables:**

1. Introduce lifecycle state and event vocabulary.
2. Wire process monitor, registry, passive completion notes, and
   child command forwarding through the lifecycle model where appropriate.
3. Add transition tests for launch, socket readiness, idle/busy,
   completion note coalescing, exit/failure, and kill.
4. Ensure parent-facing `get_subagents`, `agent_cmd get_messages`, and passive notes
   preserve existing semantics.

**Acceptance criteria:**

- Existing subagent monitor/registry/tool tests pass.
- New tests assert legal lifecycle transitions.
- Race-prone behaviours have explicit tests:
  - completion before observation;
  - observation before completion;
  - kill during busy;
  - child exits before socket ready.

---

### Phase 5 — Typed identifiers at high-risk boundaries

**Purpose:** Reduce ambiguity between command ids, message ids, session keys,
agent ids, and tool-call ids.

**Deliverables:**

1. Add lightweight string-serializing newtypes for the first identifier set:
   `SessionKey`, `AgentId`, `MessageId`, `ToolCallId`, `CommandId`.
2. Adopt them first in UDS forwarding, subagent history retrieval, and session
   message lookup code.
3. Provide test helpers/builders to avoid noisy tests.
4. Preserve JSON wire and persistence representation as strings.

**Acceptance criteria:**

- No UDS JSON shape changes.
- High-risk forwarding/history functions no longer accept multiple unrelated
  raw `String` ids in the same signature.
- Serialization round-trip tests prove compatibility.

---

### Phase 6 — Role-segregated ports where pressure exists

**Purpose:** Keep domain ports expressive and narrow as harness features grow.

**Deliverables:**

1. Audit existing domain ports for role pressure, starting with tool registry
   responsibilities.
2. Split only ports where callers demonstrably need separate roles.
3. Preserve ergonomic composition in construction roots.
4. Update contract tests for any new public ports.

**Acceptance criteria:**

- No trait is split without a concrete caller/test benefit.
- Contract tests exist for new public ports.
- Architecture boundary tests still pass.

---

### Phase 7 — Contributor cookbooks and local check scripts

**Purpose:** Make the architecture easier to follow for humans and agents.

**Deliverables:**

1. Add cookbooks for at least:
   - adding a built-in tool;
   - adding/changing a UDS command;
   - adding provider/model runtime capability;
   - adding progress/audit event;
   - changing session persistence;
   - adding subagent behaviour;
   - changing context policy.
2. Add or document focused local test commands/scripts for:
   - agent loop;
   - context;
   - UDS;
   - subagents;
   - protocol docs;
   - architecture/contract tests.
3. Link cookbooks from README or developer docs.

**Acceptance criteria:**

- A contributor can identify production files, tests, docs, and compatibility
  concerns for each cookbook topic.
- Repo-doc tests pass.

---

## 8. Rollout strategy

- Land phases independently.
- Prefer behaviour-preserving refactors with tests before and after.
- Do not combine protocol behaviour changes with structural refactors.
- For each phase, run the narrow subsystem tests plus the standard harness CI
  suite before merge.
- If a phase uncovers a needed public behaviour change, pause and write a
  separate ADR/PRD for that change.

---

## 9. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Refactor changes subtle agent-loop behaviour | Preserve existing tests; add transition tests before moving logic. |
| UDS routing split changes command ordering | Introduce forwarding pre-router first; add regression tests for fallthrough prevention. |
| Context subsystem extraction changes pruning semantics | Golden/regression tests around current outcomes before policy changes. |
| Typed ids create excessive boilerplate | Adopt only at high-risk boundaries; add builders/helpers. |
| Port segregation over-fragments APIs | Split only when caller pressure exists. |
| Docs drift | Link docs in repo-doc tests and require updates in PR checklist. |

---

## 10. Open questions

1. Should typed identifiers live in `domain` or in a protocol/session-specific
   module? Default: put broadly meaningful identifiers in domain, protocol-only
   identifiers near UDS types.
2. Should the turn state machine be represented as a public enum for tests or as
   private transition functions with observable outcomes? Default: keep private
   unless tests need public vocabulary.
3. Should command-family routing use enum family methods or direct match arms in
   the top-level router? Default: direct static routing until dynamic dispatch is
   justified.
4. Should context subsystem extraction happen before or after ADR-0008 pending
   paging work? Default: extract boundary first if it is behaviour-preserving;
   avoid policy changes until paging work is settled.

---

## 11. Implementation checklist by phase

- [ ] Phase 0: docs, ADR index, protocol matrix, architecture map.
- [ ] Phase 1: context subsystem boundary and invariant tests.
- [ ] Phase 2: agent turn state machine extraction.
- [ ] Phase 3: UDS command-family routing.
- [ ] Phase 4: subagent lifecycle state machine.
- [x] Phase 5: typed identifiers at high-risk boundaries.
- [x] Phase 6: role-segregated ports where needed.
- [x] Phase 7: contributor cookbooks and focused local checks.
