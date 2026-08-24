# Issue #1567 scope lock

Implemented phase: provider usage normalization for OpenAI Chat Completions and OpenAI Responses/Codex cached-token payloads, plus directly affected protocol/test fixtures.

Covered acceptance criteria in this slice:
- OpenAI Chat Completions parses `prompt_tokens_details.cached_tokens` into normalized `UsageInfo`.
- OpenAI Responses/Codex parses `input_tokens_details.cached_tokens` into the same normalized `UsageInfo` semantics.
- Provider wire parsing remains infrastructure-local.
- Normalized `UsageInfo.prompt_tokens` means full-price non-cache input; `context_tokens` carries provider prompt/input occupancy.
- Existing protocol fixtures use normalized `tokens.total = input + output`, with cache buckets separate.
- Tests preserve absent/zero/malformed/overflow/clamped cached-token boundaries and streaming final usage chunk extraction for OpenAI/Codex.

Non-goals for this PR slice:
- No provider request behaviour changes except retaining/parsing usage metadata already returned.
- No billing dashboard.
- No provider wire JSON types moved into domain/application.
- No new end-to-end TUI/log/session aggregation feature work beyond fixtures directly touched by normalized parser outputs.
- No new Anthropic wire-shape support beyond existing behaviour.

Expected touched surfaces/files:
- `quecto-agentic-harness/src/infrastructure/providers/usage.rs` for OpenAI/Codex usage parsing normalization.
- OpenAI/Codex parser/SSE tests and stale compatibility tests using the changed normalized semantics.
- `quecto-agentic-harness/src/domain/message.rs` comments documenting normalized cache/context semantics.
- Protocol fixture tests whose total/cache semantics were stale.
- Issue notes for scope, matrix, test design, and RED/falsifiability evidence.

Architecture ownership:
- Infrastructure adapters parse provider-native JSON into normalized `UsageInfo`.
- Domain/application owns aggregation/cost/cache-hit meaning over normalized usage; this slice avoids moving provider-native fields upward.
- Interface/TUI/API surfaces consume normalized stats DTOs; this slice only corrects stale protocol fixtures, not UI behaviour.

Verification evidence:
- RED evidence for parser cached-token behavior is summarized in `notes/issue-1567-red-evidence.md`.
- Targeted GREEN commands run locally included usage parser tests, OpenAI/Codex SSE tests, issue_996 compatibility tests, and protocol fixture tests.

Deferred work:
- Additional providers beyond currently supported adapters.
- Rich billing dashboard/analytics beyond exposed normalized stats.
- Broader end-to-end log/session/TUI coverage and any changes required by that coverage.
