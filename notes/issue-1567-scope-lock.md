# Issue #1567 scope lock — full AC completion

Phase: continue PR #1568 from parser/protocol slice to complete all remaining issue #1567 acceptance criteria.

Covered in this run:
- Shared cache-hit ratio/efficiency calculation outside provider adapters.
- Normalized token/cache/cost stats exposed through `get_session_stats`.
- Normalized token/cache/cost stats emitted in structured logs.
- TUI `/session` displays normalized token/cache/cost stats.
- TUI status/detail surfaces consume the same shared normalized session stats, not provider-specific fields.
- Acceptance coverage proves selected surfaces use the shared accounting path.
- PR #1568 body/notes updated so no #1567 ACs are deferred.

Already covered by previous slice and preserved:
- OpenAI Chat `prompt_tokens_details.cached_tokens` parsing.
- OpenAI Responses/Codex `input_tokens_details.cached_tokens` parsing.
- Normalized `UsageInfo` mapping for Anthropic/OpenAI/Codex.
- Provider wire parsing remains infrastructure-local.

Non-goals:
- No provider request behavior changes except usage metadata already returned.
- No billing dashboard.
- No provider wire JSON moved into domain/application.

Expected touched surfaces:
- `quecto-agentic-harness/src/application/agent_usage.rs` and stats/session aggregation helpers.
- `quecto-agentic-harness/src/interface/cli/protocol.rs`, command handling, logs.
- TUI session/status/detail handling/rendering paths.
- Acceptance/unit tests for stats, logs, TUI/session/status, architecture/docs.
- PR/issue notes and developer docs.

Verification evidence planned:
- RED tests for shared cache-hit stats, `get_session_stats`, structured logs, TUI `/session`/status shared DTO consumption.
- GREEN targeted tests plus fmt/clippy/push gate.
