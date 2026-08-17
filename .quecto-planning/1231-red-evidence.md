# Issue 1231 RED / falsifiability evidence

Targeted behavioural assertions introduced before/followed by minimal implementation:

- `domain::thinking_visibility_tests::message_view_exposes_display_safe_thinking_without_private_replay_fields`
  - RED: failed with `left: Null right: "visible reasoning"` because recovered message JSON had no `visibleThinking` field.
  - GREEN after additive display-safe `visibleThinking` serializer; asserts signatures/redacted blobs are absent.
- `application::agent_loop::tests::thinking_delta_emits_distinct_progress_before_answer_token`
  - RED compile: no `AgentProgressEvent::ThinkingDelta` variant; after variant, RED runtime: no forwarded thinking progress event.
  - GREEN after forwarding `StreamEvent::ThinkingDelta` to distinct app progress while leaving token handling unchanged.
- `interface::cli::uds::tests::progress_clear_tests::test_forward_progress_event_emits_thinking_distinct_from_token`
  - RED runtime: no output line for thinking progress (`Option::unwrap()` on missing line), proving additive UDS event assertion was not hollow.
  - GREEN after adding additive `AgentEvent::Thinking { text }` and forwarding distinct from `Token { token }`.

Characterization/falsifiability notes:
- Existing token/tool/turn/agent_end/pre-call spinner tests remained targeted and untouched while adding exhaustive match arms; thinking deltas are ignored by REPL spinner to preserve non-interactive/spinner semantics.
- No temporary mutation residue remains; all RED proofs came from actual missing behaviour before the corresponding implementation.
