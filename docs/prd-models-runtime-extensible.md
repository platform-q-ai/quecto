# PRD: Runtime-Extensible Models & Providers (auto-load on next turn)

**Status:** Accepted for implementation (Phase 1 first; Gemini protocol fast-follow)
**Owner:** core team (kernel)
**Surface:** #2 Models / providers (per [kernel-boundary.md](kernel-boundary.md))
**Related:** #1 Skills auto-load, #3 Workflow-set auto-load (share the reload path)
**Decisions:** governed by [ADR-0001](kernel-boundary.md#adr-0001--wire-protocols-stay-kernel-owned)
(wire protocols kernel-owned, 3 impls),
[ADR-0002](kernel-boundary.md#adr-0002--reload-trigger-for-startup-loaded-surfaces)
(hybrid reload trigger — Accepted), and
[ADR-0003](kernel-boundary.md#adr-0003--uds-register_provider-for-dynamic-modelprovider-registration)
(UDS `register_provider` deferred). This PRD must stay consistent with those ADRs.

---

## 1. Why (problem statement)

Per the kernel boundary, the **Models / providers** surface is rated
**🟡 capability · ❌ auto-load**. Two concrete failures follow from that, and we
hit both this week trying to add Fireworks serverless models (GLM 5.2, Kimi K2.7
Code):

1. **No auto-load (the autonomy breaker).** Providers/models come from
   `Config::load`, read **once at startup**. When the agent (or a human) adds a
   model to `~/.quecto/config.json` mid-session, it is **invisible until the
   process is restarted**. This silently puts a human back in the loop and
   breaks quecto's recursion: an agent cannot add a model and *use it next turn*.

2. **Capability gaps that block real serverless models.** Even with a restart,
   adding Fireworks revealed three sharp edges:
   - **Routing encodes provider+model in one slash-delimited string.**
     `parse_qualified_model` in `src/infrastructure/providers/router.rs` treats a
     model as `prefix/model_id` and **rejects any `model_id` containing `/`**.
     Fireworks *requires* `accounts/fireworks/models/glm-5p2`, so
     `fireworks/accounts/fireworks/models/glm-5p2` fails to parse, falls through
     to "bare model → first provider" (the user's codex/OpenAI slot), and errors
     with *"codex provider expects a bare model id."* There is currently **no
     config-only way** to use Fireworks serverless models. The fix is not to
     tweak the split but to **stop encoding provider+model in one string** —
     provider is a key, model `id` is opaque (see FR2, following pi's data model).
   - **No model-metadata registry.** Pricing, context window, and capability
     flags (thinking, tools, vision) for community models don't exist; the kernel
     degrades gracefully (request still works) but cost/҂context UX is blind.
   - **The selector model list is a hardcoded constant.** `KNOWN_MODELS` in
     `quecto-tui/src/interface/components/model_selector.rs` is compiled in, so
     community models never appear in `/model` without a recompile — and when we
     *did* compile them in, the short names (`glm-5.2`) didn't match Fireworks'
     real IDs (`accounts/fireworks/models/glm-5p2`), producing 404s.

The autonomy contract says every startup-loaded surface needs a **reload path
that runs at the top of each turn** (or on an explicit reload signal) and is
cheap when nothing changed. This PRD defines that for models/providers and closes
the capability gaps that make community models actually usable.

---

## 1a. Core design decision: wire protocols stay kernel-owned (3 robust impls)

**Decision:** Wire/streaming protocol *implementations* remain **kernel-owned**.
The team builds and maintains exactly **three robust, fully-tested wire
protocols**, and the community extends *models* freely on top of them — never the
protocols themselves.

The three kernel protocols (the `api` values):

1. **`openai-completions`** — OpenAI Chat Completions and compatibles.
2. **`anthropic-messages`** — Anthropic Messages and compatibles.
3. **`google-generative-ai`** — native Google Gemini. *(New: not shipped today;
   added as a **fast-follow** immediately after Phases 1–3 so
   "OpenAI/Anthropic/Google compatible" is literally true, not
   "Gemini-via-OpenAI-gateway only." pi's `google.ts` is the reference
   implementation.)*

### Why this is the right boundary
- **Cheap to own.** Three protocols are trivial for the core team to maintain and
  keep semver-stable; they are the correctness- and security-critical surface
  (parsing untrusted provider output into the agent's context and tool-call
  stream), so they belong with audited kernel code.
- **Massive community scope for free.** These three cover the overwhelming
  majority of real providers with **zero recompile**:
  - *OpenAI-shaped:* Fireworks, Groq, DeepSeek, Together, OpenRouter, Mistral-
    compatible endpoints, Ollama, vLLM, LM Studio, corporate gateways, OpenAI.
  - *Anthropic-shaped:* Claude + Anthropic-compatible proxies.
  - *Gemini:* natively (and Gemini via OpenAI-compatible gateways already works
    through `openai-completions`).
- **quecto adds models on the fly.** The community (and the agent itself) adds any
  model that speaks one of the three protocols via `models.json` + the live
  `ModelRegistry` + reload — usable on the next turn, no restart. We ship only a
  **handful of example models per protocol** out of the box (a curated built-in
  pack); the registry overrides/extends it.

### What we deliberately do NOT do (smaller than pi)
- **No community-authored wire protocols.** pi lets an in-process extension
  register a custom `streamSimple` for a brand-new API. quecto has no in-process
  extension surface (compiled binary), and a streaming-over-UDS protocol or
  embedded runtime is a large, risky, security-sensitive addition for a long-tail
  capability. We skip it.

### The one remaining wall, and its escape hatch
- A *genuinely novel* wire protocol (Cohere, Bedrock Converse, a bespoke API)
  still requires a kernel change. This is a deliberately rare tail.
- **Escape hatch (documented, not a v1 feature):** run a small UDS/local process
  that translates the bespoke API to an **OpenAI-compatible** endpoint, then add
  it as a normal `openai-completions` provider. "No wall in practice."
- If real demand for novel protocols emerges, a future kernel version may add a
  UDS `register_stream` surface — but only once the need is proven.

**Net:** we own 3 protocols; the community owns unlimited *models*. This upholds
"smallest useful kernel, no walls" while making config-only model expansion the
norm.

---

## 2. Goals / non-goals

### Goals
- **G1 — Auto-load:** A provider/model added to config (or a model registry file)
  is usable on the **next turn**, with **no restart**.
- **G2 — Self-extension:** The agent can add a model to its own available set via
  a runtime surface (config edit or a dedicated tool) and select it next turn.
- **G3 — Real serverless models work config-only:** Fireworks/Together/OpenRouter
  multi-segment IDs route correctly with no recompile (fixes the router bug).
- **G4 — Discoverable model list:** `/model` (TUI) and any model-listing surface
  reflect configured + registry models at runtime, not a compiled constant.
- **G5 — Optional metadata:** A config/file-driven model-metadata registry
  supplies pricing/context-window/capabilities; absence never blocks a request.
- **G6 — Cheap when unchanged:** The reload is a no-op (mtime/hash check) when
  nothing changed; no measurable per-turn latency.

### Non-goals
- **NG1 — Community-authored wire protocols.** Per
  [ADR-0001](kernel-boundary.md#adr-0001--wire-protocols-stay-kernel-owned), wire
  protocols stay kernel-owned. This PRD does **not** add a community/extension
  path for new wire protocols. The third kernel protocol
  (`google-generative-ai`, native Gemini) is a **fast-follow** (its own
  feature-workflow change, immediately after this PRD's Phases 1–3 land), not part
  of the core models work; further protocols (Cohere, Bedrock Converse, bespoke)
  remain kernel changes, bridged meanwhile by an OpenAI-compatible shim.
- **NG2 — OAuth for custom providers.** Custom-provider OAuth stays kernel-owned.
- **NG3 — Secret management overhaul.** Keys continue to live in config /
  credential store / env; no new vault. (We *do* adopt `$ENV`/`!command` value
  resolution for keys — see FR5 — but introduce no new secret store.)
- **NG4 — Per-project/folder config discovery.** Tracked separately on the
  roadmap; this PRD assumes the existing single-config path (+ optional registry
  file) and is compatible with folder discovery later.
- **NG5 — UDS `register_provider`.** Deferred per
  [ADR-0003](kernel-boundary.md#adr-0003--uds-register_provider-for-dynamic-modelprovider-registration);
  not in this PRD's scope. Dynamic discovery is bridged by a sidecar that
  regenerates `models.json` (picked up by the FR1 reload).

---

## 3. Users & stories

- **As the agent (recursion):** "I discover a task needs a cheaper coding model.
  I add a Fireworks endpoint + model to my config (or call a `model` tool), and on
  my next turn I `set_model` to it and continue — no human, no restart."
- **As a community user:** "I drop my provider + model IDs into config, start a
  turn, and the model is in `/model` and usable immediately."
- **As a cost-conscious user:** "When I add metadata for my model, the TUI shows
  correct context window and per-token cost; when I don't, it still just works."

---

## 4. Functional requirements

### FR1 — Provider/model reload path (auto-load) — per ADR-0002
- Implements the **hybrid trigger** from
  [ADR-0002](kernel-boundary.md#adr-0002--reload-trigger-for-startup-loaded-surfaces):
  - **Top-of-turn reload (guarantee):** at the start of each turn (agent loop and
    REPL), re-read the `providers` and (new) `models` sources and rebuild the
    provider set / model registry **iff** a source changed (mtime, then content
    hash only when mtime moved).
  - **On-consume reload (freshness):** operations that read the registry (e.g. the
    `/model` selector, `set_model`) also trigger the gated reload so interactive
    edits land immediately.
- Reload is **fail-safe**: a malformed source on reload logs a warning and keeps
  the **last-good** provider/registry state (never crashes the session).
- Expose an explicit `reload` trigger too (convenience, not the only path; the
  no-op `reload_extensions` is the precedent).
- **Shared mechanism** with Skills (#1) and Workflow-set (#3): one "sources
  changed → rebuild affected live state" component, three consumers — not three
  bespoke reloads. This shared component is the concrete realization of ADR-0002.

### FR2 — Opaque model IDs: provider is a key, not a string prefix (capability fix)
- **Adopt pi's data model.** A model is a record `{ provider, id, … }`. The
  provider is an explicit key (config/registry); the model `id` is **opaque** and
  passed verbatim to the API. Resolution is a direct lookup
  (`registry.find(provider, id)`), **not** string-splitting on `/`.
- This **eliminates the router-bug class at the root** rather than patching it:
  `accounts/fireworks/models/glm-5p2` is just an opaque `id` under provider
  `fireworks`; nested `/` is irrelevant because the kernel never re-parses it.
- **Migration / back-compat:** the existing `provider/model` string form (e.g.
  `set_model fireworks/...`, `--model`) is still accepted at the *UI/CLI boundary*
  and resolved by **splitting on the first `/`** into `(provider, id)`, then
  looked up in the registry. There is no further parsing of `id`; bare strings
  with no known provider prefix resolve as a model `id` on the default/first
  provider. Internally, routing carries `(provider, id)`, not a recombined string.
- If a referenced `provider` is not configured, error clearly
  ("no configured provider 'fireworks'"), never silently fall back to the first
  provider.
- **Resolved (was Q1):** there is no "bare multi-segment" ambiguity anymore —
  multi-segment text is always an opaque `id`, never re-split. The only split is
  the single first-`/` at the UI boundary to recover the provider key.

### FR3 — Runtime model registry (discoverable list + metadata)
- **Decision (Q2 closed):** the registry is a **single source** —
  `~/.quecto/models.json` (folder-ready) — **not** a `models` section inside
  `config.json`. One file, pi-shaped, keeps parsing/merge/reload simple.
- **Decision (Q5 closed):** the kernel ships a **small curated built-in pack** of
  popular hosted models (correct `api`/`contextWindow`/`cost` for zero-config
  UX), which `models.json` **overrides/extends** by `provider`+`id`. `KNOWN_MODELS`
  becomes that built-in pack, not a hardcoded selector constant.
- `~/.quecto/models.json` **follows pi's providers-own-their-models shape**:
  providers are keyed objects carrying their own `models[]`, an explicit `api`
  (wire protocol), `baseUrl`, and key/headers. Model `id` is opaque; metadata is
  optional with graceful defaults.
  ```json
  {
    "providers": {
      "fireworks": {
        "baseUrl": "https://api.fireworks.ai/inference/v1",
        "apiKey": "$FIREWORKS_API_KEY",
        "api": "openai-completions",
        "authHeader": true,
        "models": [
          {
            "id": "accounts/fireworks/models/glm-5p2",
            "name": "GLM 5.2 (Fireworks)",
            "reasoning": true,
            "input": ["text"],
            "contextWindow": 200000,
            "maxTokens": 32000,
            "cost": { "input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0 }
          }
        ]
      }
    }
  }
  ```
- The `api` field selects one of the **three kernel-owned wire protocols**
  (ADR-0001): `openai-completions`, `anthropic-messages`, `google-generative-ai`.
  An unknown `api` is a validation error (we do not run community protocols).
- All metadata fields **optional** with defaults (e.g. `contextWindow` 128000,
  `maxTokens` 16384, `cost` zeros, `input` `["text"]`); missing metadata degrades
  gracefully exactly as today (request works; cost/context UX absent).
- **Merge semantics (pi-style):** custom models upsert into the built-in pack by
  `provider`+`id` (custom wins); a provider entry with only `baseUrl`/`headers`
  overrides built-ins (proxy redirect) without redefining models; optional
  per-model overrides may patch individual built-in models later (not required for
  v1).
- The TUI `/model` selector and any model-listing API read this registry +
  built-in pack at runtime. `KNOWN_MODELS` becomes a **fallback/built-in pack**,
  not the source of truth.

### FR4 — Self-extension surface (agent-authored models)
- **v1 (sufficient given FR1 + ADR-0002):** the agent edits `~/.quecto/models.json`
  via existing `write`/`edit` tools; the top-of-turn reload makes the model usable
  next turn. No new tool or protocol verb required for the autonomy bar.
- **Optional ergonomic add (discuss, may defer):** a small first-class `model`
  tool action (`add`/`list`/`remove`/`set_default`) that validates and writes the
  registry, mirroring `quecto skills install` — nicer than raw JSON surgery and
  gives validation/error messages.
- **Out of scope here:** a UDS `register_provider` verb for live/dynamic
  registration is **deferred per ADR-0003**; the interim bridge is a sidecar that
  regenerates `models.json` (picked up by the FR1 reload).

### FR5 — Validation, value resolution & safety
- Endpoint validation unchanged (HTTPS required unless `allow_remote_http`;
  reserved provider keys rejected; max providers/endpoints).
- **Value resolution (adopt pi's):** `apiKey` and header values support
  `$ENV_VAR` / `${ENV_VAR}` interpolation, `!command` execution (resolved at
  request time), and literals; `$$`/`$!` escape. This is how a key in the
  environment (e.g. `$FIREWORKS_API_KEY`) is referenced from `models.json` without
  embedding the secret. Auth *status* checks must not execute commands.
- Registry entries whose `provider` has no resolved key/endpoint are **listed but
  flagged** "no credentials/endpoint configured" rather than silently failing at
  request time.
- **Secrets never stored in `models.json` as literals when avoidable:** prefer
  `$ENV`/`!command`; keys may still live in `providers.*`/credential store.
  `models.json` is not a new secret store (NG3).

---

## 5. Acceptance criteria (BDD-style)

- **AC1 (auto-load):** Given a running session with no Fireworks model, when a
  Fireworks endpoint + model are added to config, then on the next turn
  `set_model fireworks/accounts/fireworks/models/glm-5p2` succeeds **without
  restart**.
- **AC2 (opaque IDs):** Given provider `fireworks` configured and a model with
  `id: accounts/fireworks/models/glm-5p2`, selecting it resolves via
  `find("fireworks", "accounts/fireworks/models/glm-5p2")` and sends the `id`
  verbatim — no `/`-based re-parsing. The UI form
  `fireworks/accounts/fireworks/models/glm-5p2` splits once on the first `/` into
  `(fireworks, accounts/fireworks/models/glm-5p2)`.
- **AC3 (no silent fallback):** Given a model referencing a `provider` that is not
  configured, the request errors with "no configured provider '<name>'", never
  routing to the first provider.
- **AC4 (selector):** Given a model registry with two Fireworks models, `/model`
  lists both at runtime with their labels; with no registry, the built-in
  fallback list is shown.
- **AC5 (metadata optional):** Given a registry entry without pricing, the model
  is usable and the cost line is absent/zeroed; given pricing, cost is computed.
- **AC6 (cheap no-op):** Given an unchanged config/registry, the per-turn reload
  performs no rebuild (verified via mtime/hash short-circuit).
- **AC7 (fail-safe):** Given a malformed config on reload, the session keeps the
  last-good providers and logs a warning.
- **AC8 (Gemini native — fast-follow):** *Acceptance for the Gemini fast-follow,
  not the core phases.* Given a provider with `api: google-generative-ai` and a
  Gemini model `id`, a chat request streams correctly through the kernel's native
  Gemini wire implementation (no OpenAI-compat gateway required).
- **AC9 (unknown api rejected):** Given a provider with an `api` value outside the
  kernel-shipped protocols (`openai-completions`, `anthropic-messages`; plus
  `google-generative-ai` once the fast-follow lands), `models.json` fails
  validation with a clear error and the session keeps last-good state (community
  wire protocols are not run).
- **AC10 (value resolution):** Given `apiKey: "$FIREWORKS_API_KEY"`, the key is
  resolved from the environment at request time; auth-status checks do not execute
  `!command` values.

---

## 6. Design sketch (kernel-boundary-respecting)

- **Model registry:** a runtime `ModelRegistry` (built-in pack + `models.json`,
  merged by `provider`+`id`) holding `{ provider, id, api, baseUrl, metadata }`
  records. Resolution is `find(provider, id)` — opaque `id`, no slash re-parsing
  (FR2). The TUI and pricing code consult the registry; `KNOWN_MODELS` becomes the
  built-in pack/fallback.
- **Routing change (remove slash-encoding):** internal request carries
  `(provider, id)`. The only `/`-split is a single first-`/` at the UI/CLI
  boundary to recover the provider key from forms like `fireworks/...`; the model
  `id` is never re-parsed. The old `parse_qualified_model` reject-on-nested-slash
  rule is **removed**, not patched.
- **Wire protocol selection:** the model's `api` field picks one of the three
  kernel-owned stream implementations (ADR-0001) via an `api`-keyed registry
  (OpenAI/Anthropic exist; **add native `google-generative-ai`**). Unknown `api`
  → validation error.
- **Reload (ADR-0002):** a single shared `RuntimeReload` component (mtime+hash
  gate) consulted at **top of turn** (agent loop / REPL) and on registry-consuming
  ops; rebuilds the registry/provider set behind an `ArcSwap`/`RwLock`. Same
  component feeds skills (#1) and workflow-set (#3) rebuilds. Fail-safe: keep
  last-good on error.
- **Value resolution (FR5):** `$ENV`/`${ENV}`/`!command`/literal with `$$`/`$!`
  escapes, resolved at request time; status checks never execute commands.
- **Stays out of the kernel:** no community wire protocols (ADR-0001), no UDS
  `register_provider` (ADR-0003), no `LlmProvider` trait change. Everything new is
  data + a reload path + one new (kernel-owned) wire impl.

### What this deliberately leaves in the kernel
- The `LlmProvider` trait and the **three** wire implementations
  (OpenAI/Anthropic + new native Gemini).
- The routing/resolution logic (we *replace* slash-encoding with opaque-id lookup,
  a deliberate fix — not a contract change for the community).
- The reload mechanism itself (engine), with the *content* (config/registry)
  owned by the community.

---

## 7. Rollout / phasing

1. **Phase 1 (unblocks Fireworks now):** FR2 — replace slash-encoding with opaque
   `(provider, id)` resolution; remove the reject-on-nested-slash rule; first-`/`
   split only at the UI boundary. Ship via the feature workflow; reinstall.
   Config-only Fireworks works (still restart-bound until Phase 2).
2. **Phase 2 (autonomy — ADR-0002):** FR1 shared reload component (top-of-turn +
   on-consume, mtime/hash gate, fail-safe) for providers/models. Models added
   mid-session work next turn.
3. **Phase 3 (discoverability + metadata):** FR3 `models.json` registry
   (providers-own-models, `api` field, optional metadata, merge semantics) + FR5
   value resolution; demote `KNOWN_MODELS` to the built-in pack; TUI reads runtime
   list.
4. **Phase 4 (ergonomics, optional):** FR4 `model` tool action for self-extension;
   extend the shared reload component to knowledge (#1) and workflow-set (#3).
   (UDS `register_provider` remains deferred per ADR-0003; sidecar-rewrites-
   `models.json` is the interim dynamic-discovery bridge.)

**Fast-follow (separate feature-workflow change, after Phases 1–3):**
add the third kernel wire protocol (`google-generative-ai`, native Gemini) behind
the `api`-keyed registry (ADR-0001), making "OpenAI/Anthropic/Google compatible"
literally true. Reference: pi's `google.ts`. This is intentionally *not* gated
inside this PRD's core phases so the registry + reload work ships first.

---

## 8. Open questions (for our discussion)

**Resolved (decisions recorded above):**
- **Q1 — bare multi-segment models → RESOLVED.** No longer applicable: model `id`
  is opaque (FR2), never re-split. Only a single first-`/` split at the UI
  boundary recovers the provider key.
- **Q2 — registry shape/location → RESOLVED (FR3):** single `~/.quecto/models.json`
  (pi-shaped, providers-own-models); **not** a `config.json` section.
- **Q4 — reload trigger → RESOLVED by
  [ADR-0002](kernel-boundary.md#adr-0002--reload-trigger-for-startup-loaded-surfaces):**
  hybrid (top-of-turn + on-consume) behind an mtime/hash gate; one shared
  mechanism. (Note: ADR-0002 is **pull**; tools stay **push** — see ADR-0002's
  push-vs-pull clause. Phase 2 must build the pull trigger; it cannot reuse the
  tool path.)
- **Q5 — built-in metadata pack → RESOLVED (FR3):** ship a small curated built-in
  pack, overridable by `models.json`.
- **Q8 — native Gemini scope → RESOLVED (NG1):** **fast-follow** — its own
  feature-workflow change immediately after Phases 1–3; reference pi's `google.ts`.

**Still open:**
- **Q3 / FR4:** Is a first-class `model` tool (Phase 4) worth building, or is
  "agent edits `models.json` via write tool + reload" enough for the foreseeable
  future? (Autonomy bar is already met without it. Lean: defer.)
- **Q6:** Fold the model list into the proposed generalized knowledge-retrieval
  surface, or keep models a distinct surface? (Lean: distinct — models carry
  request-routing/auth, not just text.)
- **Q7 → ADR-0007 (reload-trust):** re-reading config each turn (ADR-0002) means a
  process that can rewrite config can change *endpoints* between turns. Same trust
  level as writing config at all, but do we want a higher bar
  (confirmation/trust check) for **endpoint/key changes** vs. **model/knowledge
  additions**? Tracked as a future **ADR-0007**, not blocking Phases 1–3.
