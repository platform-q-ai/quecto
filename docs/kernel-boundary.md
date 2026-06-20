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

---

## The four extension surfaces (definitive status)

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
| 1. Skills | ✅ full | ❌ **missing** — scanned once when the system prompt is built |
| 2. Models / providers | 🟡 partial (OpenAI/Anthropic-shaped only) | ❌ **missing** — config read once at startup |
| 3. Workflows | 🟡 partial (config templates) | 🟡 **partial** — selecting/advancing is live; *adding* a template needs restart |
| 4. Tools / extensions | 🟡 partial (external process only) | ✅ **full** — UDS `register_tools` is live next turn |

Only surface 4 (tools, via UDS) currently meets the autonomy bar. The other
three are startup-loaded and need a reload path before an agent can extend
*itself* mid-run.

### 1. Skills — capability ✅ · auto-load ❌
- **Implemented today (full capability):** `workspace/skills/<name>/SKILL.md`
  with validated YAML frontmatter (`name`, `description`, …; name format,
  dir-name match, size cap). Managed via `quecto skills install
  <owner>/<repo>/<name>` (GitHub), `list`, `remove`. Folder-discovered and
  injected into the system prompt. Kernel owns the loader + injection; community
  owns the skills.
- **Partial:** progressive disclosure. Today the *full* skill content is
  concatenated into the prompt; the scaling model (name+description in-prompt,
  agent reads `SKILL.md` on demand) is not yet implemented.
- **Missing — auto-load ❌:** `load_skill_prompt` runs **once**, when the
  agent/REPL builds its system prompt (`src/interface/cli/agent.rs`,
  `src/interface/repl/mod.rs`). A skill installed *mid-session* — including one
  the agent installs for itself — is **invisible until the process restarts**.
  To close: re-scan `workspace/skills/` and rebuild the system prompt at the
  start of each turn (or on a reload signal), so a freshly-installed skill is in
  the prompt next turn with no restart.

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
- **Skills, models, workflows-set: ❌/🟡** content is read **once at construction**
  (system prompt for skills; `Config::load` for providers and the workflow
  template set), so a self-made change is invisible until the binary is restarted.

To make autonomy hold, every startup-loaded surface needs a **reload path** that
runs at the top of each turn (or on an explicit reload signal) and is cheap when
nothing changed:

- re-scan `workspace/skills/` and rebuild the injected system prompt;
- re-read the `providers` and `workflow.templates` sections of config and rebuild
  the provider set / template list.

Until those exist, the only fully-autonomous escape hatch is to express a new
capability as a **UDS tool** (the one live surface) — or to accept that a human
must restart quecto for skill/provider/workflow changes to land.

---

## In the kernel (team-owned, semver-stable)

- The agent loop and context management.
- Core tools: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`, `docs`.
- The UDS protocol and sub-agent spawn/orchestration (quecto's recursion).
- The workflow engine.
- The provider protocol (OpenAI/Anthropic-compatible) and the `LlmProvider` trait.
- The registries and discovery for skills / workflows / models / tools.
- The embedded `docs` capability tool.

## Out of the kernel (community content, no recompile)

- Skills (`workspace/skills/`).
- Workflow templates (config).
- Provider endpoints and models (config).
- Extension tools (UDS clients / MCP servers).

## What forces a kernel change (deliberately the team's domain)

- A new LLM **wire protocol**.
- A new **core tool**, tool-routing, or `Tool`-trait change.
- New **workflow tool actions** or **engine semantics**.
- A **UDS protocol** change.

These are the "kernel" in "smallest useful kernel" — they're correctly ours to
own and keep stable.

---

## Discovery & footprint

- **Config:** a single `~/.quecto/config.json` (or `--config` /
  `QUECTO_BASE_DIR`). *Roadmap:* per-project + folder discovery for parity with
  skills, so every surface follows "drop a file in a known dir → it's picked up."
- **Skills:** `workspace/skills/` folder scan.
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
