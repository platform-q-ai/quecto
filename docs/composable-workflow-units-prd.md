# PRD: Quecto as the Smallest Repeatable Unit for Composable Agentic Workflows

## Problem

The quecto harness already has the right *shape* for composing complex agentic
workflows: a sub-agent is the **same binary over the same protocol** as its
parent (`spawn` launches `quecto agent --mode uds`), so composition is naturally
recursive — a parent is just a quecto whose current workflow step happens to
spawn children. This homogeneity is the harness's core asset.

But the unit is not yet *defined by a contract*, and that blocks safe
composition at depth:

1. **Assignment is a boolean, not a spec.** `SpawnTool` (`src/infrastructure/tools/spawn.rs`)
   accepts only `workflow: bool` / `workflow_guards: bool`. A parent can enable
   the *capability* on a child but cannot hand it a specific template, inputs, or
   acceptance criteria. Each child builds its own `WorkflowEngine`
   (`src/domain/workflow.rs`) from shared config — there is no way to instantiate
   a *particular* sub-workflow.

2. **Results are unstructured, and completion is conflated with idleness.** A
   parent that spawns a child cannot reliably branch on a *typed* outcome; it
   must interpret prose. There is no verdict contract to verify against.
   Concretely (verified): a per-child `subagent_monitor` pushes a notification
   on the child's `agent_end`→`Idle` transition (and on exit), delivered both to
   connected clients and into the parent LLM as a follow-up message — but there
   is **no distinct "workflow-completed" signal and no typed verdict**. The
   parent learns "the child went idle," not "workflow X finished with result Y."
   The explicit `agent_cmd await` pull returns a workflow *snapshot* (current
   state), not a completion verdict.

3. **Observability is per-connection and identity-free.** `workflow_state`
   events are broadcast on a per-session channel and carry no `agent_id`
   (`src/interface/cli/protocol.rs`). A child's workflow is invisible to the
   parent and to any supervisor. `SubagentInfoEvent` carries only
   `agent_id / status / last_tool / last_error / pid` — no workflow, no parentage.
   The recently shipped quecto-tui sub-agent/workflow UI improvements can only
   render what the harness emits, so the tree cannot be reconstructed by any
   consumer.

4. **Repeatability is partial.** Sessions and `workflow_run` are persisted, but
   there is no per-step journal that makes a unit *replayable* (resume from last
   good step; re-run as replay rather than redo).

The goal of this PRD is to define the properties that turn the existing process
into a **contract-bounded, repeatable, composable unit** — so arbitrarily deep
agentic workflows can be assembled, observed, verified, and resumed uniformly.

## Goal

Define the quecto unit as a near-pure function:

```
unit(spec) -> result        (with a durable journal as a side effect)
```

where every node in a workflow tree is an identical quecto unit, and the only
coupling between parent and child is **the spec in** and **the typed result out**,
plus an **identity-tagged event stream** that any consumer can subscribe to.

Delivered in stages so each is independently shippable and reviewable:

| Stage | Capability | Primary surface |
|---|---|---|
| A | Spec-based assignment + typed result | `spawn`, workflow engine init, `agent_cmd_await` |
| B | Identity-tagged event bus (push) | UDS event layer, `protocol.rs`, TUI mirror |
| C | Per-step journaling + resume | workflow engine, session/`workflow_run` persistence |
| D | First-class verification gates | template schema, workflow engine |
| E | Isolation bounds (budget / depth / concurrency) | `spawn`, workflow engine |

Stages A and B together make the per-sub-agent-workflow visibility we previously
scoped fall out for free and *uniformly* (see "Relationship to prior work").

**Status (2026-06-18):** Stage A shipped (PRs #682–#684) — by-value `workflow_spec`
assignment + typed `agent_cmd await` result. Stage B shipped (PR #685) —
identity-tagged event bus: `--parent-id`, `agent_id`/`parent_id` on every
`workflow_state` event, child→parent event forwarding, `parent_id` + workflow
snapshot on `SubagentInfo`, and `UnitTree::from_events` tree reconstruction.
Stages C–E are not yet started.

## Non-goals

- Do not introduce a privileged "orchestrator" agent type. Every node remains a
  full quecto unit; the parent is not special. (This invariant is load-bearing —
  it is what keeps any node independently runnable.)
- Do not move orchestration control flow into model free-choice. Control flow
  (which steps, what to spawn, fan-out/fan-in, gates) stays declarative or
  scripted and deterministic; the model decides *content within a step*.
- Do not change existing single-agent `--workflow` / `--no-workflow` launch
  semantics (see `docs/workflow-availability-prd.md`).
- Do not redesign the workflow template vocabulary; extend it additively.
- Do not require the TUI for any harness capability. The TUI is one subscriber.

## Design principles (invariants)

1. **Homogeneous recursion.** A sub-agent is the same unit as its parent. No
   orchestrator-only code path.
2. **Contract in, contract out.** A unit is defined by a typed input spec and a
   typed output result. No hidden coupling, no prose parsing for control flow.
3. **Deterministic control flow, nondeterministic content.** Orchestration is
   deterministic and journaled; the LLM fills in work inside a step.
4. **Durable & resumable.** Every completed step is checkpointed; a re-run is a
   replay, not a redo. Orchestration is a pure function of `(spec + journal)`.
5. **Verify, don't trust.** A step is "done" only when its acceptance gate
   passes; a child's result is verified before the parent proceeds.
6. **Uniform, identity-tagged observability.** Every unit emits structured
   events tagged with its identity and parent. Any subscriber reconstructs the
   tree from the stream.
7. **Isolation + bounds.** Each unit has its own context, sandbox, and budget;
   recursion is depth- and concurrency-capped.
8. **Stable, versioned, additive protocol.** The UDS wire format is the only
   coupling; new fields are additive and forward-compatible.

## User stories

### 1. Parent assigns a sub-workflow to a child (Stage A)
As an agent (or a deterministic orchestration script), I want to spawn a child
and hand it a specific workflow template with inputs and acceptance criteria, so
the child runs that sub-workflow autonomously and returns a typed verdict I can
branch on.

> spawn(template="feature", inputs={ issue: 712 }, acceptance="all hooks green",
>       budget=150k, max_depth=2)

### 2. Supervisor observes the whole tree (Stage B)
As a supervising agent — or the TUI, or a logger — I want to subscribe to one
event stream and reconstruct the full tree of units and each unit's workflow
progress, because every event carries `agent_id` and `parent_id`.

### 3. A crashed deep workflow resumes (Stage C)
As an operator, when a unit at depth 3 dies, I want the orchestration to resume
that unit from its last completed step using its journal, not restart the whole
tree.

### 4. A child's result is verified before the parent proceeds (Stage D)
As an orchestrator, I want a child's verdict checked against the step's
acceptance gate (optionally by an independent verifier unit) before the parent's
workflow advances, so plausible-but-wrong intermediate state cannot propagate.

### 5. Recursion stays bounded (Stage E)
As an operator, I want depth, concurrency, and token budget passed down and
enforced, so a runaway workflow cannot fork unboundedly or exhaust budget.

## Requirements

### Stage A — Spec-based assignment + typed result

> **Resolved decisions** (see "Design decisions" below): `workflow_spec` is
> **by-value** (carries the full inline template definition, not a template-id
> reference) and **binding** (an assigned child MUST run that template — no
> model-driven selection). The existing `config` parameter is retained but its
> role narrows: runtime/providers/model/context/isolation + *default* template
> library. It is no longer the mechanism for steering a child's workflow.

- **R-A1 — Workflow spec on spawn (by-value).** `SpawnTool` accepts an optional
  structured `workflow_spec`:
  `{ template (full inline template object), inputs (object),
  acceptance (string|object, optional), budget_tokens (int, optional),
  max_depth (int, optional) }`. The `template` is the complete definition
  (id/label/steps/phases), not an id reference, so assignment does not depend on
  the child's `config` template library.
- **R-A2 — Child instantiation from spec (binding).** When a `workflow_spec` is
  present, the child `quecto agent` initializes its `WorkflowEngine` directly in
  **Active** mode with the assigned template + inputs — it does NOT enter
  `SelectingTemplate` mode and cannot select a different template. The spec is
  passed on the child invocation (e.g. a `--workflow-spec <path>` file, since an
  inline template is too large for a bare CLI arg), consistent with how
  `--config` is forwarded today.
- **R-A3 — Typed result contract.** `agent_cmd await` returns a typed `result`
  verdict alongside the lifecycle status: `{ status, summary, workflow_progress
  { done, total } }`. The parent branches on `result.status` without parsing
  prose, which closes the "idle ≠ completed" gap from the Problem section.
  - **Status** (implemented, derived from lifecycle + the child's workflow
    snapshot): `completed` (workflow reached `complete`, or clean exit),
    `failed` (await/agent error or non-clean exit), `incomplete` (went idle or
    timed out without completing). `blocked` is reserved for Stage E
    (budget/depth bounds).
  - **`outputs`** (custom, child-declared) is deferred: it needs a child-side
    mechanism to set named outputs and is tracked as a follow-up, not part of
    the derived verdict.
  - This pass also fixed a latent bug: `await`'s workflow snapshot read the
    wrong keys (`steps_completed`/`steps_total`) while `get_state` emits
    `progress { done, total }`, so step counts were always 0; the snapshot now
    reads the real shape.
- **R-A4 — Backward compatibility.** `spawn` with no `workflow_spec` behaves
  exactly as today (boolean `workflow` / `workflow_guards` flag inheritance,
  model-driven template selection). The spec is purely additive.
- **R-A5 — `config` / `workflow_spec` boundary.** `config` and `workflow_spec`
  are orthogonal and may be combined: `config` supplies the child's runtime
  (providers, model, `max_context_tokens`, isolation, default template library);
  `workflow_spec` supplies the bound sub-workflow to run. `workflow_spec`'s
  `budget_tokens` (a task output-spend cap) is distinct from config's
  `max_context_tokens` (context-window size) and must not be conflated.

### Stage B — Identity-tagged event bus (push)

- **R-B1 — Identity on every workflow/subagent event.** `workflow_state` and the
  subagent event carry `agent_id` and `parent_id`. (`Unknown`/V1/V2 parsing keeps
  older consumers working.)
- **R-B2 — Children forward events to the parent.** A child's `workflow_state`
  changes are forwarded to the parent's event stream (push), so a parent/
  supervisor sees descendant workflows without polling each child socket.
- **R-B3 — `SubagentInfoEvent` gains `workflow` + `parent_id`.** The server
  `SubagentInfo` (`protocol.rs`) and the TUI mirror (`quecto-tui/.../client.rs`)
  add an optional `workflow` snapshot and `parent_id`. Additive; absent ⇒ render
  as today.
- **R-B4 — Tree reconstructable from the stream alone.** Given only the event
  stream, a consumer can build the unit tree and each unit's workflow state. No
  consumer-specific side channel is required.

### Stage C — Journaling + resume

- **R-C1 — Per-step journal.** The workflow engine appends a journal entry on
  each step transition (started/completed/failed) into the persisted
  `workflow_run`.
- **R-C2 — Resume from journal.** Re-instantiating a unit from `(spec + journal)`
  resumes at the first incomplete step; completed steps are not re-run.
- **R-C3 — Deterministic orchestration replay.** Orchestration decisions are a
  function of `(spec + journal)` only — no wall-clock/random inputs in the
  control path (mirrors the determinism constraints already used elsewhere in
  the harness).

### Stage D — Verification gates

- **R-D1 — Acceptance gate per step.** A template step may declare an
  `acceptance` gate (command/check or verifier spec). A step is "done" only when
  its gate passes.
- **R-D2 — Verify child results.** A parent step that consumes a child's result
  evaluates it against the step's acceptance gate before advancing; optionally
  via an independent verifier unit (adversarial check).

### Stage E — Isolation bounds

- **R-E1 — Budget propagation.** `budget_tokens` from the spec is enforced per
  unit and drawn from a shared pool across the tree; exhaustion fails the unit
  with `status: "blocked"` rather than silently continuing.
- **R-E2 — Depth + concurrency caps.** `max_depth` and a per-parent concurrency
  cap are enforced at `spawn`; exceeding them is a structured error, not a
  silent truncation.

## Proposed implementation model

### Spec (input contract — by-value, binding)
```jsonc
// spawn(workflow_spec = { ... })
{
  // Full inline template definition (NOT an id reference) — same shape as a
  // workflow-config.json template entry. The child runs exactly this; it cannot
  // select another. This makes the spawn self-contained (no config dependency).
  "template": {
    "id": "review-pr",
    "label": "Review PR",
    "steps": [
      { "key": "analyze", "label": "Analyze the diff", "phase": "review" },
      { "key": "verify",  "label": "Run tests",        "phase": "review" }
    ]
  },
  "inputs":   { "pr": 712 },      // free-form, template-defined
  "acceptance": "all tests pass", // optional
  "budget_tokens": 150000,        // optional; task output-spend cap (≠ context size)
  "max_depth": 2                  // optional; remaining recursion budget
}
```

### Result (output contract)
```jsonc
{
  "status": "completed",          // completed | failed | blocked
  "summary": "Implemented #712; 14/14 steps; hooks green.",
  "outputs": { "pr": 731 },       // template-defined
  "workflow_progress": { "done": 14, "total": 14 }
}
```

### Event tagging (push)
```jsonc
// workflow_state and subagent events gain identity:
{ "type": "workflow_state", "agent_id": "...", "parent_id": "...", "steps": [...], "progress": {...} }
```

### Observability: poll vs push (decision)
The earlier quecto-tui increment chose an on-demand **poll** of each child's
`get_state` because it required no server changes. At the *harness* level the
best-practice choice is **push** (R-B2): children forward identity-tagged events
up the bus, so observability is uniform across all consumers and no consumer is
privileged. **This PRD supersedes the poll approach for anything beyond the
already-shipped TUI increment.** The poll may remain as a fallback for a child
that predates event forwarding.

## Relationship to prior work

- `docs/workflow-availability-prd.md` defines single-agent launch modes
  (normal / `--workflow` / `--no-workflow`). This PRD is orthogonal and additive:
  spec-based assignment is a new spawn capability, not a change to those modes.
- quecto-tui already shipped: animated sub-agent spinner + elapsed time +
  aggregate header, and a generalized phase-pill overview in the live workflow
  widget. Those are pure consumers of harness state. Once Stage B lands, the TUI
  renders the per-sub-agent workflow track from the identity-tagged stream
  instead of polling — no further TUI protocol assumptions required.

## Acceptance criteria

1. `spawn` accepts a `workflow_spec`; a child launched with one starts the named
   template with the given inputs (no selector mode) and does not require
   `--workflow` to be set separately.
2. `spawn` with no `workflow_spec` is byte-for-byte equivalent to today's
   behavior (regression-tested).
3. A parent awaiting a spec-driven child receives a typed result document and can
   branch on `status` without parsing prose.
4. Every `workflow_state` and subagent event carries `agent_id` and `parent_id`;
   a test reconstructs a 2-level unit tree purely from a captured event stream.
5. A child's workflow changes appear on the parent's stream without the parent
   polling the child's socket.
6. Re-instantiating a unit from a persisted `(spec + journal)` resumes at the
   first incomplete step; completed steps are not re-executed.
7. A step with an acceptance gate is not marked done until the gate passes; a
   parent step does not advance on an unverified child result.
8. `max_depth` and `budget_tokens` are enforced at `spawn`; exceeding either
   yields a structured `blocked`/error result, logged, never a silent stop.
9. Forward compatibility: a consumer that ignores the new fields (older TUI,
   logger) continues to function; missing fields render/behave as before.
10. Existing workflow, spawn, and protocol tests continue passing, with new
    coverage for spec assignment, event identity, journaling/resume, and bounds.

## Design decisions (resolved)

- **By-value, binding template.** `workflow_spec.template` carries the full
  inline template definition, not an id reference, and the assigned child MUST
  run it (no `SelectingTemplate`). Rationale: makes the spawn self-contained per
  the "contract in" principle and removes the dependency on the child's `config`
  template library. Trade-off accepted: larger spawn payloads.
- **`config` retained, role narrowed.** `config` continues to supply the child's
  runtime (providers/model/context/isolation + *default* library); it is no
  longer the workflow-steering mechanism. `workflow_spec` and `config` compose.

## Open questions

1. **Result schema breadth.** Is a minimal `{ status, summary, outputs,
   workflow_progress }` enough, or should the verdict include per-step results /
   artifacts by reference?
2. **Inputs typing.** Should template `inputs` be schema-validated against a
   template-declared input schema at `spawn` time, or remain free-form initially?
3. **Budget pool semantics.** Shared global pool vs per-subtree allotment — and
   behavior on exhaustion (block vs degrade vs ask parent).
4. **Verifier independence.** Is an independent verifier *unit* required for
   R-D2, or is an in-unit gate acceptable for the first cut?
5. **Event forwarding fan-in.** Do grandchild events flow through each
   intermediate parent, or to a tree-root bus directly? (Affects ordering and
   backpressure.)
