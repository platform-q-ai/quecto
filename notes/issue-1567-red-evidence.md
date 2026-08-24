# Issue #1567 full AC RED/falsifiability evidence

New RED tests added before implementation:

## Shared cache-hit ratio
Command:
```sh
cargo test -p quecto-agentic-harness usage_totals_cache_hit_ratio -- --nocapture
```
Result: RED (`exit code 101`). Expected failure: no shared cache-hit ratio helper existed. Evidence: `/tmp/full_red_tests.txt`.

## get_session_stats cost/ratio protocol shape
Command:
```sh
cargo test -p quecto-agentic-harness get_session_stats_tokens_camel_case -- --nocapture
```
Result: RED (`exit code 101`). Expected failure: `SessionStats` had no `cost_micro_usd` or `cache_hit_ratio` fields. Evidence: `/tmp/full_red_tests.txt`.

## TUI typed session stats preserves cache/cost/ratio
Command:
```sh
cargo test -p quecto-tui parse_session_stats -- --nocapture
```
Result: RED (`exit code 101`). Expected failure: TUI `SessionStats` lacked cache, total, `cost_micro_usd`, and `cache_hit_ratio` fields. Evidence: `/tmp/full_red_tui.txt`.

## Review-added surface assertions
Added after local review and verified with targeted GREEN runs:
- `record_agent_result_emits_normalized_session_usage_log` parses the JSON structured event and asserts exact shared usage fields.
- `record_agent_result_without_usage_does_not_emit_session_usage_log` prevents misleading zero/no-usage logs.
- `query_get_session_stats_returns_shared_usage_accounting_fields` proves query dispatch returns shared usage stats, not only protocol serialization.
- TUI `/session` and footer/status tests assert shared stats cache/cost/ratio rendering.

Falsifiability rationale for review-added assertions:
- Removing/gating the structured log event, renaming a structured field, or changing ratio denominator fails the parsed log test.
- Returning default usage from `GetSessionStats` dispatch fails the query fixture test.
- Dropping cache/cost/ratio from TUI typed parsing or footer rendering fails the TUI surface tests.

No temporary mutation residue introduced.
