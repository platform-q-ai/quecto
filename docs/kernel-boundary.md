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

### 1. Skills — ✅ binary-only
- `workspace/skills/<name>/SKILL.md` with YAML frontmatter (`name`,
  `description`, …); validated (name format, dir-name match, size cap).
- Managed via `quecto skills install <owner>/<repo>/<name>` (GitHub), `list`,
  `remove`. Folder-discovered at startup and injected into the system prompt.
- **Kernel owns:** the loader + prompt injection. **Community owns:** the skills.
- **Roadmap:** progressive disclosure — today the full skill *content* is
  concatenated into the prompt; the agentskills.io model (name+description in the
  prompt, agent reads `SKILL.md` on demand) is the scaling upgrade.

### 2. Models / providers — 🟡 binary-only for OpenAI/Anthropic-compatible
- `providers.openai_compatible.endpoints` (`prefix`, `api_base`, `api_key`,
  `allow_remote_http`). Any model string; **no allowlist**. Covers Ollama, vLLM,
  LM Studio, Fireworks, Groq, DeepSeek, and most hosted endpoints.
- **Recompile triggers:** a genuinely new *wire protocol* (e.g. native Gemini /
  Cohere, not OpenAI/Anthropic-shaped), or OAuth on a custom provider.
- **Graceful degradation:** unknown-model metadata (pricing, thinking, context
  window) is missing but never blocks the request.
- **Roadmap:** a config-driven model-metadata registry so community models get
  correct context-window/pricing/capability info.

### 3. Workflows — 🟡 binary-only (config templates)
- `workflow.templates` in config: arbitrary steps/keys/phases/guidance/guards;
  the engine is generic (no hardcoded step keys; unknown phases pass through).
  Covers the large majority of development workflows.
- **Intentional design:** a community config's `templates` **replace** the
  kernel defaults. The kernel keeps the `feature` template as the team's
  reference + internal-dev workflow — it ships as an *example*, and is cleanly
  out of the way the moment a community config provides its own.
- **Recompile triggers:** new workflow *tool actions* (beyond the standard
  check/skip/select/guards set) or new engine progression semantics (approval
  gates, conditional branches, multi-actor execution).

### 4. Tools / extensions — 🟡 binary-only via UDS + MCP
- **UDS `register_tools`** (any language): an external process registers tools
  live; they're callable on the next turn. Documented in `docs/extensions.md`.
- **`quecto-mcp` sidecar:** point it at any MCP server → its tools are bridged
  in, zero code.
- **Core tools** (`read`/`write`/`edit`/`bash`/`grep`/`find`/`ls`/`docs`) are
  always present; disable per-run with `--disable-tool`; they cannot be shadowed.
- **No folder/manifest discovery** (script extensions were removed in #353;
  `reload_extensions` is a no-op).
- **Recompile triggers:** a new *core* tool, tool-routing changes, or `Tool`
  trait changes.
- **Roadmap:** discovery ergonomics (folder-drop / manifest) and an extension
  SDK/template to lower authoring friction.

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
