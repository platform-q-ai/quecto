# Issue #1567 full AC test design

Trace: `notes/issue-1567-semantic-matrix.md`.

Contracts locked from review:
- Cost wire field is `costMicroUsd: u64`; missing/unknown pricing serializes `0` for backward compatibility but must not hide token/cache stats.
- TUI parser precedence: `costMicroUsd` wins; legacy `cost` float is converted to micros only when `costMicroUsd` is absent; if both missing => 0. Round legacy dollars to nearest micro.
- `cacheHitRatio` is `Option<f64>` serialized as `null` when no denominator exists; tests compare with tolerance `1e-9`.
- Structured-log AC is satisfied by at least one structured session-usage event with shared stats fields matching `get_session_stats`; no-usage must not emit misleading nonzero fields.
- Architecture checks avoid brittle code layout assertions; use forbidden wire-field grep plus black-box cross-provider/shared-stats equivalence.

## Application/shared accounting tests
Concrete tests:
- `usage_totals_cache_hit_ratio_none_without_denominator`: no tokens => `None`/JSON null.
- `usage_totals_cache_hit_ratio_mixed_input_and_read`: input=70, read=30, write=0 => 0.30.
- `usage_totals_cache_hit_ratio_write_only_zero_hit`: input=0, read=0, write=50 => 0.0.
- `usage_totals_cache_hit_ratio_read_only_full_hit`: input=0, read=30, write=0 => 1.0.
- `usage_totals_cache_hit_ratio_read_and_write`: input=0, read=30, write=20 => 30/50.
- `usage_totals_cache_hit_ratio_uncached_input_zero_hit`: input=70, read=0, write=0 => 0.0.
- `session_stats_from_shared_usage_fixture_reports_cost_and_ratio`: AgentResult/shared fixture input=70, output=20, read=30, write=5, context=105, cost_micro_usd=1234 => shared stats include tokens, context, `costMicroUsd=1234`, `cacheHitRatio=30/105`.
- `equivalent_provider_usage_produces_identical_shared_stats`: normalized Anthropic/OpenAI/Codex-equivalent fixtures produce identical stats/log-surface DTO values.

## get_session_stats protocol/API tests
Concrete tests:
- `session_stats_serializes_cost_micro_usd_and_cache_hit_ratio`: JSON has `costMicroUsd`, `cacheHitRatio`, normalized token/cache/context fields; `tokens.total == input + output`.
- `session_stats_serializes_null_ratio_without_denominator`: no tokens => `cacheHitRatio: null` and `costMicroUsd: 0`.
- `session_stats_deserializes_legacy_missing_cost_ratio`: old payload missing cost/ratio defaults safely.
- `query_get_session_stats_uses_shared_accounting_fixture`: query/dispatch returns exact shared fixture values; no provider-native fields.

## Structured log tests
Concrete tests:
- `record_agent_result_emits_normalized_session_usage_log`: capture tracing during result/session recording; assert at least one structured event contains exact shared fixture fields (`input`, `output`, `cacheRead`, `cacheWrite`, `context`, `costMicroUsd`, `cacheHitRatio`) matching `get_session_stats`.
- `record_agent_result_without_usage_does_not_emit_misleading_usage_log`: no-usage path has no event with nonzero/made-up stats.

## TUI protocol/session tests
Concrete tests:
- `parse_session_stats_preserves_cache_cost_ratio`: TUI typed `SessionStats` parses tokens.input/output/cacheRead/cacheWrite/total, context, `costMicroUsd`, `cacheHitRatio`.
- `parse_session_stats_cost_micro_usd_precedes_legacy_cost`: both present => micros wins.
- `parse_session_stats_converts_legacy_cost_when_micros_absent`: legacy `cost: 0.001234` => `cost_micro_usd=1234`.
- `parse_session_stats_missing_cost_ratio_defaults`: missing cost/ratio => 0/None.

## TUI surface tests
Use one shared JSON fixture for all TUI tests: input=70, output=20, cacheRead=30, cacheWrite=5, total=90, context=105, costMicroUsd=1234, cacheHitRatio=30/105.
- `/session` response renders normalized input/output/cache/cost/ratio/context from typed session stats.
- Footer/status/detail update from the same parsed typed session stats fixture and show consistent values.
- Tests inject only `get_session_stats` DTO; no provider wire payload or provider branch.

## Architecture/docs checks
- Grep provider-native wire fields outside infrastructure providers/tests/docs/notes: none.
- Grep TUI/harness production for provider-native wire fields: none.
- Developer note documents new provider path: native usage -> normalized UsageInfo -> shared totals/stats/log/TUI.

## Acceptance coverage matrix to record before completion
- Shared cache-hit ratio: application tests above.
- `get_session_stats` cost/cache/ratio: protocol/query tests above.
- Structured logs: log tests above.
- TUI `/session`: render test above.
- TUI status/detail shared stats: typed DTO + render tests above.
- Parser/provider ACs from PR #1568 retained by existing parser/SSE/BDD tests.
