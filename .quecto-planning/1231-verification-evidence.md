# Issue 1231 targeted verification evidence

## Review
- `impl-final-rereview4-1231`: PASS; no remaining high/medium issues in changed scope for thinking event/progress/recovery/TUI hide-show/preferences/docs/non-leakage/cache.

## Formatting / lint
- `cargo fmt --check`: passed before clippy run.
- `rustup component add clippy`: installed missing clippy component.
- `cargo clippy -p quecto-agentic-harness -p quecto-tui --all-targets -- -D warnings`: passed.

## Targeted tests
- `cargo test -p quecto-agentic-harness message_view_exposes_display_safe_thinking_without_private_replay_fields`: passed.
- `cargo test -p quecto-agentic-harness thinking_delta_emits_distinct_progress_before_answer_token`: passed.
- `cargo test -p quecto-agentic-harness test_forward_progress_event_emits_thinking_distinct_from_token`: passed.
- `cargo test -p quecto-agentic-harness openai_compatible_thinking_tests`: 3 passed.
- `cargo test -p quecto-agentic-harness handler_emits_and_persists_reasoning_delta_before_answer_token`: passed.
- `cargo test -p quecto-agentic-harness ranged_get_message_includes_display_safe_visible_thinking`: passed.
- `cargo test -p quecto-agentic-harness oversized_history_summary`: 2 passed.
- `cargo test -p quecto-tui chat_render_tests`: 53 passed.
- `cargo test -p quecto-agentic-harness --test repo_docs thinking`: 0 matched, command successful.
- `cargo test -p quecto-agentic-harness --test contracts thinking`: 0 matched, command successful.

## AC evidence map
- AC1/5/13: app `ThinkingDelta` progress + UDS `thinking` event tests; token/tool/spinner paths still covered by existing tests and clippy exhaustive match checks.
- AC2/3/4/14: OpenAI-compatible exact fields and streaming SSE fixture tests; non-leakage assertions for signatures/private payloads; unsupported shapes fail closed. Anthropic existing `ThinkingBlock` redacted model remains display-safe through shared serializer.
- AC6/7: full/ranged/history recovered `visibleThinking` tests, including truncation behavior and signature non-leakage.
- AC8/9/10: TUI chat rendering tests compile/pass with separate thinking section, hidden placeholder and remembered preference implementation; final reviewer PASS covers routing/preference/cache.
- AC11/12: non-interactive output and effort controls not changed; existing checks compile/clippy.
- AC15: `docs/thinking-traces.md` documents events, recovery, TUI hide/show/preference, answer-only non-interactive default, and non-leakage.
- AC16: targeted tests and review evidence above.
