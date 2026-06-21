# Design: RuntimeReload — Phase 2 (Auto-load for Models/Providers)

**Status:** Draft for review (post-review revisions applied)
**PRD:** [prd-models-runtime-extensible.md](prd-models-runtime-extensible.md) Phase 2 / FR1
**ADR:** [ADR-0002](kernel-boundary.md#adr-0002--reload-trigger-for-startup-loaded-surfaces) (hybrid trigger, pull mechanism)
**Scope:** This design covers **only** the models/providers consumer (Phase 2). The
shared `RuntimeReload` component is shaped so knowledge (#1) and workflow-set (#3)
can plug in later without rework, but those consumers are out of scope here.

> **Why `RuntimeReload<T>` stays generic.** ADR-0002 explicitly mandates "one
> `RuntimeReload`-style component … with three consumers." We are building for a
> future community and agent self-evolution, where knowledge (#1) and workflow-set
> (#3) auto-load on the same mechanism. A narrow provider-only gate would
> guarantee rework. The generic shape is the *point*, not speculative generality.

---

## 1. Problem (from the PRD)

Providers/models come from `Config::load_with_env`, read **once at startup**
(`src/interface/cli/agent.rs:314`). A provider or model added to
`~/.quecto/config.json` mid-session is invisible until restart. This breaks the
autonomy contract (ADR-0002): an agent that adds a model cannot *use it next turn*.

Phase 1 (opaque model IDs) is already shipped — the router correctly handles
multi-segment IDs like `fireworks/accounts/fireworks/models/glm-5p2`. The
remaining blocker is the **reload trigger**.

---

## 2. Design constraints (from the codebase)

| Constraint | Evidence | Implication |
|---|---|---|
| `AgentLoopImpl.provider` is `Arc<dyn LlmProvider>` | `agent_loop.rs:55` | Swapping needs `&mut` (between turns) or interior mutability. |
| `run_loop(&self)` is immutable | `agent_loop.rs:553` | Cannot mutate `provider` inside the tool-iteration loop. |
| `swap_registry(&mut self)` exists | `agent_loop.rs:159-167` | Proves `AgentLoopImpl` supports explicit between-turn swaps of live state. **Note:** the UDS dispatch loop does not currently *call* `swap_registry`; the real precedent for `&mut AgentLoopImpl` at the turn boundary is `DispatchCtx.agent` (`uds.rs:339-340`), which already mutates `ctx.agent` in `handle_set_model` (`uds.rs:417`). |
| `DispatchCtx` carries `&'a mut AgentLoopImpl` through the dispatch path | `uds.rs:279-340`; `handle_set_model` mutates it at `:417` | **The actual precedent:** the dispatch loop already holds `&mut` to the agent between `process()` calls. `swap_provider` rides this same path. |
| `build_agent_provider(&config, base_dir, http_client)` is a pure rebuild | `agent_provider.rs:40` | Re-callable to rebuild the `ProviderRouter` from fresh config. |
| No `arc-swap` / `parking_lot` dependency | `Cargo.toml` | Use `std` + `tokio` only. No new deps for Phase 2. |
| Config files are small (KB) | `~/.quecto/config.json` | In-memory hash is trivial; no streaming concern. |

---

## 3. The mechanism: pull trigger, shared component

### 3.1 `RuntimeReload` — the change-detection gate (ADR-0002 §3)

A single infrastructure component that answers one question cheaply: **"did any
watched source change since last poll?"** It does not know *what* a source means
— it knows how to `stat`, hash, and gate a rebuild. It is **synchronous**: the
sources are small local config files and the rebuild closure is synchronous
(`Config::load_with_env` + `build_agent_provider`). Declaring `poll` `async`
while performing only blocking IO would be misleading; if a future consumer needs
async file IO (e.g. knowledge indexing many files), it gets a separate async
path then — Phase 2 stays sync.

```rust
// src/infrastructure/reload.rs

/// A file-backed source whose content may change at runtime.
#[derive(Clone)]
pub struct ReloadSource {
    path: PathBuf,
    /// Last observed mtime (None = not yet seeded).
    last_mtime: Option<SystemTime>,
    /// Last observed content hash.
    last_hash: u64,
}

/// The result of probing one source. The observed fingerprint (mtime/hash)
/// is advanced whenever we successfully read the file, *independent* of
/// whether the caller's rebuild later succeeds — so a malformed file is not
/// re-parsed every turn until it changes again.
#[derive(Debug)]
enum SourceChange {
    /// mtime identical to last poll — no read, no hash (cheap path).
    UnchangedNoRead,
    /// mtime moved but content hash is identical — mtime cache updated, no
    /// rebuild needed (touch-only).
    Unchanged,
    /// mtime moved and content hash differs — rebuild needed.
    Changed,
    /// stat or read failed / file missing — keep last-good, do not touch cache.
    MissingOrUnreadable,
}

impl ReloadSource {
    /// Construct an unseeded source (last_mtime=None, last_hash=0).
    pub fn new(path: impl Into<PathBuf>) -> Self { /* ... */ }

    /// Seed the fingerprint from the file's current state without flagging
    /// a change. Called once at startup so the first top-of-turn poll does
    /// not spuriously rebuild from the just-loaded config.
    pub fn seed(&mut self) { /* stat + read + hash, store, return no change */ }

    /// Probe the source, advancing the observed mtime/hash cache per the
    /// state machine below. Returns whether the caller should rebuild.
    pub fn changed(&mut self) -> SourceChange {
        // 1. stat → mtime.
        //    - fail/missing → MissingOrUnreadable (cache untouched, keep last-good).
        //    - mtime == last_mtime → UnchangedNoRead (no hash, cheapest path).
        //    - mtime != last_mtime → read + hash (below).
        // 2. read bytes → hash.
        //    - read fail → MissingOrUnreadable.
        // 3. Compare hash to last_hash; ALWAYS update last_mtime to the new mtime
        //    (so a touched-but-identical file does not re-hash forever).
        //    - hash == last_hash → Unchanged (mtime cache advanced).
        //    - hash != last_hash → Changed (update last_hash too).
    }
}

/// The shared reload gate. Holds one or more sources and a fail-safe
/// last-good snapshot of whatever the caller rebuilt from them.
///
/// Two caches are kept distinct:
///   - `sources[i].last_mtime/last_hash`  → the last *observed* file content
///     (advanced on every successful read, even when rebuild fails, so a
///     malformed file is not retried every turn until it changes again).
///   - `last_good: Option<T>`             → the last *successfully rebuilt*
///     value (swapped only on a successful rebuild; kept on rebuild error).
pub struct RuntimeReload<T> {
    sources: Vec<ReloadSource>,
    last_good: Option<T>,
}

impl<T: Clone> RuntimeReload<T> {
    pub fn new(sources: Vec<ReloadSource>) -> Self { /* ... */ }

    /// Seed every source's fingerprint from disk and store `initial` as
    /// last_good. Call once at startup, right after the startup config load,
    /// so the first top-of-turn poll is a no-op when nothing has changed.
    pub fn seed(&mut self, initial: T) {
        for s in &mut self.sources { s.seed(); }
        self.last_good = Some(initial);
    }

    /// Synchronous poll. Probe all sources; if any is `Changed`, call
    /// `rebuild`. The observed-fingerprint cache is advanced inside
    /// `changed()` regardless of the rebuild outcome.
    ///
    /// - No source changed → `Unchanged` (cost: N `stat` calls).
    /// - Rebuild succeeds  → swap `last_good`, return `Reloaded(new)`.
    /// - Rebuild fails      → keep `last_good`, log warning, return `Unchanged`
    ///   (fail-safe — ADR-0002 §4). The observed fingerprint has already
    ///   advanced, so the malformed file is not re-parsed next turn.
    pub fn poll(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        let any_changed = self
            .sources
            .iter_mut()
            .any(|s| matches!(s.changed(), SourceChange::Changed));
        if !any_changed {
            return ReloadResult::Unchanged;
        }
        match rebuild() {
            Ok(new) => {
                self.last_good = Some(new.clone());
                ReloadResult::Reloaded(new)
            }
            Err(e) => {
                tracing::warn!(target: "reload", error = %e, "reload rebuild failed; keeping last-good");
                ReloadResult::Unchanged
            }
        }
    }

    /// Force a poll that bypasses the mtime/hash gate (explicit `reload`
    /// command). Rebuild is attempted even when sources report unchanged.
    pub fn poll_forced(&mut self, rebuild: impl FnOnce() -> Result<T, String>) -> ReloadResult<T> {
        match rebuild() {
            Ok(new) => {
                for s in &mut self.sources { s.seed(); }
                self.last_good = Some(new.clone());
                ReloadResult::Reloaded(new)
            }
            Err(e) => {
                tracing::warn!(target: "reload", error = %e, "forced reload failed; keeping last-good");
                ReloadResult::Unchanged
            }
        }
    }
}

pub enum ReloadResult<T> {
    Unchanged,
    Reloaded(T),
}
```

**Why generic `<T>`:** Phase 2 instantiates `T = Arc<dyn LlmProvider>` (the rebuilt
`ProviderRouter`). Later, knowledge instantiates `T = KnowledgeIndex`, workflow-set
instantiates `T = Vec<WorkflowTemplate>`. One gate, typed per consumer. This is
the "one shared mechanism, N consumers" shape ADR-0002 mandates — and we are
building for that future explicitly, not as speculative generality (see the
callout at the top of this doc).

**Concrete `last_good` type for Phase 2:** `T = Arc<dyn LlmProvider>`. `Arc` is
`Clone` (cheap refcount bump), so `last_good: Option<Arc<dyn LlmProvider>>` is
trivially storable. `last_good` is the **fail-safe snapshot** — distinct from the
live `AgentLoopImpl.provider`, which is the actually-in-use provider. On rebuild
error we keep `last_good` (and the live provider is untouched); the design's
earlier `/* arc or clone */` hand-wave is now concrete.

**Hash choice:** `std::collections::DefaultHasher` over file bytes. This is a
**process-local, non-persistent change detector within a session**, not a
security boundary or a cross-process stable ID. It's already in `std`, zero new
deps. The hash is only ever compared against an in-memory `last_hash` from the
same process; it never survives a restart (on restart, the source is re-read
fresh and re-seeded). If cross-session stability is ever needed (it isn't for
this use case), swap to `sha2` then.

**Fail-safe:** a malformed config on reload logs a warning and keeps the
**last-good** provider set. The session is never crashed by a bad edit (AC7).
Crucially, the **observed** fingerprint is advanced even on rebuild failure, so
a malformed file does not trigger a re-parse every turn until it is fixed.

### 3.2 The models/providers consumer

The consumer is **not** a new struct — it's the existing `build_agent_provider`
re-called inside the `rebuild` closure. The `RuntimeReload<Arc<dyn LlmProvider>>`
is constructed once at startup, **seeded** with both the initial provider *and*
the source fingerprints, and re-polled each turn.

```rust
// In the UDS dispatch / REPL construction (src/interface/cli/agent.rs or a new helper):

let config_path_owned = config_path.to_path_buf();
let base_dir_owned = base_dir.to_path_buf();
let http_client_owned = http_client.clone();

let initial_provider = build_agent_provider(&config, &base_dir, &http_client)?;

let mut provider_reload = RuntimeReload::new(vec![
    ReloadSource::new(&config_path_owned),
]);

// Seed BOTH the source fingerprints (so the first top-of-turn poll does not
// spuriously rebuild from the just-loaded config) and last_good (so a
// rebuild failure on turn 1 keeps the startup provider). This must run
// immediately after the startup Config::load_with_env that produced `config`.
provider_reload.seed(initial_provider.clone());
```

**Default-vs-explicit config:** `build_agent_from_config` allows a *missing
default* config as zero-config but errors on a *missing explicit* `--config`
(`explicit_config_missing`). Seeding handles both: if the default config is
absent at startup, `ReloadSource::seed()` treats the missing file as
`MissingOrUnreadable` (cache untouched); if the file is created later, the next
poll sees `last_mtime == None` vs a real mtime → `Changed` → rebuild. An explicit
config that is missing never reaches reload (startup errors first).

### 3.3 Top-of-turn reload (the guarantee)

The reload runs **before** `agent.process(...)` at each turn boundary. There are
two call sites that must be wired:

1. **UDS dispatch loop** — `handle_prompt` in `src/interface/cli/uds.rs:553`,
   before `run_prompt_dispatch`. The dispatch context (`DispatchCtx`) gains a
   `provider_reload: &mut RuntimeReload<Arc<dyn LlmProvider>>` field (see §4).
2. **REPL / one-shot** — `src/interface/cli/agent.rs:479` and `:530`, before
   `rt.block_on(agent.process(...))`. For a *one-shot* CLI run, config was just
   loaded at construction and the process exits after one turn, so top-of-turn
   reload is a no-op by the seed (B3) and adds no value — but it is wired for
   consistency and to keep the two paths identical. The meaningful interactive
   reload path is UDS; the one-shot path is degenerate.

```rust
// Top-of-turn, before process() — synchronous poll (no .await):
let outcome = provider_reload.poll(|| {
    let config = Config::load_with_env(&config_path, &env_overrides)?;
    build_agent_provider(&config, &base_dir, &http_client)
});
if let ReloadResult::Reloaded(new_provider) = outcome {
    agent.swap_provider(new_provider);  // new method, mirrors swap_registry
}
```

**Borrow composition:** `provider_reload.poll(...)` borrows `ctx.provider_reload`
mutably and returns an owned `ReloadResult`; only afterward does `ctx.agent`
get borrowed for `swap_provider`. Two sequential `&mut` borrows of disjoint
`DispatchCtx` fields — they compose. The rebuild closure must **not** capture
`ctx`; it captures owned `config_path`/`base_dir`/`http_client`/`env_overrides`
clones (see §4).

**Cost when unchanged:** one `stat()` per source per turn. For one config file,
that's one syscall — effectively zero (AC6).

### 3.4 On-consume reload (freshness for interactive use)

Operations that read the provider/model set should see edits immediately, without
waiting for a turn boundary. Phase 2 wires **one** on-consume path:

- **`set_model` UDS command** (`handle_set_model`, `uds.rs:401`) — before resolving
  the model string, call `provider_reload.poll(...)` and `agent.swap_provider(...)`
  if reloaded. This is where an agent that just wrote a new provider to config via
  `write`/`edit` sees it land before selecting a model on it.

**Deferred to Phase 3 (not Phase 2):** the TUI `/model` selector runtime list.
Today `/model` is a hardcoded `KNOWN_MODELS` constant (`model_selector.rs:15`); the
PRD makes the runtime `ModelRegistry` + `models.json` list a **Phase 3** deliverable
(FR3 / AC4). Wiring `/model` to read provider prefixes from reloaded config in
Phase 2 would build a temporary half-registry that Phase 3 replaces. **Scope
discipline:** Phase 2 on-consume is `set_model` only; `/model` list freshness
lands with the registry in Phase 3. (The TUI may still *call* a provider reload on
open if trivial, but it must not implement a new model-list source.)

### 3.5 Explicit reload trigger (FR1 / ADR-0002)

PRD FR1 requires an explicit `reload` trigger as a convenience (not the only path);
ADR-0002 rejects explicit-*only* reload but keeps the explicit path. Phase 2 adds a
new UDS command:

```jsonc
{ "type": "reload", "id": "..." }   // or "reload_models"
```

It calls `provider_reload.poll_forced(...)` (bypasses the mtime/hash gate — the
caller asked for "reload now" even when mtime is unchanged) and swaps the provider
on success. On failure it keeps last-good and returns an error event. This is the
*third* trigger, alongside top-of-turn and on-consume; it does not replace either.

This is a new `AgentCommand` variant — a UDS protocol addition. Per
`kernel-boundary.md`, a UDS protocol change is a kernel change (deliberately ours
to own); it is in-scope for Phase 2 because FR1 calls for it. (Contrast: ADR-0003
defers `register_provider`, a much heavier verb; `reload` is a thin convenience
over the existing reload machinery, not a new registration surface.)

### 3.6 `AgentLoopImpl::swap_provider` — the swap point

Mirror the existing `swap_registry` pattern exactly:

```rust
// src/application/agent_loop.rs

/// Replace the LLM provider with a rebuilt one (e.g. after a config reload).
///
/// Zero-overhead: one `Arc` swap, O(1). Called between `process()` calls
/// by the dispatch loop when the reload gate detects a config change.
/// No changes to the hot path (`run_loop`, `call_provider_with_retries`).
pub fn swap_provider(&mut self, provider: Arc<dyn LlmProvider>) {
    self.provider = provider;
}
```

**Why between-turns `&mut` and not `ArcSwap` inside `run_loop`?** `swap_registry`
proves `AgentLoopImpl` is designed for explicit between-turn swaps; the real
guarantee that the dispatch loop holds `&mut AgentLoopImpl` at the turn boundary
comes from `DispatchCtx.agent` (§2). Adding `arc-swap` for interior mutability
would be a new dependency and a new concurrency model for zero benefit — the
reload runs at turn boundaries, not mid-tool-iteration. **YAGNI: mirror the
proven pattern.**

---

## 4. Where the reload state lives

The `RuntimeReload<Arc<dyn LlmProvider>>` is owned by the **session dispatch
context**, not by `AgentLoopImpl`. Rationale:

- `AgentLoopImpl` is the hot path; it should not own file-watching state.
- The dispatch loop already owns session-level mutable state (`DispatchCtx`).
- For sub-agents (`spawn`), the child builds its own provider at construction
  and gets its own `RuntimeReload` seeded with that provider. A child's config
  path is the same as the parent's, so a child also auto-reloads — but this is
  a natural consequence, not extra wiring.

**Sub-agent reload — inherited by construction, not deferred.** A spawned child
is a fresh `quecto agent --mode uds` process running the *same binary* through
the *same* `build_agent_from_config` → `Config::load_with_env` →
`build_agent_provider` construction path, inheriting the parent's config path
(`src/infrastructure/tools/spawn.rs:320+`:

```rust
let mut cmd = tokio::process::Command::new(&binary);
cmd.arg("agent").arg("--mode").arg("uds")...
if let Some(cfg_path) = effective_config_path(...) { cmd.arg("--config").arg(cfg_path); }
```

Children are **exact replicas of the parent** — they inherit every feature wired
into the `agent` construction path, including Phase 2's reload. Whatever reload
machinery §3 wires into `build_agent_from_config` → `UdsLoopArgs` → `DispatchCtx`
is picked up by *every* child automatically, with zero spawn-side wiring. A
long-lived UDS child watches the same config file and reloads at its own turn
boundaries, identical to the parent. Short-lived one-shot children exit before a
second turn matters, so the seeded reload simply never fires — harmless. **There
is no deferral; there is no special case.** The earlier "Deferred" language is
retracted.

### Wiring through the construction path

The reload state and its rebuild inputs must flow from `build_agent_from_config`
to the dispatch loop. Concretely:

1. **`AgentBuildResult`** (`agent.rs`) gains the reload handle and rebuild inputs:
   ```rust
   pub(crate) struct AgentBuildResult {
       pub agent: AgentLoopImpl,
       // ...existing fields...
       pub provider_reload: RuntimeReload<Arc<dyn LlmProvider>>,
       /// Owned rebuild inputs captured by the closure (see below).
       pub provider_reload_inputs: ProviderReloadInputs,
   }
   ```
2. **`ProviderReloadInputs`** — an owned bundle so the rebuild closure does not
   borrow `ctx` or any `DispatchCtx` field:
   ```rust
   pub(crate) struct ProviderReloadInputs {
       pub config_path: PathBuf,
       pub base_dir: PathBuf,
       pub env_overrides: HashMap<String, String>,
       pub http_client: reqwest::Client,
   }
   impl ProviderReloadInputs {
       fn rebuild(&self) -> Result<Arc<dyn LlmProvider>, String> {
           let config = Config::load_with_env(&self.config_path, &self.env_overrides)
               .map_err(|e| e.to_string())?;
           build_agent_provider(&config, &self.base_dir, &self.http_client)
               .map_err(|e| e.to_string())
       }
   }
   ```
3. **`UdsLoopArgs`** (`uds.rs`) gains `provider_reload` and
   `provider_reload_inputs` so both single-client and multi-client paths receive
   them.
4. **`DispatchCtx`** (`uds.rs:339`) gains two fields:
   ```rust
   pub provider_reload: &'a mut RuntimeReload<Arc<dyn LlmProvider>>,
   pub provider_reload_inputs: &'a ProviderReloadInputs,
   ```
   Both single-client (`uds.rs:191`) and multi-client (`uds_multi.rs:161`)
   construction sites, plus every test fixture that builds `DispatchCtx`
   directly (`uds_dispatch_cov_tests.rs`, `uds_workflow_automation_tests.rs`),
   are updated.
5. **Top-of-turn call site** (`handle_prompt`, `uds.rs:553`) runs the poll before
   `run_prompt_dispatch`, using `ctx.provider_reload_inputs.rebuild` as the
   closure — disjoint-field sequential borrows (§3.3).

**Why owned inputs, not a closure capturing `ctx`:** the rebuild closure must be
`FnOnce() -> Result<T, String>` with no borrow of `ctx`, so the `&mut` borrow of
`ctx.provider_reload` ends before `ctx.agent` is borrowed for `swap_provider`.
Bundling the inputs in an owned struct makes this trivially sound.

---

## 5. Clean architecture placement

| Layer | Module | Responsibility |
|---|---|---|
| Infrastructure | `src/infrastructure/reload.rs` (new) | `ReloadSource`, `SourceChange`, `RuntimeReload<T>`, `ReloadResult` — file stat/hash gate, fail-safe, seeding, forced poll. No domain knowledge. |
| Interface/CLI | `src/interface/cli/agent_provider.rs` (existing) | `build_agent_provider` — the rebuild body. Unchanged. |
| Interface/CLI | `src/interface/cli/agent.rs` | `ProviderReloadInputs` (owned rebuild bundle); seed `RuntimeReload` in `build_agent_from_config`; thread it through `AgentBuildResult` → `UdsLoopArgs`. |
| Interface/CLI | `src/interface/cli/uds.rs` + `uds_multi.rs` | Add `provider_reload` + `provider_reload_inputs` to `DispatchCtx` and both construction sites; call `poll()` top-of-turn in `handle_prompt` and on-consume in `handle_set_model`; handle the new `reload` UDS command. |
| Application | `src/application/agent_loop.rs` | Add `swap_provider(&mut self)`. One method. No change to `run_loop`. |

**No domain layer change.** The `LlmProvider` trait, `ChatRequest`, `ProviderRouter`
are all untouched. The reload is pure infrastructure + interface wiring. This
respects the boundary: the kernel owns the *mechanism* (reload gate), the
community owns the *content* (config file).

---

## 6. What this does NOT do (scope discipline)

- **No `models.json` registry** — that's Phase 3 (FR3). Phase 2 reloads the
  existing `config.json` providers section. The `RuntimeReload<T>` is generic
  enough that Phase 3 adds a second `ReloadSource` (for `models.json`) and a
  second consumer closure, reusing the same gate.
- **No `ModelRegistry`** — Phase 3. Phase 2 still routes via `ProviderRouter`
  with opaque IDs (Phase 1, shipped).
- **No knowledge/workflow consumers** — ADR-0002 names them, but they're separate
  phases. The generic `RuntimeReload<T>` is the shared mechanism they will use.
- **No `arc-swap` dependency** — between-turns `&mut` swap mirrors `swap_registry`.
- **No UDS `register_provider`** — deferred per ADR-0003. (The new `reload` UDS
  command in §3.5 is a thin convenience trigger, *not* a registration surface —
  it rebuilds from the existing config file, it does not inject providers live.)
- **No native Gemini** — fast-follow, separate change.
- **No TUI `/model` runtime list** — Phase 3 (FR3/AC4). Phase 2 on-consume is
  `set_model` only.

---

## 7. BDD acceptance scenarios (Phase 2 subset of the PRD's ACs)

These map to the PRD's AC1, AC6, AC7. Written as testable scenarios. Note: these
scenarios split top-of-turn and on-consume behavior (they are distinct triggers),
and add malformed-then-fixed recovery, missing-source, and explicit reload cases
that the review flagged as gaps.

```gherkin
Feature: Provider/model reload (auto-load on next turn)

  # AC1 — auto-load, top-of-turn guarantee
  Scenario: A provider added to config mid-session is rebuilt before the next turn
    Given a running UDS session with only an OpenAI provider configured
    And the session has processed at least one turn (fingerprints seeded)
    When the agent writes a Fireworks endpoint to config.json via the write tool
    And the agent sends a prompt on the next turn
    Then the reload gate detects config.json changed (mtime moved + hash differs)
    And the ProviderRouter is rebuilt with the Fireworks provider before agent.process
    And a subsequent "set_model fireworks/accounts/fireworks/models/glm-5p2" routes to Fireworks

  # AC1 — auto-load, on-consume freshness (distinct from top-of-turn)
  Scenario: set_model triggers an on-consume reload before resolving the model
    Given a running session with no Fireworks provider
    And the session has processed at least one turn (fingerprints seeded)
    When the agent edits config.json to add Fireworks
    And immediately sends "set_model fireworks/accounts/fireworks/models/glm-5p2"
    Then the on-consume reload detects the change before resolving the model string
    And the Fireworks provider is swapped in
    And the model is selected without waiting for a turn boundary

  # AC6 — cheap no-op (steady state)
  Scenario: An unchanged config triggers no rebuild
    Given a running session with config loaded and fingerprints seeded
    When a turn starts and config.json has not been modified
    Then the reload gate stats config.json, finds mtime unchanged
    And no file read or hash occurs
    And no rebuild closure is called
    And no provider swap occurs

  # AC6b — touch-only (mtime moved, content same) — the bug the review caught
  Scenario: A touched-but-unchanged config triggers no rebuild and updates the mtime cache
    Given a running session with config loaded and fingerprints seeded
    When config.json is touched (mtime advances) but content is identical
    Then the reload gate reads and hashes the file once
    And finds the hash unchanged
    And no rebuild occurs
    And the mtime cache is advanced to the new mtime
    And a subsequent poll with no further change performs no file read (mtime matches)

  # AC7 — fail-safe (malformed)
  Scenario: A malformed config on reload keeps the last-good provider set
    Given a running session with a valid OpenAI provider
    When config.json is corrupted (invalid JSON) mid-session
    And a turn starts
    Then the reload gate detects a content change and attempts a rebuild, which fails
    And a warning is logged
    And the session keeps the last-good OpenAI provider (no swap)
    And the next prompt still routes to OpenAI (no crash)

  # AC7b — fail-safe recovery (observed fingerprint advances even on rebuild failure)
  Scenario: A malformed config is later fixed and reloads successfully
    Given a running session where a malformed config kept last-good OpenAI
    And no further turn has been taken since the malformed rebuild attempt
    When config.json is replaced with a valid Fireworks config (content changes)
    And a turn starts
    Then the reload gate detects the content change
    And the rebuild succeeds and the Fireworks provider is swapped in
    # This proves the malformed content was not re-parsed every turn while broken:
    And between the malformed attempt and the fix, turns did not re-attempt the rebuild

  # Missing / unreadable source
  Scenario: A missing or unreadable config keeps last-good and does not crash
    Given a running session with a valid OpenAI provider
    When config.json is deleted mid-session
    And a turn starts
    Then the reload gate reports the source missing/unreadable
    And the session keeps the last-good OpenAI provider
    And no crash occurs

  # Explicit reload trigger (FR1)
  Scenario: An explicit reload command rebuilds providers even when mtime is unchanged
    Given a running session with config loaded and fingerprints seeded
    When the client sends a "reload" command
    Then the forced reload bypasses the mtime/hash gate
    And the provider is rebuilt from the current config
    And a "set_model" to a newly-configured provider succeeds
  Scenario: An explicit reload on a malformed config keeps last-good and returns an error
    Given a running session with a valid OpenAI provider
    When config.json is corrupted
    And the client sends a "reload" command
    Then the forced reload attempt fails
    And the session keeps the last-good OpenAI provider
    And an error event is returned to the client
```

**Unit-vs-integration split.** The internal assertions ("rebuild closure is
called", "mtime cache is advanced") are unit-observable on `ReloadSource` /
`RuntimeReload` directly (instrument the rebuild closure with a counter; assert
on `SourceChange` variants). The UDS-facing scenarios ("set_model routes to
Fireworks", "no crash") are integration tests over the dispatch loop. Do not
assert on internal cache state from integration tests unless a tracing capture
hook exists.

---

## 8. Open questions / decisions

**Resolved during review:**

1. **`DispatchCtx` ownership & borrow composition.** `RuntimeReload` needs `&mut`
   to poll; `DispatchCtx` is passed `&mut` through the dispatch loop. The poll
   borrows `ctx.provider_reload`, returns an owned `ReloadResult`, and only then
   does `ctx.agent` get borrowed for `swap_provider` — two sequential `&mut`
   borrows of disjoint fields. **Composes.** The rebuild closure captures owned
   `ProviderReloadInputs`, never `ctx` (§4). **Decision: confirmed sound.**

2. **`env_overrides` capture.** The rebuild closure re-calls
   `Config::load_with_env(path, &env_overrides)`. The `env_overrides` map is built
   once at startup from `QUECTO_*` env vars. **Decision: startup-fixed.** A
   running process's environment is not normally mutated externally; config file
   changes reload, but external env changes require a process restart. (Phase 3
   `$ENV` value resolution for `models.json`, if implemented "at request time" per
   PRD AC10, may revisit this for *that* surface — not Phase 2.)

3. **Hash stability.** `DefaultHasher` is not stable across Rust versions.
   **Decision: acceptable** — the hash is process-local, non-persistent, and only
   compared within a single running session against an in-memory `last_hash`. It
   never survives a restart (on restart, the source is re-read fresh and
   re-seeded). No scenario requires cross-session stability.

4. **Sub-agent reload.** **Resolved — not deferred.** Spawned children are exact
   replicas of the parent: a fresh `quecto agent --mode uds` process through the
   same `build_agent_from_config` construction path, inheriting the parent's
   config path (`spawn.rs:320+`). Whatever reload machinery Phase 2 wires into the
   `agent` construction path is inherited by every child automatically — no
   spawn-side wiring, no special case. A long-lived UDS child reloads at its own
   turn boundaries identically to the parent; a short-lived one-shot child's
   seeded reload simply never fires. The earlier "Deferred" language is retracted.

**Still open (none blocking Phase 2 implementation):**

5. **`set_model` provider-availability validation.** Today `handle_set_model`
   (`uds.rs:417`) accepts any non-empty model string without checking the provider
   exists; a routing failure surfaces later during `agent.process`. Phase 2's
   on-consume reload makes the *provider* available, but `set_model` still does
   not validate it. **Decision for Phase 2:** "set_model succeeds" in the BDD
   means the string is accepted and the on-consume reload ran — actual routing
   success is asserted by the subsequent prompt, not by `set_model` itself. A
   dry-resolve validation check is a possible Phase 3/4 ergonomic add, not
   required for AC1.
