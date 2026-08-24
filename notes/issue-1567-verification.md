# Issue #1567 targeted verification

Step 10 GREEN evidence.

Tooling setup/workarounds:
- Initial `cargo fmt --all -- --check` failed because `rustfmt` was not installed for toolchain `1.97.1`; installed with `rustup component add rustfmt`.
- Initial clippy failed because `clippy` was not installed; installed with `rustup component add clippy`.

Formatting/lint:
- `cargo fmt --all` applied required formatting in `usage.rs`.
- `cargo fmt --all -- --check` passed.
- `cargo clippy -p quecto-agentic-harness --all-targets -- -D warnings` passed.

Targeted tests passed:
- `cargo test -p quecto-agentic-harness usage::tests -- --nocapture`
- `cargo test -p quecto-agentic-harness issue_996_efficiency_tests -- --nocapture`
- `cargo test -p quecto-agentic-harness test_chat_text_response -- --nocapture`
- `cargo test -p quecto-agentic-harness test_parse_response_with_usage -- --nocapture`
- `cargo test -p quecto-agentic-harness handler_captures_usage_chunk_into_response -- --nocapture`
- `cargo test -p quecto-agentic-harness parse_sse_response_extracts_usage_chunk -- --nocapture`
- `cargo test -p quecto-agentic-harness test_parse_sse_text_response -- --nocapture`
- `cargo test -p quecto-agentic-harness handler_text_delta_and_response_completed_usage -- --nocapture`
- `cargo test -p quecto-agentic-harness protocol_shape_tests -- --nocapture`
- `cargo test -p quecto-agentic-harness test_session_stats_serializes -- --nocapture`
- `cargo test -p quecto-agentic-harness test_session_stats_deserializes_without_cost -- --nocapture`
- `cargo test -p quecto-agentic-harness record_falls_back_to_prompt_tokens_when_context_tokens_absent -- --nocapture`

Architecture/contract checks:
- `rg 'cache_read_input_tokens|cache_creation_input_tokens|prompt_tokens_details|input_tokens_details' quecto-agentic-harness/src/domain quecto-agentic-harness/src/application` returned no matches.
- Same provider-wire-name grep over production interface code returned no matches.
- Provider parser/test wire-name grep found expected matches under `quecto-agentic-harness/src/infrastructure/providers`.

Version/docs sync (step 11):
- Bumped `quecto-agentic-harness` from `0.105.25` to `0.105.26` in `quecto-agentic-harness/Cargo.toml` and `Cargo.lock`.
- Updated workspace and harness README current-version lines to `0.105.26`.
- `cargo test -p quecto-agentic-harness --test repo_docs -- --nocapture` passed.
- Re-ran `cargo fmt --all -- --check` and `cargo clippy -p quecto-agentic-harness --all-targets -- -D warnings`; both passed.

AC/semantic coverage:
- OpenAI Chat cached-token parsing: covered by `usage::tests`, OpenAI non-streaming, and OpenAI SSE targeted tests.
- OpenAI Responses/Codex cached-token parsing: covered by `usage::tests`, Codex response and SSE targeted tests.
- Absent/zero/malformed/overflow/clamped cached-token boundaries: covered by `usage::tests`.
- Protocol `tokens.total = input + output`: covered by protocol fixture tests.
- Clean Architecture ownership: covered by grep checks and reviewer clean verdicts.
