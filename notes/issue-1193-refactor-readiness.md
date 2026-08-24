# Issue #1193 refactor workflow readiness gate

Issue: #1193 — Converge on one effective provider/model catalogue and application composition path
Workflow: `refactor`
Step: 2 — Read the issue; run the readiness gate; write the parity contract

## Inputs verified

- Full issue and comments read with:
  - `gh issue view 1193 --json title,body,comments`
- Comments: none.
- Local quality hooks were installed before this readiness work.

## Readiness gate result

**BLOCKED.** The selected `refactor` workflow requires a zero-behaviour-change issue before characterization and implementation. Issue #1193 is an architecture/product feature issue that intentionally changes externally observable provider/model catalogue, refresh, reload, extensibility, availability, and runtime-publication behaviour.

The workflow's blocking instruction says to stop and report when any blocking condition is true. The blocking conditions below are present.

## Blocking findings

### 1. The issue does not explicitly mandate zero behaviour change

Issue #1193 does not say observable behaviour must remain unchanged. It requires new or changed behaviour including:

- one effective provider/model catalogue used by all consumers;
- user-owned provider/model configuration and overrides;
- data-only provider/model extension using existing adapters;
- application-level refresh with per-source outcomes;
- TUI slash-command refresh path;
- valid hot reload without restarting Quecto/TUI;
- structured known/configured/available/runnable/unavailable reasons;
- atomic generation-consistent catalogue/runtime publication;
- failed reload/refresh retaining the last valid generation;
- no remote calls during ordinary catalogue reads or local hot reload.

What good looks like for this workflow:

> This is a pure refactor. Existing CLI, UDS/API, TUI, runtime routing, discovery persistence, model selection, configuration, errors, logging, and compatibility behaviour must remain unchanged. New catalogue extension, refresh, reload, availability, generation, and migration behaviours are out of scope.

### 2. The issue mixes behavioural change with refactoring

Issue #1193 includes Clean Architecture refactoring, but also product behaviours and acceptance criteria that are not currently parity-preserving:

- a documented user configuration surface for adding providers/models;
- a data-only provider extension path;
- a data-only model extension/override path;
- refresh all/selected sources with per-source outcomes;
- refresh isolation/cancellation semantics;
- reload semantics visible without restarting;
- structured non-runnable reasons;
- generation consistency between catalogue and routing;
- TUI refresh command support.

What good looks like for this workflow:

> Split #1193 into a pure zero-behaviour-change refactor, followed by feature issues for new refresh/reload/extensibility/availability/generation behaviours.

### 3. The touched observable surfaces cannot be enumerated as an identical-afterwards parity contract

The issue touches too many surfaces that are required to change or become newly defined:

- CLI model/provider listing;
- CLI discovery/refresh;
- CLI agent-provider composition/startup;
- UDS catalogue queries;
- UDS model switching;
- TUI model listing/selection;
- TUI refresh slash command;
- agent startup;
- child-agent startup;
- runtime provider reload;
- runtime routing and provider/model selection;
- `provider/model` serialized compatibility;
- `models.json` parse/merge/persist/migration;
- built-in registry metadata;
- user config provider/default-model settings;
- credential/auth resolution;
- OpenAI-compatible discovery;
- Anthropic/unsupported-provider handling;
- reasoning-effort/capability inference;
- missing-credential, unsupported-transport, malformed-source failures;
- snapshot generation/publication;
- rollback after failed refresh/reload;
- #1273 future policy boundary.

A parity contract that promises these behaviours remain identical would contradict #1193's acceptance criteria.

What good looks like for this workflow:

> A per-surface parity list of current behaviours to preserve unchanged, with new behaviours explicitly excluded from the issue.

### 4. Test/render harness sufficiency cannot be established for a zero-change refactor spanning all surfaces

The issue does not identify harnesses capable of pinning every touched surface as unchanged. Harnesses would be required for at least:

- CLI contract/golden behaviour;
- UDS/API schema and response contracts;
- TUI render and command projection behaviour;
- application fake-port behaviour;
- infrastructure adapter persistence/discovery behaviour;
- runtime integration behaviour;
- failure/rollback behaviour;
- hot reload behaviour.

Because the issue requires intentional behaviour changes, such harnesses should test new feature contracts, not a zero-change refactor contract.

## Warning finding

Performance characteristics of replaced specialized code are not recorded.

Named consequence to carry forward if this work is converted to a suitable workflow:

> Allocation/complexity regressions will be undetectable unless the existing registry, discovery, projection, routing, and reload paths are characterized before replacement.

## Draft parity contract status

**Not approved.** It cannot be approved under the selected `refactor` workflow because the readiness gate failed and the issue intentionally requires changed/new behaviour.

### Non-approvable parity checklist

The following would need to be converted into an approved parity checklist only after #1193 is split or rewritten as a pure zero-behaviour-change refactor:

- [ ] CLI model/provider listing output, ordering, empty states, and errors remain unchanged.
- [ ] Existing discovery command behaviour, network timing/failure handling, and persistence remain unchanged.
- [ ] Existing UDS catalogue and model-switching schemas/results remain unchanged.
- [ ] Existing TUI `/model` rendering, selection, and failure text remain unchanged.
- [ ] Existing startup provider routing for every stable provider/model ID remains unchanged.
- [ ] Existing `models.json` parse/merge/persist behaviour remains unchanged.
- [ ] Existing config provider/default-model semantics remain unchanged.
- [ ] Existing credential-missing and unsupported-provider behaviours remain unchanged.
- [ ] Existing capability heuristics remain unchanged.
- [ ] Existing performance characteristics are characterized and preserved.

## Required unblock

Use a non-refactor feature/architecture workflow, or split #1193:

1. A pure refactor issue with explicit zero observable behaviour change and an approved parity contract.
2. Follow-up feature issues for effective-catalogue precedence, user extension, refresh, TUI refresh, hot reload, structured availability, atomic generation publication, migration, and #1273 integration.

## Characterization coverage identified on unmodified code

The following existing tests were run before production-code edits to pin current behaviour for the provider/model surfaces that #1193 touches. One attempted multi-filter cargo invocation failed because `cargo test` accepts only one test-name filter; it was replaced with separate focused invocations.

- `cargo test -p quecto-agentic-harness --lib model_registry`
  - Result: GREEN, 28 passed.
  - Maps to: built-in registry contents, user `models.json` defaults, stable provider/model IDs, overrides by provider+id, auth identity separation, API protocol validation, env interpolation, explicit limits, fallback limit accessors, malformed registry failures.
- `cargo test -p quecto-agentic-harness --lib uds_models`
  - Result: GREEN, 2 passed.
  - Maps to: UDS catalogue projection fields, configured flag, registry-error projection.
- `cargo test -p quecto-agentic-harness --lib discover`
  - Result: GREEN, 59 passed.
  - Maps to: current CLI-owned discovery command behaviour, URL validation, bounded fetch handling, provider-targeted merge/persistence, OAuth unsupported result, no unrelated-provider clobbering.
- `cargo test -p quecto-agentic-harness --lib agent_provider`
  - Result: GREEN, 53 passed.
  - Maps to: current CLI-owned runtime composition, built-in provider construction, OAuth/API-key coexistence, registry provider construction/skipping/rejection, duplicate prefix behaviour, remote HTTP rejection, no-provider error.
- `cargo test -p quecto-agentic-harness --lib provider_reload`
  - Result: GREEN, 6 passed.
  - Maps to: current reload polling/force behaviour, unchanged config not rebuilding, malformed config error, changed config reload.

These are characterization anchors for the current implementation. They do not make the #1193 parity contract approvable as a pure zero-change refactor; they only document the currently green coverage before edits.

## Mutation/falsifiability log

Temporary mutations were applied and reverted cleanly before production edits. A post-mutation `git diff` over the touched production files was empty, and the characterization set returned GREEN.

- `registry_custom_models_override_builtin_by_provider_and_id`
  - Mutation: drop the existing-record replacement in registry upsert.
  - Observed failure: display name stayed `GPT 5.5 (API key)` instead of `Custom GPT`.
- `list_models_data_serializes_registry_models`
  - Mutation: hard-code UDS `configured` to `false`.
  - Observed failure: configured provider projected `false` instead of `true`.
- `discover_replaces_only_target_provider_models_and_preserves_auth`
  - Mutation: publish an empty discovered model list instead of fetched entries.
  - Observed failure: persisted provider `models` became `[]` instead of the discovered `alpha`/`beta` entries.
- `test_build_agent_provider_registry_anthropic_api_key_provider`
  - Mutation: skip pushing the registry-created provider into the runtime list.
  - Observed failure: runtime provider names contained only `openai-api`, not `anthropic-api`.
- `changed_poll_reloads_new_provider`
  - Mutation: on successful poll rebuild, return `ReloadResult::Unchanged` instead of recording/publishing the rebuilt provider.
  - Observed failure: test expected `Reloaded(_)` and received unchanged.

Exploratory mutation note: removing `models.json` from the reload watch-source list did not fail `changed_poll_reloads_new_provider` because that test changes the config file, not only `models.json`. That gap is recorded for follow-up coverage when adding generation-consistent catalogue/runtime reload tests.

## Post-mutation green verification

- `cargo test -p quecto-agentic-harness --lib model_registry` — GREEN, 28 passed.
- `cargo test -p quecto-agentic-harness --lib uds_models` — GREEN, 2 passed.
- `cargo test -p quecto-agentic-harness --lib discover` — GREEN, 59 passed.
- `cargo test -p quecto-agentic-harness --lib agent_provider` — GREEN, 53 passed.
- `cargo test -p quecto-agentic-harness --lib provider_reload` — GREEN, 6 passed.

## Step 5 characterization review dispositions

### Falsifiability finder

- Concern: broad readiness/parity checklist is not fully falsifiable by the referenced tests.
  - Disposition: ACCEPTED AS A KNOWN LIMIT OF #1193 UNDER `refactor`. The checklist remains marked non-approvable. Implementation verification will add architecture/application/domain/infrastructure/interface tests for new #1193 behaviour rather than pretending broad product changes are zero-change parity.
- Concern: several legacy `agent_config_tests.rs` provider-build assertions only assert `is_ok()`.
  - Disposition: ACCEPTED. Stronger provider-name/routing assertions already exist for registry OAuth/API-key paths and will be supplemented by application-composition tests in the implementation slice.
- Concern: `cmd_models_discover_success_reports_count_and_accepts_interval` is weak.
  - Disposition: ACCEPTED. Discovery will be moved behind an application refresh use case and tested at that boundary; the old CLI-specific count assertion is not treated as complete parity coverage.
- Concern: private-helper/test-constructed-state assertions in `agent_provider_cov_tests.rs` are weaker than end-to-end observable checks.
  - Disposition: ACCEPTED. They remain useful focused characterization but are not cited as the sole proof of provider/runtime composition.

### Coverage finder

- Concern: numeric limit boundaries are incomplete on some consuming surfaces.
  - Disposition: PARTIALLY DECLINED FOR FREEZE, ACCEPTED FOR IMPLEMENTATION TESTING. Existing application-layer clamp tests already cover above, below, and omitted `maxTokens`; context-window application tests cover above/below/omitted and set_model rederive. UDS/CLI duplicate-boundary expansion is deferred to implementation tests where catalogue resolution and selection use cases become the shared seam.
- Concern: explicit-vs-synthesized numeric limits are only pinned at registry lookup and some consumers.
  - Disposition: ACCEPTED. New domain/application catalogue tests will distinguish declared values from defaults when translating the legacy registry.
- Concern: UDS catalogue projection does not explicitly assert every numeric field/explicitness case.
  - Disposition: ACCEPTED. New projection tests will be added if UDS payload shape changes; current compatibility tests remain anchors.
- Concern: discovery replacement of prior per-model metadata is only indirectly documented.
  - Disposition: ACCEPTED. Application refresh-source tests will make replacement/retention semantics explicit.
- Concern: `models.json`-only reload watch-source gap.
  - Disposition: ACCEPTED. New application snapshot-store/reload tests will cover local catalogue publication without rebuilding unrelated state.

### Gherkin finder

- Result: NO FINDINGS. The characterization set is Rust unit/lib tests, not Gherkin scenarios.
