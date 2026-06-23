# ADR-0004 — Dissolve the Skills Surface

**Status:** Accepted.

**Implementation status:** Implemented for prompt injection removal; compatibility curation remains.

## Context

The Skills surface (`workspace/skills/<name>/SKILL.md`, installed by
`quecto skills`) was originally the community's file-based capability surface. On
review, it bundles three concerns that now have better homes:

- **Procedure** belongs in workflows.
- **Knowledge** belongs in generalized `docs`/knowledge retrieval.
- **Persona / handoff context** belongs in community sub-agent definitions over
  the existing `spawn` mechanism.

The skill loader injects full skill bodies into the system prompt at startup,
which consumes prompt budget proportional to library size and is invisible to a
self-extending agent until restart.

## Decision

Dissolve **Skills** as a distinct kernel extension surface.

- Procedural skills become **workflow templates**.
- Knowledge skills become entries/sources in the **knowledge-retrieval surface**.
- Task/persona bundles become **community sub-agent definitions** passed to
  existing `spawn` as `system` + `workflow_spec` + optional knowledge scope.
- `quecto skills install/list/remove` may remain temporarily as compatibility
  curation over the knowledge folder, but not as a prompt-injection surface.

## Current Implementation

- Skills are still represented in domain and persistence code for temporary
  compatibility curation.
- `workspace/skills/` is no longer loaded into the system prompt at startup.
- The replacement knowledge/workflow/sub-agent-definition migration is not fully
  implemented.

## Consequences

- *Positive:* one fewer conceptual surface; no prompt bloat; no bespoke skill
  reload path.
- *Negative / migration cost:* existing `workspace/skills/` content and commands
  need a migration story.
- *Autonomy:* the intended auto-load path is through knowledge retrieval and
  ADR-0002, not prompt reinjection.

## Alternatives Considered

- **A. Keep Skills and add progressive disclosure.** Rejected because it recreates
  generalized docs/knowledge under another name.
- **B. Keep Skills for small procedures.** Rejected because ordering/gates imply a
  workflow; advice is knowledge.
- **C. Keep startup prompt injection and add auto-reload.** Rejected because it
  does not solve prompt bloat.
- **D. Delete skills with no compatibility path.** Rejected because existing
  content should continue where possible.

## Related

- [Kernel boundary](../kernel-boundary.md)
- [Knowledge retrieval surface](../knowledge-retrieval-surface.md)
- [Workflow](../workflow.md)
- [Subagents](../subagents.md)
