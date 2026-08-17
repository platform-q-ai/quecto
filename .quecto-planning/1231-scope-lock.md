# Issue 1231 scope lock

Implement all six phases from the published execution plan for #1231 in this PR.

## Covered phases / ACs
- Phase 1: contract and regression characterization for additive thinking events, recovered message shape, old-client compatibility, answer-only non-interactive output, and stable token/tool/turn/agent-end/spinner behaviour. Covers AC5, AC11, AC13.
- Phase 2: provider normalization for Anthropic thinking/redacted placeholders, OpenAI Responses reasoning summaries, and fixture-backed OpenAI-compatible reasoning fields without leaking private payloads. Covers AC2, AC3, AC4, AC14.
- Phase 3: application progress and additive UDS/API/TUI protocol events. Covers AC1, AC5, AC13.
- Phase 4: persistence and message recovery for visible thinking across text-only/tool turns and reload/recovery. Covers AC6, AC7.
- Phase 5: TUI live/recovered rendering plus display-only remembered hide/show preference. Covers AC8, AC9, AC10, AC12.
- Phase 6: docs and regression verification. Covers AC15, AC16 and final cross-checks.

## Non-goals
No effort selector redesign; no model-aware effort vocabulary; no Pi-style thinking-level system; no Shift+Tab cycle; no mandatory --thinking flag; no RPC thinking-level commands; no summarizing/rewording provider thinking; no user-visible encrypted/signature/redacted provider internals; no live-provider integration tests; no #1230 fix beyond emitted thinking visibility.

## Expected touched surfaces
- Provider adapters/parsers for Anthropic, OpenAI Responses, and explicit OpenAI-compatible fixture-backed fields.
- Domain/application progress model and agent loop forwarding.
- UDS/API protocol DTOs and message recovery views.
- Session persistence/reload paths for display-safe thinking.
- TUI transcript rendering, keybinding/settings preference storage, routing tests.
- Protocol/user docs and capability matrix.

## Architecture constraints
Provider adapters parse provider formats; domain/application normalize and emit display-safe progress; persistence owns session data including private replay fields; protocol exposes additive DTOs; TUI renders and stores display preference only. Token, tool, turn, agent_end, effort, and pre-call spinner semantics must remain stable.

## Deferred work
None of the six published phases are deferred. Non-goals remain out of scope.

## Verification evidence to produce
Targeted provider fixture tests, protocol/API tests, persistence/recovery reload tests, TUI render/preference tests, non-interactive answer-only regression, docs updates, and relevant workspace checks.
