# Issue #1567 full AC verification

Step 10 GREEN evidence for the continued full-AC slice.

## Formatting and lint

Passed:

```sh
cargo fmt --all -- --check
cargo clippy -p quecto-agentic-harness --all-targets -- -D warnings
cargo clippy -p quecto-tui --all-targets -- -D warnings
```

Evidence: `/tmp/step10_fmt_clippy_fullac.txt`.

## Targeted tests

Passed targeted harness tests:

```sh
cargo test -p quecto-agentic-harness usage_accounting -- --nocapture
cargo test -p quecto-agentic-harness test_model_pricing_known_models -- --nocapture
cargo test -p quecto-agentic-harness usage::tests -- --nocapture
cargo test -p quecto-agentic-harness test_anthropic_cache_usage_matches_normalized_openai_equivalent -- --nocapture
cargo test -p quecto-agentic-harness dispatch_message_stop_attaches_shared_cost_when_model_present -- --nocapture
cargo test -p quecto-agentic-harness handler_captures_usage_chunk_into_response -- --nocapture
cargo test -p quecto-agentic-harness handler_emits_text_delta_then_done_on_completed -- --nocapture
cargo test -p quecto-agentic-harness test_chat_text_response -- --nocapture
cargo test -p quecto-agentic-harness test_codex_provider_success -- --nocapture
cargo test -p quecto-agentic-harness record_agent_result_emits_normalized_session_usage_log -- --nocapture
cargo test -p quecto-agentic-harness record_agent_result_without_usage_does_not_emit_session_usage_log -- --nocapture
cargo test -p quecto-agentic-harness query_get_session_stats_returns_shared_usage_accounting_fields -- --nocapture
cargo test -p quecto-agentic-harness test_session_stats_include_usage_totals -- --nocapture
cargo test -p quecto-agentic-harness get_session_stats_tokens_camel_case -- --nocapture
cargo test -p quecto-agentic-harness --test repo_docs -- --nocapture
```

Passed targeted TUI tests:

```sh
cargo test -p quecto-tui parse_session_stats -- --nocapture
cargo test -p quecto-tui show_session_stats_with_context_updates_footer_flag -- --nocapture
cargo test -p quecto-tui update_footer_stats_sets_context_and_clears_zero_cost -- --nocapture
cargo test -p quecto-tui update_footer_stats_consumes_positive_cost_without_context -- --nocapture
cargo test -p quecto-tui reset_session_clears_chat_and_notifies -- --nocapture
```

Evidence: `/tmp/step10_targeted_fullac.txt` and `/tmp/step9_cost_precedence_tests.txt`.

## Architecture / contract / docs checks

Passed:

```sh
rg 'prompt_tokens_details|input_tokens_details|cache_read_input_tokens|cache_creation_input_tokens' \
  quecto-agentic-harness/src/domain \
  quecto-agentic-harness/src/application \
  quecto-agentic-harness/src/interface \
  quecto-tui/src \
  --glob '!**/*tests.rs'

rg 'fn attach_cost|Self::attach_cost|model_pricing\(' \
  quecto-agentic-harness/src/infrastructure/providers \
  --glob '!**/*tests.rs'

rg 'costMicroUsd|cacheHitRatio|get_session_stats' \
  quecto-agentic-harness/README.md \
  quecto-agentic-harness/docs/uds-protocol.md -n
```

Results:
- Provider-native usage wire names are absent from domain/application/interface/TUI production code.
- Provider-private cost attach remnants are absent from provider production code; shared `domain::usage_accounting::attach_cost` is used.
- README and UDS protocol docs document `costMicroUsd`, `cacheHitRatio`, and normalized `get_session_stats` semantics.

Evidence: `/tmp/step10_arch_docs_checks.txt`.

## AC / matrix coverage

- Shared cache-hit ratio: `usage_accounting` tests and query/log/TUI consumers.
- Shared cost accounting: model pricing tests, `usage_accounting::attach_cost`, OpenAI/Codex/Anthropic non-streaming and streaming cost tests.
- `get_session_stats`: protocol shape tests and `query_get_session_stats_returns_shared_usage_accounting_fields`.
- Structured logs: `record_agent_result_emits_normalized_session_usage_log` plus no-usage negative test.
- TUI `/session`/footer/status: `parse_session_stats`, `/session` response rendering, footer cost/cache/ratio, reset clearing stale stats, and cost precedence tests.
- Docs: `repo_docs` plus direct docs grep.
