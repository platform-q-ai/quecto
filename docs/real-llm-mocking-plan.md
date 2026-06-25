# Real LLM Mocking Plan

## Goal

Reduce paid LLM provider usage in e2e tests by moving behavioral coverage to deterministic mocked provider responses, while keeping a very small real-provider smoke suite that verifies each provider still works against the live API.

## Current State

The real LLM e2e suite lives under `tests/features/e2e_real_llm*.feature` and contains roughly 140 `@real-llm` scenarios across agent, REPL, UDS, entrypoint matrix, workflow, and tool coverage.

The BDD runner excludes these scenarios unless `QUECTO_REAL_LLM=1` is set. The pre-push path currently supports running this suite through `scripts/run-bdd-shards.sh --real-llm`.

The current real workspace setup in `tests/bdd/e2e_steps.rs` is OpenAI-centric. `Given a real LLM workspace is configured` resolves `OPENAI_API_KEY`, writes an OpenAI provider config, and runs against model `gpt-5.2`.

The repo already has useful no-cost infrastructure:

- BDD e2e tests use WireMock for OpenAI-compatible `/chat/completions` responses.
- Provider tests already mock OpenAI, Anthropic, and Codex HTTP behavior.
- Config supports provider `api_base`, including local mock servers.

## Target Test Split

### 1. Mocked E2E Lane

This should become the default e2e path and should not require provider credentials.

Coverage should include the current behavioral scenarios that exercise application behavior:

- Agent CLI entrypoints.
- REPL entrypoints.
- UDS agent flows.
- Tool use and multi-turn tool loops.
- Session persistence.
- Workflow tool interactions.
- Error recovery.
- Skills behavior.
- Subagent and spawn behavior.

These tests should use local WireMock servers that return deterministic provider-shaped responses.

Provider endpoints to mock:

- OpenAI: `POST /chat/completions`.
- Anthropic: `POST /v1/messages`.
- Codex: `POST /codex/responses`.

Assertions should focus on application behavior, not model reasoning quality. Examples: files were created, tool calls were executed, sessions were persisted, UDS messages were emitted, REPL output was rendered, and expected stdout text appeared.

### 2. Provider Contract Lane

This lane should stay mocked and should verify provider protocol compatibility at the HTTP boundary.

Coverage should include:

- Text responses.
- Tool-call responses.
- Streaming/SSE responses.
- Usage extraction.
- Auth headers.
- Error classification.
- Malformed or partial provider responses.
- Provider-specific fields such as Anthropic beta headers and Codex Responses API fields.

Most of this already exists in provider tests. The plan is to make gaps explicit and avoid duplicating provider contract assertions in expensive live e2e tests.

### 3. Real Provider Smoke Lane

This should be the only paid-provider lane.

Add a new tag such as `@provider-smoke`, separate from `@real-llm`. It should be gated by an explicit environment variable such as `QUECTO_PROVIDER_SMOKE=1`.

Keep this suite intentionally tiny:

- One minimum-token request for OpenAI API key auth.
- One minimum-token request for Anthropic API key auth.
- One minimum-token request for Codex/OpenAI OAuth if CI has appropriate credentials.

Smoke prompt:

```text
Reply exactly OK
```

Request constraints:

- No tools.
- No session persistence.
- Temperature `0` when supported.
- Lowest safe output token limit for the provider.
- Short timeout.
- Minimal model that still validates the intended provider path.

Assertions:

- Exit code is `0`.
- Response is non-empty or contains `OK`.
- Provider-specific request path succeeds.

Do not test tool use, sessions, REPL, UDS, or workflow against live providers.

## Implementation Plan

### Phase 1: Add Shared Mock Provider Fixtures

Create BDD support helpers for provider-shaped responses:

- `Given a mocked OpenAI workspace is configured`.
- `Given a mocked Anthropic workspace is configured`.
- `Given a mocked Codex workspace is configured`.
- `And the mock provider returns text "..."`.
- `And the mock provider returns a tool call for "..." with args:`.
- `And the mock provider returns a tool call sequence:`.
- `And the mock provider streams text "..."`.
- `And the mock provider returns HTTP status ...`.

Generalize the existing OpenAI-only helpers in `tests/bdd/e2e_steps.rs`:

- Keep `rewrite_config_to_uri` or introduce a provider-aware equivalent.
- Reuse the existing OpenAI response builders.
- Add Anthropic response builders for text, tool use, and SSE.
- Add Codex response builders for text, function calls, and SSE.

### Phase 2: Convert Behavioral Real-LLM Scenarios

Move most scenarios in `tests/features/e2e_real_llm*.feature` to mocked e2e features.

Recommended structure:

- `tests/features/e2e_mock_llm_agent_matrix.feature`.
- `tests/features/e2e_mock_llm_repl_matrix.feature`.
- `tests/features/e2e_mock_llm_uds.feature`.
- `tests/features/e2e_mock_llm_workflow.feature`.
- `tests/features/e2e_mock_llm_tools_coverage.feature`.

Replace prompt-dependent real LLM steps with explicit deterministic mock scripts. For example, a real prompt asking the model to create a file should become a mock response sequence that first returns a `write` tool call and then returns final text.

### Phase 3: Add Provider Smoke Feature

Create `tests/features/provider_smoke.feature` with `@provider-smoke` scenarios.

Add BDD steps that configure each live provider from credentials only when the corresponding environment variable is present. Missing credentials should skip or filter the provider-specific scenario, not fail unrelated smoke scenarios.

Suggested environment variables:

- `QUECTO_PROVIDER_SMOKE=1` to enable the lane.
- `OPENAI_API_KEY` for OpenAI.
- `ANTHROPIC_API_KEY` for Anthropic.
- Existing OAuth credential storage for Codex/OpenAI OAuth, if supported in CI.

### Phase 4: Update Runner and Hook Behavior

Update the BDD runner filtering so `@provider-smoke` is independent from `@real-llm`.

Recommended behavior:

- Default BDD run includes mocked e2e and excludes live provider smoke.
- `QUECTO_PROVIDER_SMOKE=1 QUECTO_TAG=provider-smoke` runs only live smoke scenarios.
- Existing `QUECTO_REAL_LLM` can be deprecated or reserved for manual exploratory live suites.

Update scripts and docs:

- `scripts/pre-push.sh` should run no-cost mocked e2e by default.
- Live provider smoke should be opt-in locally and secret-backed in CI.
- README test instructions should describe the new split.

### Phase 5: Retire or Reclassify the Old Real Suite

After mocked equivalents exist, remove `@real-llm` from behavioral coverage or move those scenarios behind a manual-only tag.

The goal is to avoid paying for full behavioral e2e on every push. Live provider calls should answer only one question: can this provider still accept a minimal request and return a valid response?

## Implemented: duplicated (not converted) mocked e2e lane (issue #791)

Rather than convert the live suite in place, the `@real-llm` suite is kept
exactly as-is (`tests/features/e2e_real_llm*.feature`) for occasional on-demand
runs, and a parallel **zero-cost mocked copy** drives the default gate:

- **Structure — consolidated, not 1:1.** The mocked copy is a single
  representative feature, `tests/features/e2e_mock_llm.feature` (plus the
  pre-existing `e2e_mock_llm_agent_matrix.feature`), tagged `@mock-llm`. It is
  deliberately NOT a file-per-file mirror of every `e2e_real_llm*.feature`:
  PR #780's WireMock helpers (`configure_mock_provider_workspace`, the mock
  provider/tool-call response steps) let one deterministic feature reproduce the
  behaviours that many prompt-dependent `@real-llm` scenarios exercise — plain
  text/token responses, single and chained tool-call loops (write/read/edit/
  bash), multi-tool tasks, tool-error recovery, system-prompt influence, and
  session-memory persistence across turns. Coverage parity is enforced
  behaviourally by the `@architecture` guard
  (`tests/features/architecture.feature` →
  `then_mock_e2e_preserves_coverage`), which asserts the `@mock-llm` features
  collectively cover each of those capabilities rather than checking filename
  symmetry.
- **Default lane is free.** `scripts/pre-push.sh` step 9 runs
  `run-bdd-shards.sh --suite mock-llm-bdd --tag mock-llm` (no `--real-llm`, no
  `.env` sourced), so a normal push makes zero paid provider calls and passes
  with no API key.
- **No key auto-trigger.** The old `REAL_LLM_STATE` key-probe was removed; a
  provider key in `.env` no longer auto-enables the paid suite.
- **Documented opt-in.** The live `@real-llm` suite still runs on demand via
  `QUECTO_RUN_REAL_LLM=1 git push` (or the `run-bdd-shards.sh ... --real-llm`
  command in the README), so occasional real-provider validation stays a
  one-liner.

## Implemented: provider smoke lane

The live provider smoke lane is now separate from `@real-llm` and remains
explicitly opt-in:

- `tests/features/provider_smoke.feature` carries `@provider-smoke` scenarios
  for OpenAI API, Anthropic API, and Codex/OpenAI OAuth.
- The BDD runner excludes `@provider-smoke` unless `QUECTO_PROVIDER_SMOKE=1` is
  set, independently from `QUECTO_REAL_LLM`.
- Provider-specific tags filter missing credentials before execution:
  `@provider-smoke-openai` checks `OPENAI_API_KEY`,
  `@provider-smoke-anthropic` checks `ANTHROPIC_API_KEY`, and
  `@provider-smoke-codex` checks for an existing OpenAI OAuth credential in the
  `quecto` credential store.
- Smoke workspaces are temporary and configured with minimal output settings,
  one agent iteration, ephemeral sessions, and short wall-clock timeouts. Codex
  copies the OpenAI OAuth credential into the temporary smoke base directory so
  the user's real config is not modified.
- CI and README test instructions document the provider split. CI runs the lane
  only when at least one API-key provider secret is configured; within the lane,
  unavailable providers are filtered rather than failing unrelated scenarios.

## Acceptance Criteria

- Default test and pre-push paths do not require LLM provider credentials.
- Behavioral e2e coverage remains equivalent or better than the current real suite.
- OpenAI, Anthropic, and Codex provider protocols are covered with mocked HTTP fixtures.
- The live smoke suite performs at most one minimum-token request per provider.
- Live smoke tests are explicitly gated and documented.
- Failures clearly distinguish application behavior regressions from provider availability or auth failures.

## Risks and Mitigations

Risk: Mocked responses may drift from provider APIs.

Mitigation: Keep provider contract tests close to provider modules, use provider-shaped fixtures, and keep live smoke tests for API availability.

Risk: Mocked e2e may stop validating model-driven planning behavior.

Mitigation: Treat model planning as outside deterministic CI scope. Validate the app loop, tool execution, protocol handling, and persistence deterministically.

Risk: Too many provider-specific BDD helpers could duplicate provider unit tests.

Mitigation: Keep BDD helpers focused on application flows. Put protocol edge cases in provider tests.

Risk: Live smoke still flakes due to provider outages.

Mitigation: Keep the smoke lane separate from no-cost CI. In required CI, consider retrying once and reporting provider/auth failures distinctly.

## Suggested First PR

The first PR should be small:

1. Add provider-aware mock workspace helpers for OpenAI and Anthropic.
2. Convert a small representative subset of current `@real-llm` scenarios to mocked equivalents.
3. Add `provider_smoke.feature` with one OpenAI smoke scenario.
4. Update README test commands to document mocked e2e versus live smoke.

Follow-up PRs can convert the full real suite and add Anthropic/Codex smoke coverage.
