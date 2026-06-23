# Quecto Kernel Boundary

This document defines the conceptual boundary between the **quecto kernel** and
the community ecosystem around it.

The kernel is the smallest semver-stable binary the core team owns. Community
content is everything that can be added, changed, shared, or removed at runtime
without editing or rebuilding that binary.

> Quecto is the smallest useful, recursive kernel. The core team maintains the
> kernel. The community builds and shares knowledge, workflows, models, and tools
> on top of it.

## Core Standard

Everything the community could reasonably want must be reachable through a
runtime surface: config, files, or an external process. If a capability requires
editing the binary, it belongs either in the kernel roadmap or behind one of the
runtime surfaces.

The kernel's job is to stay small and have no walls.

## Boundary Principles

- **Runtime before recompile:** community extension must be possible without a
  new binary.
- **Live before restart:** new or changed runtime content should become usable on
  the next turn where feasible.
- **Data outside, contracts inside:** community content is data, configuration,
  or external processes; the kernel owns the stable contracts that load and run
  them.
- **Protocols are kernel-owned:** wire protocols, tool protocols, and execution
  contracts are part of the stable kernel surface.
- **Policy is external by default:** orchestration, ranking, graph policy,
  storage choices, and domain-specific behavior should live outside the kernel
  unless they are required for the minimal recursive loop.
- **No dead surfaces:** a documented extension point must be wired into runtime
  behavior. Dormant or aspirational surfaces should be removed or captured as an
  ADR/proposal.

## Runtime Surfaces

Quecto exposes four community extension surfaces.

### Knowledge

Knowledge is external reference material made available through retrieval rather
than pasted wholesale into prompts.

The kernel owns the retrieval contract, discovery rules, and any minimal
bootstrap documentation needed for the agent to understand itself. The community
owns knowledge content: markdown folders, migrated skill/reference material,
project notes, graph-backed sources, database-backed sources, and remote sources.

Graph, database, embedding, ranking, and remote retrieval systems are tools or
external services, not kernel features.

### Models

Models are runtime-selectable LLM targets reachable through kernel-owned wire
protocols.

The kernel owns the supported provider protocols, streaming behavior, and the
`LlmProvider` contract. The community owns provider endpoints, model names,
model metadata, credentials, and deployment-specific configuration.

Adding a model should not require a kernel change when it speaks an existing
kernel-owned protocol. Adding a new wire protocol is a kernel change.

### Workflows

Workflows are data-defined procedures that guide agent execution.

The kernel owns the generic workflow engine, execution contract, and built-in
tool actions. The community owns workflow templates, step structure, guidance,
guards, sub-agent definitions, and domain-specific procedures.

New workflow content should be expressed as templates or specs. New engine
semantics, approval primitives, or built-in workflow actions are kernel changes.

### Tools

Tools are external capabilities exposed across a process boundary.

The kernel owns core tools, the UDS tool-registration protocol, tool routing, and
the stable `Tool` contract. The community owns external tool processes, MCP
servers, taskgraph tools, knowledge-graph tools, domain integrations, and local
automation.

Tools are the universal escape hatch: if a capability can be implemented by a
program or service, it should not require a kernel change.

## In The Kernel

The kernel includes only the stable machinery required for the recursive agent:

- Agent loop and context management.
- Core tools required for basic operation.
- UDS protocol and tool registration.
- Sub-agent spawn and unit execution contract.
- Generic workflow engine.
- Kernel-owned LLM wire protocols and provider trait.
- Registries, discovery, and reload mechanisms for runtime surfaces.
- Minimal embedded documentation and knowledge-retrieval contract.

Kernel code is team-owned, versioned with the binary, and treated as a stable
contract.

## Out Of The Kernel

Community content must remain outside the binary:

- Knowledge files and external knowledge sources.
- Workflow templates and sub-agent definitions.
- Provider endpoints, model entries, and model metadata.
- UDS clients, MCP servers, and other external tools.
- Taskgraph, knowledge graph, storage, ranking, and orchestration policy.

Community content may be large, project-specific, experimental, or private. It
should not increase binary footprint or require a kernel release.

## What Forces A Kernel Change

A change belongs in the kernel only when it changes a stable contract or the
minimal recursive runtime:

- A new LLM wire protocol beyond the kernel-owned protocols.
- A new core tool or a change to tool routing / the `Tool` contract.
- A UDS protocol change.
- New workflow engine semantics or built-in workflow actions.
- Changes to the agent loop, context management, or sub-agent execution contract.
- New discovery/reload behavior that applies across runtime surfaces.

If the same outcome can be achieved as knowledge, workflow data, model config, or
an external tool, it should stay out of the kernel.

## Autonomy Standard

Runtime surfaces should support self-extension: an agent should be able to add or
modify community content and use it on a later turn without human intervention.

The preferred implementation standard is:

- Detect changes at the top of a turn or when a surface is consumed.
- Gate reload work by mtime/hash or an equivalent cheap change detector.
- Preserve last-good state when new content is invalid.
- Make failures visible without breaking unrelated surfaces.

Restart-only extension is acceptable only as a temporary limitation, not as the
target design.

## Governance

The four runtime surfaces are the public contract between the kernel and the
community. They must be documented, versioned with the kernel, and discoverable
from inside the harness.

Significant boundary decisions are recorded as ADRs. ADRs explain why a boundary
exists, what alternatives were rejected, and what consequences the team accepts.
They are append-only: supersede rather than rewrite.

## Architecture Decision Records

- [ADR-0001 — Wire protocols stay kernel-owned](architecture-design-records/adr-0001-wire-protocols-stay-kernel-owned.md)
- [ADR-0002 — Reload trigger for startup-loaded surfaces](architecture-design-records/adr-0002-reload-trigger-for-startup-loaded-surfaces.md)
- [ADR-0003 — UDS `register_provider` for dynamic model/provider registration](architecture-design-records/adr-0003-uds-register-provider-for-dynamic-model-provider-registration.md)
- [ADR-0004 — Dissolve the Skills surface](architecture-design-records/adr-0004-dissolve-the-skills-surface.md)
- [ADR-0005 — Knowledge as a retrieval surface](architecture-design-records/adr-0005-knowledge-as-a-retrieval-surface.md)
- [ADR-0006 — Composable unit contract is kernel; orchestration is an external tool](architecture-design-records/adr-0006-composable-unit-contract-is-kernel-orchestration-is-an-external-tool.md)
