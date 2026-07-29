# ADR-0005 — Knowledge as a Retrieval Surface

**Status:** Accepted.

**Implementation status:** Partially implemented.

## Context

The kernel already has a `docs` tool: a curated library whose names are
discoverable and whose bodies are fetched on demand. Context7 demonstrates the
same pattern over an external corpus via MCP. The proposed knowledge-retrieval
surface generalizes this into one mechanism for embedded docs, community files,
knowledge graphs/databases, and remote sources.

## Decision

Make **knowledge retrieval** the canonical knowledge surface. The kernel owns the
retrieval contract and a small always-on index; community content and rich backing
stores live outside the kernel.

- Kernel-owned sources: embedded kernel docs and a simple folder-backed markdown
  source, e.g. `workspace/knowledge/`.
- External-tool sources: graph/database/remote retrieval are **UDS/MCP tools**,
  not in-kernel graph engines.
- Retrieval is progressive-disclosure by construction.
- The always-on bootstrap in the system prompt stays deliberately tiny.

## Current Implementation

- Embedded kernel docs are implemented through the `docs` tool.
- Folder-backed `workspace/knowledge/` retrieval is not implemented.
- Graph/database/remote retrieval remains external-tool territory.

## Consequences

- *Positive:* unifies embedded docs, file-backed knowledge, Context7-style remote
  docs, and knowledge graphs behind one contract.
- *Negative / risk:* retrieval quality becomes load-bearing.
- *Boundary:* a knowledge graph is a **tool-backed source**; the kernel does not
  own graph schema, embeddings, RAG, ranking, or storage.

## Alternatives Considered

- **A. Put a graph database / embedding index in the kernel.** Rejected because
  storage and ranking are dependency-heavy and community-specific.
- **B. Rely only on external MCP/Context7.** Rejected because it loses zero-config
  embedded docs and local community files.
- **C. Inject all knowledge summaries into the system prompt.** Rejected because
  prompt budget scales with corpus size.

## Related

- [Kernel boundary](../architecture/kernel-boundary.md)
- [ADR-0002](adr-0002-reload-trigger-for-startup-loaded-surfaces.md)
