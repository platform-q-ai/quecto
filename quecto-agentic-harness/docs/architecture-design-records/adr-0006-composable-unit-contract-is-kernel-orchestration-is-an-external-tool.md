# ADR-0006 — Composable Unit Contract Is Kernel; Orchestration Is an External Tool

**Status:** Accepted.

**Implementation status:** Implemented.

## Context

Quecto's recursive unit contract is: sub-agents are the same binary over the same
protocol as parents; a parent can spawn a child with a by-value, binding
`workflow_spec`; children return typed results; identity-tagged events let any
consumer reconstruct the tree. The spec-based assignment, typed `agent_cmd await`
result, and identity-tagged event bus have shipped; per-unit journaling/resume,
verification gates, and isolation bounds (budget/depth/concurrency) remain planned
kernel work. Separately, a **taskgraph** (DAG / fan-out / fan-in / dependency
orchestration) is useful, but putting a graph engine into the kernel would enlarge
the workflow engine and introduce a privileged orchestrator role.

## Decision

Keep the **composable unit contract** in the kernel; implement **taskgraph / DAG
orchestration** as an external tool.

- Kernel-owned: `spawn`, `workflow_spec`, typed `agent_cmd await` result,
  identity-tagged event bus, per-unit workflow engine, and future per-unit
  journaling/resume/verification/bounds.
- Tool-owned: graph construction, dependency scheduling, fan-out/fan-in policy,
  and which sub-agent definitions to use for each node.
- A taskgraph tool drives ordinary quecto units by calling existing surfaces
  (`spawn`, `agent_cmd`, workflow specs, tools). It is not a privileged agent
  type and does not bypass kernel bounds.
- Per-node procedure remains a **workflow**. The taskgraph decides *which units
  exist and their dependencies*; each unit's internal step sequence is handled by
  the kernel workflow engine.

## Consequences

- *Positive:* complex orchestration is possible without bloating the kernel; the
  graph layer can evolve independently as a UDS/MCP tool; the kernel retains the
  safety invariants (typed results, event identity, bounds, journaling).
- *Negative / cost:* graph-level semantics are not kernel-standardized at first;
  different taskgraph tools may make different scheduling choices. That is
  acceptable because the kernel-standard contract is the unit boundary, not the
  graph language.
- *Enforcement line:* the taskgraph tool may decide the graph, but the kernel
  enforces per-unit constraints (budget/depth/concurrency), journaling/resume,
  and verification gates once those capabilities land. A tool cannot opt out by
  claiming to be an orchestrator.

## Alternatives Considered

- **A. Put a taskgraph/DAG engine in the kernel workflow engine.** *Rejected:* it
  expands the kernel from linear per-agent workflows into multi-agent scheduling,
  duplicates what external tools can do, and conflicts with "smallest useful."
- **B. Create a privileged orchestrator agent type.** *Rejected:* violates the
  composable-units invariant that every node is the same quecto unit.
- **C. Let model free-choice control orchestration.** *Rejected:* non-deterministic
  control flow is not replayable. Deterministic orchestration belongs in a script
  or tool; the model supplies content inside a unit.
- **D. Treat taskgraph as a new first-class extension surface.** *Rejected:* it is
  just a tool consumer of existing kernel mechanisms; no new surface is needed.

## Related

- [Kernel boundary](../kernel-boundary.md)
- Surface #4: tools/extensions
- Workflow surface #3
- ADR-0002: reload trigger for startup-loaded surfaces
