# Quecto Kernel Boundary

This document defines what lives **inside the quecto kernel** (maintained by the
core team, shipped as a compiled binary, semver-stable) versus what is
**community content** (added at runtime, with no recompile). It is the contract
that makes quecto's model possible:

> Quecto is the smallest useful, recursive kernel. The core team maintains the
> kernel. The community builds and shares **skills, workflows, models, and
> extensions** on top of it — without ever editing or rebuilding the binary.

Open source, with contributions closed initially. Because the community cannot
send patches *and* may only have the binary, the binding rule is:

> **Everything the community could want must be reachable through a runtime
> surface — config, files, or an external process — never through the binary.**
> The kernel's job is to be *small* **and** to have *no walls*.

---

## The two universal escape hatches

These are why "no walls" holds even with a binary-only release:

1. **Process-boundary tools.** A tool can be *any external process* that speaks
   the UDS protocol (`register_tools`), and `quecto-mcp` bridges *any* MCP
   server's tools with zero code. So the community can add anything a program or
   MCP server can do — no recompile.
2. **Skills.** Pure instructions/data dropped in `workspace/skills/`. So the
   community can add any capability or guidance — no recompile.

Anything not yet covered by a first-class surface can be bridged by one of these
until the team ships a kernel update.

> **Decisions are recorded as ADRs.** Significant, hard-to-reverse boundary
> choices — what stays in the kernel and why, and which alternatives were ruled
> out — live in [Architecture Decision Records](#architecture-decision-records-adrs)
> at the end of this doc. Inline sections state *what* is true today; the ADRs
> capture *why* and *what was rejected*.


---

## The extension surfaces (definitive status)

ADR-0004 dissolves **Skills** as a standalone surface: procedure belongs in
workflows; knowledge belongs in the generalized `docs`/knowledge-retrieval tool;
sub-agent persona/procedure bundles are community data over the existing `spawn`
mechanism. The active surfaces are therefore **Knowledge, Models, Workflows, and
Tools**.

Each surface is rated on **two independent axes**, because "the community can do
it without a recompile" is not the same as "the agent can do it without a human":

- **Capability** — can community content do this *at all* with no recompile?
- **Auto-load (autonomy)** — does new or changed content take effect on the
  **next turn**, with **no human restarting the quecto process**? This is the
  autonomy contract (see below). A surface that needs a restart silently puts a
  human back in the loop, which is fatal for a self-extending agent.

Legend: ✅ full · 🟡 partial · ❌ missing.

| Surface | Capability (no recompile) | Auto-load on next turn (no restart) |
|---|---|---|
| 1. Knowledge retrieval | 🟡 **partial** — embedded docs + planned folder source; graph/remote sources via external tools | 🟡 **partial** — embedded docs live; runtime folder/graph sources need the shared reload/index path |
| 2. Models / providers | 🟡 partial (OpenAI/Anthropic today; native Gemini to add; community adds compatible models) | ❌ **missing** — config/model registry read once at startup |
| 3. Workflows | 🟡 partial (config templates; engine kernel-owned) | 🟡 **partial** — selecting/advancing is live; *adding* a template needs reload |
| 4. Tools / extensions | 🟡 partial (external process only; taskgraph/knowledge-graph are tools) | ✅ **full** — UDS `register_tools` is live next turn |

Only surface 4 (tools, via UDS) currently fully meets the autonomy bar. Knowledge
(folder source), models, and workflow-template sets need the shared ADR-0002
reload path before an agent can extend them *itself* mid-run.

### 1. Knowledge retrieval — capability 🟡 · auto-load 🟡
- **Implemented today:** the kernel ships the embedded `docs` tool — curated
  kernel documentation, names discoverable and bodies fetched on demand. This is
  the reference pattern for progressive disclosure.
- **Accepted direction (ADR-0005):** generalize `docs` into the knowledge surface:
  embedded docs + folder-backed markdown (including legacy `workspace/skills/` and
  `workspace/knowledge/`) + graph/database/remote sources behind external
  UDS/MCP tools. The kernel owns the retrieval contract and a tiny always-on
  bootstrap; community owns the knowledge content.
- **Skills dissolved (ADR-0004):** `workspace/skills/` is no longer a distinct
  surface. Existing skill content migrates to folder-backed knowledge (for facts /
  references) or workflow templates (for procedure). `quecto skills` may remain
  temporarily as a compatibility curator over the knowledge folder, not as a
  prompt-injection surface.
- **Missing / partial:** folder-backed runtime indexing, snippet-level search, and
  source reload still need implementation. Graph/DB/remote retrieval is **not**
  in-kernel; it is supplied by external tools (surface #4), e.g. a knowledge-graph
  tool exposing `list/search/fetch`.

### 2. Models / providers — capability 🟡 · auto-load ❌
- **Implemented today (full within scope):** `providers.openai_compatible.endpoints`
  (`prefix`, `api_base`, `api_key`, `allow_remote_http`). Any model string, **no
  allowlist** — covers Ollama, vLLM, LM Studio, Fireworks, Groq, DeepSeek, and
  most hosted endpoints. Graceful degradation: unknown-model metadata (pricing,
  thinking, context window) is absent but never blocks the request.
- **Partial:** only OpenAI/Anthropic-shaped wire protocols.
- **Missing (capability):** a genuinely new *wire protocol* (native Gemini /
  Cohere) or OAuth on a custom provider forces a recompile; a config-driven
  model-metadata registry (correct context-window/pricing/capability) does not
  exist yet.
- **Missing — auto-load ❌:** providers come from the config file, read **once at
  startup** (`Config::load`). A provider or model added to config mid-session is
  **not usable until restart**. To close: re-read the providers section of config
  at the start of each turn (or on a reload signal) and rebuild the provider set.
- **Decision — wire protocols stay kernel-owned (3 robust impls):** the kernel
  owns exactly three wire/streaming protocols (`openai-completions`,
  `anthropic-messages`, native `google-generative-ai`); the community extends
  *models* on top of them, never the protocols. Full rationale, consequences, and
  the alternatives that were ruled out are recorded in
  [**ADR-0001**](#adr-0001--wire-protocols-stay-kernel-owned) below. See also
  [prd-models-runtime-extensible.md](prd-models-runtime-extensible.md).


### 3. Workflows — capability 🟡 · auto-load 🟡
- **Implemented today (full):** `workflow.templates` in config — arbitrary
  steps/keys/phases/guidance/guards; the engine is generic (no hardcoded step
  keys; unknown phases pass through). A community config's `templates` **replace**
  the kernel defaults; the kernel keeps the `feature` template only as a
  reference/internal-dev example that steps aside the moment a community config
  provides its own. Selecting a template and checking/skipping steps via the
  `workflow` tool is fully live each turn.
- **Missing (capability):** new workflow *tool actions* (beyond
  check/skip/select/guards) or new engine semantics (approval gates, conditional
  branches, multi-actor execution) force a recompile.
- **Partial — auto-load 🟡:** runtime *use* of the loaded templates needs no
  restart, but the template **set** is fixed at startup from config. Adding or
  editing a template (a config change) is **not picked up until restart**. To
  close: re-read `workflow.templates` from config on the same reload path as the
  other surfaces, so a new template is selectable next turn.

### 4. Tools / extensions — capability 🟡 · auto-load ✅
- **Implemented today (auto-load ✅, the reference model):** UDS `register_tools`
  (any language) — an external process registers tools into the **live** registry
  and they are in the tool list on the **next turn**, no restart
  (`src/infrastructure/tools/registry.rs::register_extension`). The `quecto-mcp`
  sidecar bridges any MCP server's tools the same way, zero code. Core tools
  (`read`/`write`/`edit`/`bash`/`grep`/`find`/`ls`/`docs`) are always present,
  disable per-run with `--disable-tool`, and cannot be shadowed.
- **Named external-tool consumers (accepted boundaries):**
  - A **knowledge graph** is a tool-backed source for the knowledge-retrieval
    surface (ADR-0005), exposing `list/search/fetch` over UDS/MCP. The kernel does
    not own graph schema, embeddings, RAG, ranking, or storage.
  - A **taskgraph** is a tool that performs DAG/fan-out/fan-in orchestration over
    ordinary quecto units (ADR-0006), driving `spawn` / `workflow_spec` /
    `agent_cmd await`. The kernel owns the unit contract and enforces per-unit
    bounds; the tool owns graph policy.
- **Partial (capability):** extension tools must be an *external process* — there
  is no in-binary authoring and no extension SDK/template yet.
- **Missing:** folder/manifest discovery (drop a file → registered). Script
  extensions were removed in #353 and `reload_extensions` is now a **no-op** — so
  "auto-load" here requires a *running process* that calls `register_tools`, not
  merely a file on disk. A new *core* tool, tool-routing change, or `Tool`-trait
  change still forces a recompile.
- **Why this is the bar:** because registration targets the live registry rather
  than a startup scan, this surface already satisfies the autonomy contract. The
  other three surfaces should converge on the same "mutate live state, effective
  next turn" model.

---

## The autonomy contract: auto-load on the next turn

Quecto's recursion depends on an agent extending **itself** — installing a skill,
adding a provider, defining a workflow, registering a tool — and then *using* that
extension on its next turn, with **no human restarting the process**. The moment a
surface requires a restart to take effect, a human (or an external supervisor) is
back in the loop and unattended autonomy breaks.

Status today:

- **Tools (UDS/MCP): ✅** content goes into live state and is effective next turn.
- **Knowledge folder sources, models, workflows-set: ❌/🟡** content is startup-
  or construction-loaded today (model/provider config; workflow template set;
  future folder-backed knowledge indexes), so a self-made change is invisible
  until restart unless it is expressed as a live tool.

ADR-0002 defines the shared reload mechanism that closes this gap: **top-of-turn
reload + on-consume reload, gated by mtime/hash, fail-safe last-good state**.
Consumers:

- re-index folder-backed knowledge sources (including legacy `workspace/skills/`);
- re-read the model registry/provider sources and rebuild the provider/model set;
- re-read `workflow.templates` and rebuild the selectable template list;
- optionally discover tool manifests (drop a file → start/register a process) as a
  future complement to live `register_tools`.

Until those consumers are implemented, the only fully-autonomous escape hatch is
to express a new capability as a **UDS/MCP tool** (the one live surface) — or to
accept that a human must restart quecto for non-tool content to land.

---

## In the kernel (team-owned, semver-stable)

- The agent loop and context management.
- Core tools: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`, `docs`.
- The UDS protocol and sub-agent spawn/orchestration (quecto's recursion).
- The workflow engine.
- The three kernel-owned LLM wire protocols (`openai-completions`,
  `anthropic-messages`, `google-generative-ai`) and the `LlmProvider` trait.
- The registries and discovery/reload mechanisms for knowledge, workflows, models,
  and tools.
- The embedded `docs` capability tool and folder-backed knowledge retrieval
  mechanism.

## Out of the kernel (community content, no recompile)

- Knowledge content (`workspace/knowledge/`, legacy `workspace/skills/`, graph /
  DB / remote sources behind UDS/MCP tools).
- Workflow templates and sub-agent definitions (system prompt + workflow spec +
  optional knowledge scope) as files/config.
- Provider endpoints, model registry entries, and model metadata (`models.json` /
  config).
- Extension tools (UDS clients / MCP servers), including taskgraph and
  knowledge-graph tools.

## What forces a kernel change (deliberately the team's domain)

- A new LLM **wire protocol** beyond the three the kernel owns
  (`openai-completions`, `anthropic-messages`, `google-generative-ai`). Adding or
  changing a wire protocol is the team's job; the community extends *models* on
  top of these three, not the protocols. See the Models/providers decision above.
- A new **core tool**, tool-routing, or `Tool`-trait change.
- New **workflow tool actions** or **engine semantics** inside the kernel. A DAG /
  taskgraph orchestrator is *not* a workflow-engine change when implemented as an
  external tool over `spawn` + `workflow_spec` (ADR-0006).
- A **UDS protocol** change.

These are the "kernel" in "smallest useful kernel" — they're correctly ours to
own and keep stable.

---

## Discovery & footprint

- **Config:** a single `~/.quecto/config.json` (or `--config` /
  `QUECTO_BASE_DIR`). *Roadmap:* per-project + folder discovery, so every
  file-backed surface follows "drop a file in a known dir → it's picked up."
- **Knowledge:** `workspace/knowledge/` folder scan, with legacy
  `workspace/skills/` indexed as a compatibility source during migration.
- **Models:** `~/.quecto/models.json` (plus config/credential store for keys),
  reloaded by the shared ADR-0002 mechanism.
- **Sub-agent definitions:** community files (location TBD) that bundle a system
  prompt + workflow spec + optional knowledge scope, consumed by parents/tools and
  passed to existing `spawn`.
- **Tools:** live UDS/MCP `register_tools`; future folder/manifest discovery may
  start/register external processes via the same reload mechanism.
- **Footprint:** community content is data or external processes — never compiled
  in — so the binary stays flat no matter how large the ecosystem grows.
  Isolation is delegated to the container rather than carried in-process (nsjail
  removed), keeping the kernel lean.

---

## Governance

With closed contributions and a binary-only release, the four surfaces above are
the **public contract**: they must be documented (this file + `docs/`),
versioned with the kernel, and discoverable to the in-harness LLM via the `docs`
tool. When a surface is missing, the escape hatches (process tools, skills)
bridge the gap until the team ships a kernel update.

---

## Architecture Decision Records (ADRs)

ADRs capture **significant, hard-to-reverse decisions** about the kernel
boundary: the context that forced a choice, the decision itself, its
consequences, and — critically — the **alternatives considered and why they were
rejected**. They are append-only: supersede rather than rewrite. Each has a
stable id so other docs can link to it.

**Format:** Status · Context · Decision · Consequences · Alternatives considered
(rejected). **Statuses:** Proposed · Accepted · Superseded by ADR-XXXX ·
Deprecated.

### Index
- [ADR-0001 — Wire protocols stay kernel-owned (three robust impls)](#adr-0001--wire-protocols-stay-kernel-owned)
- [ADR-0002 — Reload trigger for startup-loaded surfaces](#adr-0002--reload-trigger-for-startup-loaded-surfaces)
- [ADR-0003 — UDS `register_provider` for dynamic model/provider registration](#adr-0003--uds-register_provider-for-dynamic-modelprovider-registration)
- [ADR-0004 — Dissolve the Skills surface](#adr-0004--dissolve-the-skills-surface)
- [ADR-0005 — Knowledge as a retrieval surface (graph/remote sources are external tools)](#adr-0005--knowledge-as-a-retrieval-surface)
- [ADR-0006 — Composable unit contract is kernel; orchestration (taskgraph) is an external tool](#adr-0006--composable-unit-contract-is-kernel-orchestration-is-an-external-tool)

---

### ADR-0001 — Wire protocols stay kernel-owned

**Status:** Accepted.

**Context.**
The Models/providers surface (surface #2) is the one extension surface where a
*genuinely new wire/streaming protocol* (the code that turns an LLM HTTP/SSE
response into the agent's text, thinking, and tool-call event stream) currently
forces a recompile. Two pressures collided:

- **The "no walls" rule** says everything the community wants should be reachable
  through a runtime surface. A blocked wire protocol is a literal wall.
- **A real incident:** adding Fireworks serverless models surfaced that quecto
  only ships OpenAI/Anthropic-shaped protocols, and that the router mishandled
  multi-segment model IDs — i.e. the model surface needed rework anyway.
- **A concrete reference design:** pi (`earendil-works/pi`) pushes wire protocols
  *out* of its kernel — an in-process TypeScript extension can register a custom
  `streamSimple` for any new API. That maximises community reach but moves a
  security- and correctness-critical parser into community hands, and relies on an
  in-process extension model quecto does not have (quecto is a compiled binary;
  its only live extension surface is the UDS tool registry).

The wire protocol is special among kernel concerns: it parses **untrusted
provider output** directly into the agent's context and tool-call stream, and its
edge cases (partial tool-call JSON, thinking-block signatures, usage/cost
accounting, context-overflow detection, abort, Unicode boundaries) are exactly
the things that break silently.

**Decision.**
Wire/streaming protocol *implementations* remain **kernel-owned**. The team
builds and maintains exactly **three** robust, fully-tested protocols, selected
per model by an `api` field:

1. `openai-completions` — OpenAI Chat Completions and compatibles.
2. `anthropic-messages` — Anthropic Messages and compatibles.
3. `google-generative-ai` — native Google Gemini *(to be added as part of this
   work; not shipped today)*.

The community extends **models** freely on top of these three — via a runtime
model registry (config/`models.json` + live reload) that can add any compatible
model on the fly, usable next turn with no restart. The kernel ships only a few
**example models per protocol**; the registry overrides/extends them.

**Consequences.**
- *Positive:* the security-/correctness-critical surface stays audited and
  semver-stable; three protocols are cheap for the team to own; coverage is huge
  with **zero recompile** — OpenAI-shaped (Fireworks, Groq, DeepSeek, Together,
  OpenRouter, Ollama, vLLM, LM Studio, gateways), Anthropic-shaped (Claude +
  proxies), and Gemini natively. The binary stays flat (no plugin loader / script
  engine). Adding native Gemini makes "OpenAI/Anthropic/Google compatible"
  literally true.
- *Negative / cost:* one wall remains — a *genuinely novel* protocol (Cohere,
  Bedrock Converse, a bespoke API) needs a kernel release; the core team is the
  bottleneck for new protocol families; an agent cannot self-extend to an
  unsupported protocol.
- *Mitigation (keeps "no walls in practice"):* bridge a novel protocol with a
  UDS/local process that translates it to an **OpenAI-compatible** endpoint, then
  add it as a normal `openai-completions` provider. Documented pattern, not a v1
  feature.
- *Door left open:* if demand for novel protocols proves real, a future kernel
  version may add a UDS `register_stream` surface — decided then, not now.

**Alternatives considered (rejected).**
- **A. Community-authored wire protocols (pi's model).** Let an extension supply a
  custom streamer. *Rejected for v1:* quecto has no in-process extension surface,
  so this means either a streaming-event protocol marshalled over UDS (per-token
  IPC overhead + a large new protocol) or an embedded scripting runtime (bloats
  the binary, breaks "smallest useful"). Worse, it hands a parser of untrusted
  model output to the community — the highest-risk injection/exfiltration and
  silent-correctness surface. Cost is broad and structural; benefit is a
  long-tail capability.
- **B. Ship only the two existing protocols (OpenAI + Anthropic), reach Gemini via
  OpenAI-compatible gateways only.** *Rejected:* leaves "Google compatible" as a
  half-truth (gateway-only, no native Gemini features), and native Gemini is a
  bounded, well-understood addition (pi's `google.ts` as reference). The marginal
  cost of the third protocol is small relative to the capability and clarity gain.
- **C. Generic/pluggable protocol config (describe any wire format declaratively
  in `models.json`).** *Rejected:* real protocols differ in streaming framing,
  tool-call semantics, and thinking handling in ways a declarative schema cannot
  capture without effectively becoming a programming language; it would recreate
  alternative A's risks with more complexity and less safety.
- **D. Keep the status quo (OpenAI/Anthropic only, no decision).** *Rejected:* the
  Fireworks incident showed the surface needs explicit scope; leaving it implicit
  invites ad-hoc additions and an unclear contract for the community.

**Related:** surface #2 above; the "What forces a kernel change" list;
[prd-models-runtime-extensible.md](prd-models-runtime-extensible.md).

---

### ADR-0002 — Reload trigger for startup-loaded surfaces

**Status:** Accepted.

**Context.**
Three surfaces are loaded **once at construction** and so fail the autonomy
contract: skills (scanned when the system prompt is built), models/providers
(`Config::load` at startup), and the workflow *template set* (read from config at
startup). For an agent to extend **itself** — write a skill, add a model, define
a template — and *use it on its next turn with no human restart*, each needs a
**reload path**. The open question is the **trigger**: when does the reload run,
and how do we avoid paying for it every turn? The reference design (pi) refreshes
its model registry on the operations that *consume* it (model-selector open,
`set_model`, login/logout), which is enough for a human at a TUI but gives an
autonomous agent self-extension only as an accidental side-effect of those ops.

**Decision.**
Adopt a **hybrid trigger** behind a **cheap change-detection gate**, implemented
once as a **single shared mechanism** for all three surfaces:

1. **Top-of-turn reload (the guarantee).** At the start of each turn the agent
   loop consults the reload mechanism; any changed source is rebuilt before the
   turn proceeds. This is what makes the autonomy contract a *guarantee*, not a
   side-effect.
2. **On-consume reload (freshness for interactive use).** Operations that read a
   surface (e.g. the model selector) also trigger a reload, so humans see changes
   immediately without waiting for a turn boundary.
3. **Change-detection gate (cheapness).** Each reload is short-circuited by an
   `mtime` check, with a content hash computed only when mtime moved. Steady-state
   cost is one `stat` per source — effectively zero; a rebuild happens only on
   actual change.
4. **Fail-safe.** A malformed source on reload keeps the **last-good** state and
   logs a warning; a live session is never crashed by a bad edit.

This is one `RuntimeReload`-style component (watch sources → rebuild affected
live state) with three consumers (knowledge folder sources → rebuild index/
injected bootstrap; models → rebuild registry/provider set; workflow-set →
rebuild template list), per the doc's "converge on the same mutate-live-state
model" direction.

**Push vs pull — tools are deliberately NOT a consumer.** ADR-0002 specifies a
**pull** trigger: the kernel re-reads a *file-backed source* on a turn boundary,
gated by mtime/hash. This is distinct from how **tools** already reload, which is
**push**: an external process *calls in* via UDS `register_tools` and mutates the
live `ToolRegistry` directly (`register_extension` → insert → `rebuild_definitions`);
there is no file re-read and no `reload_extensions` polling (that command is a
no-op). The two triggers feed the same *pattern* — a long-lived registry the agent
reads fresh each turn — but they are different mechanisms:

- **Reusable across knowledge/models/workflow-set:** the live-registry pattern
  (agent holds the registry; reads it each turn; new state visible next turn).
  Models should hold a live `ModelRegistry` read at top of turn, mirroring how the
  agent already reads `ToolRegistry::definitions()`.
- **NOT reusable from tools:** the *trigger*. Tools have no file-watch/re-read
  path; ADR-0002's pull mechanism (re-read + mtime/hash gate + fail-safe
  last-good + `ArcSwap`/`RwLock` atomic swap) is **net-new work** and is the real
  substance of Phase 2. Do not assume the tool path provides a head start on the
  trigger.
- **Tools remain push.** They keep `register_tools`. The only place tools would
  ever touch the pull mechanism is the *optional, future* "tool-manifest
  discovery" (drop a file → start/register a process) — explicitly not required
  for the autonomy bar.

**Consequences.**
- *Positive:* the autonomy contract holds for every surface; self-made changes
  land next turn with no restart; interactive edits are visible immediately;
  near-zero per-turn cost; one mechanism to build, test, and reason about.
- *Negative / cost:* a small amount of per-turn bookkeeping and the discipline of
  routing every startup-loaded surface through the shared path; a window exists
  where an edit mid-turn is only picked up at the next turn boundary (acceptable —
  that *is* the "next turn" contract).
- *Security note:* re-reading config each turn means a process that can rewrite
  config can change endpoints/keys between turns; this is the same trust level as
  writing config at all, and is called out for the provider surface specifically.
  A higher trust bar for *endpoint/key* changes vs. *model/knowledge* additions is
  deferred to a future ADR (tracked as the models PRD's reload-trust question).

**Alternatives considered (rejected).**
- **A. On-consume only (pi's model).** Reload solely when an op reads the surface.
  *Rejected:* gives autonomy only as a side-effect of whichever op happens to run;
  not a guarantee, and brittle for unattended agents.
- **B. Explicit `reload` signal only.** Require a human/supervisor (or an explicit
  tool call) to trigger reload. *Rejected:* puts a human back in the loop, which
  is exactly the autonomy failure the contract forbids. (We still *expose* an
  explicit reload as a convenience, but it is not the only path.)
- **C. Filesystem watcher (inotify/FSEvents).** Push-based live reload.
  *Rejected:* added dependency, portability/edge-case complexity (network FS,
  containers, missed events) for no benefit over a `stat` at a natural per-turn
  checkpoint.
- **D. Unconditional rebuild every turn (no gate).** *Rejected:* needless per-turn
  latency (re-parsing config, re-scanning skills) when nothing changed.

**Related:** "The autonomy contract" section above; surfaces #1–#3;
[prd-models-runtime-extensible.md](prd-models-runtime-extensible.md).

---

### ADR-0003 — UDS `register_provider` for dynamic model/provider registration

**Status:** Proposed (deferred; not in v1).

**Context.**
pi exposes a programmatic `registerProvider()` so an (in-process) extension can
register providers/models at runtime — including **dynamic discovery**, e.g. an
async factory that queries a gateway's `/models` endpoint and registers whatever
it finds. quecto has no in-process extension surface; its live extension surface
is the **UDS tool registry** (`register_tools`, surface #4). The question is
whether to add a parallel **UDS `register_provider`** verb so external processes
can inject providers/models into the live `ModelRegistry`, mirroring how
`register_tools` injects tools.

**Decision.**
**Defer.** Ship the **declarative `models.json` + the ADR-0002 reload path** as
the v1 mechanism for adding models/providers. Do **not** add a UDS
`register_provider` verb in v1. Record the design as Proposed and gate its
promotion on a concrete dynamic-discovery consumer.

**Consequences.**
- *Positive:* keeps the kernel/UDS protocol small (a new verb is a
  semver-significant protocol change — exactly what "smallest useful kernel"
  asks us not to add speculatively); avoids a second registration path to
  maintain; v1 still meets the autonomy bar because ADR-0002 lets an agent write
  `models.json` and use the model next turn.
- *Interim bridge (no kernel change needed):* a sidecar that performs dynamic
  discovery can simply **regenerate `models.json`** (or a fragment of it); the
  ADR-0002 reload picks it up next turn. This covers the dynamic-discovery use
  case today without a new protocol verb.
- *Negative / cost:* dynamic registration is slightly less ergonomic than a native
  verb (a process must write a file rather than call an API), and there's no live
  push — changes land on the next reload, not instantly.
- *Promotion path:* if a real consumer needs live, push-based, file-less
  registration, add `register_provider` then — it would write into the same live
  `ModelRegistry` that `models.json` populates, so no rework of the v1 design.

**Alternatives considered (rejected / deferred).**
- **A. Build `register_provider` in v1.** *Rejected:* speculative protocol surface
  with no current consumer, when `models.json` + reload already satisfies
  autonomy. Violates boundary discipline ("don't add a UDS verb because the
  reference design has the analogue").
- **B. Never add it.** *Rejected:* dynamic discovery (gateways, fleets of local
  models) is a plausible real need; closing the door would be premature. Hence
  **Proposed**, not rejected.
- **C. Sidecar rewrites `models.json` (the interim bridge).** *Not rejected —
  adopted as the deferral's escape hatch.* It needs no kernel change and is why
  deferring the native verb is safe.

**Related:** ADR-0002 (the reload path that makes this deferrable); surface #4
(`register_tools`, the model this would mirror);
[prd-models-runtime-extensible.md](prd-models-runtime-extensible.md).

---

### ADR-0004 — Dissolve the Skills surface

**Status:** Accepted.

**Context.**
The Skills surface (`workspace/skills/<name>/SKILL.md`, installed by
`quecto skills`) was originally the community's file-based capability surface.
On review, it bundles three different concerns that now have better homes:

- **Procedure** — ordered steps, gates, acceptance criteria. Workflows are
  strictly better: they are stateful, typed, observable, and enforceable.
- **Knowledge** — reference text, APIs, conventions. A generalized `docs` /
  knowledge-retrieval tool is strictly better: progressive disclosure, search,
  and no prompt bloat.
- **Persona / handoff context** — task-specific instructions for a child agent.
  A parent can already spawn a sub-agent with a `system` prompt and a binding
  `workflow_spec`; reusable definitions belong as community data over that spawn
  mechanism, not as prompt-injected skills.

Today's skill loader injects full skill bodies into the system prompt at startup.
That creates two failures: (1) it consumes prompt budget proportional to library
size, and (2) it is invisible to a self-extending agent until restart. The
knowledge-retrieval proposal already states that "skills with progressive
disclosure" are just generalized `docs`.

**Decision.**
Dissolve **Skills** as a distinct kernel extension surface.

- Procedural skills become **workflow templates**.
- Knowledge skills become entries/sources in the **knowledge-retrieval surface**
  (ADR-0005).
- Task/persona bundles become **community sub-agent definitions**: data that a
  parent/tool reads and passes to existing `spawn` (`system` + `workflow_spec` +
  optional knowledge scope). No new kernel spawn mechanism is required.
- `quecto skills install/list/remove` may remain temporarily as a compatibility
  curator over the knowledge folder, but it no longer defines a separate surface.

**Consequences.**
- *Positive:* one fewer conceptual surface; no prompt bloat; no bespoke skill
  reload path; the capability decomposes into mechanisms quecto already owns
  (workflows, docs/knowledge retrieval, spawn).
- *Negative / migration cost:* existing `workspace/skills/` content and commands
  need a migration story. Existing content can be indexed as a folder-backed
  knowledge source; procedural skills should be converted into workflow
  templates; mixed skills may split into both.
- *Autonomy:* the auto-load gap is closed by the knowledge-retrieval call path and
  ADR-0002's reload mechanism, not by re-injecting full skill bodies into the
  prompt.

**Alternatives considered (rejected).**
- **A. Keep Skills and add progressive disclosure.** *Rejected:* it recreates the
  generalized docs/knowledge tool under another name and leaves a redundant
  public contract.
- **B. Keep Skills for small two-step procedures.** *Rejected:* if ordering/gates
  matter, it is a workflow; if it is just advice, it is knowledge. Size is not the
  right boundary.
- **C. Keep startup prompt injection and add auto-reload.** *Rejected:* solves
  restart but not prompt bloat; it scales with installed library size, not use.
- **D. Delete skills with no compatibility path.** *Rejected:* existing content
  should continue as folder-backed knowledge where possible.

**Related:** [knowledge-retrieval-surface.md](knowledge-retrieval-surface.md),
ADR-0005, workflow surface #3, tool surface #4.

---

### ADR-0005 — Knowledge as a retrieval surface

**Status:** Accepted.

**Context.**
The kernel already has a `docs` tool: a curated library whose names are
discoverable and whose bodies are fetched on demand. Context7 demonstrates the
same pattern over an external corpus via MCP. The proposed knowledge-retrieval
surface generalizes this into one mechanism for embedded docs, community files,
knowledge graphs/databases, and remote sources. This is the correct destination
for the knowledge half of Skills (ADR-0004) and for any future community
knowledge graph.

**Decision.**
Make **knowledge retrieval** the canonical knowledge surface. The kernel owns the
retrieval contract and a small always-on index; community content and rich
backing stores live outside the kernel.

- Kernel-owned sources: embedded kernel docs and a simple folder-backed markdown
  source (e.g. `workspace/knowledge/`, legacy `workspace/skills/`).
- External-tool sources: graph/database/remote retrieval (including a knowledge
  graph) are **UDS/MCP tools**, not in-kernel graph engines.
- Retrieval is progressive-disclosure by construction: list/search exposes names,
  descriptions, and snippets; full bodies are fetched only when relevant.
- The always-on bootstrap in the system prompt stays deliberately tiny: enough to
  tell the agent that the knowledge tool exists and how to search/list/fetch, not
  the knowledge bodies themselves.

**Consequences.**
- *Positive:* unifies `docs`, skills-as-knowledge, Context7-style remote docs, and
  knowledge graphs behind one contract; keeps the kernel out of storage/indexing
  complexity; avoids prompt bloat; runtime sources can be available next turn.
- *Negative / risk:* retrieval quality becomes load-bearing. If search/indexing
  misses, the agent may fail to pull relevant knowledge. The bootstrap index and
  source descriptions must therefore be high quality and small.
- *Boundary:* a knowledge graph is a **tool-backed source**. The kernel may know
  how to call `list/search/fetch`; it does not own graph schema, embeddings, RAG,
  ranking, or storage.

**Alternatives considered (rejected).**
- **A. Keep a separate Skills loader.** *Rejected by ADR-0004:* redundant and
  prompt-heavy.
- **B. Put a graph database / embedding index in the kernel.** *Rejected:* storage
  and ranking are fast-moving, dependency-heavy, and community-specific; they fit
  the UDS/MCP tool boundary.
- **C. Rely only on external MCP/Context7.** *Rejected:* loses the zero-config
  embedded docs and folder-backed community files that make quecto self-contained
  and binary-friendly.
- **D. Inject all knowledge summaries into the system prompt.** *Rejected:* prompt
  budget scales with corpus size and recreates the skills problem.

**Related:** [knowledge-retrieval-surface.md](knowledge-retrieval-surface.md),
ADR-0004, surface #4 (tools/extensions), ADR-0002 (reload).

---

### ADR-0006 — Composable unit contract is kernel; orchestration is an external tool

**Status:** Accepted.

**Context.**
`docs/composable-workflow-units-prd.md` defines quecto's recursive unit contract:
sub-agents are the same binary over the same protocol as parents; a parent can
spawn a child with a by-value, binding `workflow_spec`; children return typed
results; identity-tagged events let any consumer reconstruct the tree. Stages A
and B have shipped. Separately, a **taskgraph** (DAG / fan-out / fan-in /
dependency orchestration) is useful, but putting a graph engine into the kernel
would enlarge the workflow engine and introduce a privileged orchestrator role.

**Decision.**
Keep the **composable unit contract** in the kernel; implement **taskgraph /
DAG orchestration** as an external tool.

- Kernel-owned: `spawn`, `workflow_spec`, typed `agent_cmd await` result,
  identity-tagged event bus, per-unit workflow engine, and future per-unit
  journaling/resume/verification/bounds from the composable-units PRD.
- Tool-owned: graph construction, dependency scheduling, fan-out/fan-in policy,
  and which sub-agent definitions to use for each node.
- A taskgraph tool drives ordinary quecto units by calling existing surfaces
  (`spawn`, `agent_cmd`, workflow specs, tools). It is not a privileged agent
  type and does not bypass kernel bounds.
- Per-node procedure remains a **workflow**. The taskgraph decides *which units
  exist and their dependencies*; each unit's internal step sequence is handled by
  the kernel workflow engine.

**Consequences.**
- *Positive:* complex orchestration is possible without bloating the kernel; the
  graph layer can evolve independently as a UDS/MCP tool; the kernel retains the
  safety invariants (typed results, event identity, bounds, journaling).
- *Negative / cost:* graph-level semantics are not kernel-standardized at first;
  different taskgraph tools may make different scheduling choices. That is
  acceptable because the kernel-standard contract is the unit boundary, not the
  graph language.
- *Enforcement line:* the taskgraph tool may decide the graph, but the kernel
  enforces per-unit constraints (budget/depth/concurrency once Stage E lands),
  journaling/resume (Stage C), and verification gates (Stage D). A tool cannot
  opt out by claiming to be an orchestrator.

**Alternatives considered (rejected).**
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

**Related:** [composable-workflow-units-prd.md](composable-workflow-units-prd.md),
surface #4 (tools/extensions), workflow surface #3, ADR-0002.

---

## Removed: dormant surfaces (kept the kernel honest)

To uphold "smallest useful," these half-built, unwired surfaces were deleted —
none were read by production code:

- **`MemoryStore` / `load_identity`** (`workspace/MEMORY.md`, `IDENTITY.md`) — a
  long-term-memory module wired to nothing. Memory, if wanted, belongs as a
  community tool/extension or a *deliberate* future kernel feature — not as
  dead code pretending to be a capability.
- **Onboarding template files** (`AGENTS.md`, `IDENTITY.md`, `SOUL.md`,
  `TOOLS.md`, `USER.md`) — written by `quecto onboard` but consumed by nothing.
  Onboarding now creates only what is actually used: the config file and the
  workspace directory (skills live under `workspace/skills/`).
