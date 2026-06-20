# Proposal: a knowledge-retrieval surface (generalize `docs`; fold skills in)

Status: **Proposed** (design note — not yet a commitment). Supersedes the
"Skills" surface as described in [`kernel-boundary.md`](kernel-boundary.md) if
adopted.

## Context

The kernel has four extension surfaces: skills, models, workflows, tools. On
review, **skills overlap the other surfaces** and offer little that's distinct:

- For *procedural* guidance, a workflow is strictly better — it has state,
  guards, and sequencing; a skill is just static prompt text with none of that.
- For *ambient* text, the system prompt already does that; a skill today is
  effectively a **modular chunk of system prompt** (full `SKILL.md` is
  concatenated in at startup — no progressive disclosure, and it doesn't
  auto-load, so a self-installed skill needs a process restart).

The **one** thing skills uniquely want to be is an **on-demand knowledge
library**: many entries, model-selected by relevance, body read only when
needed. But that is exactly the pattern the kernel already ships as the **`docs`
tool** (curated kernel docs, name → body on demand) and that **Context7** proves
works as an external **MCP tool** over a library corpus with *zero kernel code*.

In other words: "skills with progressive disclosure" is not a fourth surface —
it's the `docs` tool, generalized.

## Decision (proposed)

Generalize the `docs` tool into a **knowledge-retrieval surface** with
pluggable, optionally-runtime **sources**:

1. **Embedded kernel docs** — today's `EMBEDDED_DOCS` (compile-time, curated).
2. **Folder** — index a directory of markdown at runtime (e.g.
   `workspace/skills/`, `workspace/knowledge/`).
3. **Graph / database** — index a knowledge graph or DB table.
4. **Remote** — a Context7-style source (delegated to an external tool/MCP
   backend, not compiled into the kernel).

Properties:

- **Snippet-level, progressive disclosure by construction.** The index exposes
  *names + descriptions* cheaply; bodies (or sub-snippets) are fetched only on
  demand. Scales to large corpora without prompt bloat.
- **Skills become one folder-backed source**, not a separate surface. The
  startup skills loader and unconditional prompt injection are retired; the
  `quecto skills install/list/remove` ergonomics can remain as a thin *curator*
  over the same folder.
- **Two consumption modes:** (a) **one-off** — the model pulls a snippet for the
  current turn; (b) **persisted** — a workflow step references/pulls a knowledge
  snippet, so *process* (workflows) and *knowledge* (this tool) compose.
- **Keep a small always-on core** in the system prompt (push) for must-know
  guidance (safety rules, house invariants) — because the model can't *pull*
  what it doesn't know exists. Everything else is pull.

Resulting taxonomy (one surface fewer):

| Concern | Surface |
|---|---|
| Must-always-know | System-prompt core (push) |
| Knowledge, on demand | **Knowledge-retrieval tool** (pull) ← docs + skills + Context7 |
| Stateful, enforced process | Workflows |
| Action / external systems | Tools (UDS / MCP) |

## Consequences

- **Closes the autonomy gap.** A runtime-indexed source re-reads on each call, so
  a self-installed skill/knowledge doc is usable on the **next turn with no
  restart** — unlike today's startup-loaded skills (see the autonomy contract in
  `kernel-boundary.md`).
- **Progressive disclosure for free** — retrieval is inherently on-demand; no
  bespoke loader needed.
- **Smaller kernel** — four surfaces → three + an always-on core, in the spirit
  of "smallest useful kernel" (the same lens that retired `MemoryStore` and the
  onboarding templates).
- **One contract** unifies internal docs, community knowledge, and external
  (Context7) retrieval.

## Trade-offs / caveats

- **Pull vs push.** Retrieval requires the model to know to look; mitigate with
  the always-on core and high-quality index descriptions.
- **Retrieval quality.** A folder source can be simple name/keyword match;
  graph/db/remote sources buy relevance (RAG/embeddings) at the cost of more
  moving parts — which is why those richer backends belong **outside** the kernel
  (external tool/MCP), with the kernel shipping the folder + embedded sources.
- **Migration.** Existing `workspace/skills/` keeps working as a folder source;
  no content changes required.

## Open questions

- **Source contract:** what does a source implement — `list()` + `fetch(name)` +
  `search(query)`? Is `search` optional (folder = list+fetch only)?
- **Snippet granularity:** whole doc vs section vs embedding-chunk.
- **Workflow ↔ knowledge wiring:** how does a step reference a snippet — a query
  in the step guidance, or a new step field?
- **Relationship to recall/spill:** is retrieved knowledge spillable/recall-able
  like tool output, or a separate channel?

## Alternatives considered

- **Keep skills + build a bespoke progressive-disclosure loader.** Still a
  separate surface; doesn't unify with `docs`/Context7; doesn't fix auto-load.
- **Drop skills entirely; rely on Context7/MCP.** Loses the zero-config
  "drop a file in a folder" ergonomics and kernel-curated internal docs.
