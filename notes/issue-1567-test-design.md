# Issue #1567 test design

Trace: semantic rows in `notes/issue-1567-semantic-matrix.md`.

Implemented RED/GREEN slice: provider usage normalization plus directly affected protocol fixtures. Broader end-to-end session/log/TUI assertions are deferred unless implementation changes those paths.

## Provider parser unit tests
`quecto-agentic-harness/src/infrastructure/providers/usage_tests.rs`:
- OpenAI Chat positive: `prompt_tokens=100`, `prompt_tokens_details.cached_tokens=30`, `completion_tokens=20` => normalized full-price `prompt_tokens=70`, `cache_read_tokens=Some(30)`, `context_tokens=Some(100)`, completion unchanged.
- OpenAI Chat zero: `cached_tokens=0` => `cache_read_tokens=Some(0)`, prompt unchanged, context full prompt.
- OpenAI Chat malformed/overflow/absent: cache detail ignored as `None`; prompt/completion still parsed; no panic.
- OpenAI Chat clamping boundary: `prompt_tokens=10`, `cached_tokens=30` => `prompt_tokens=0`, `cache_read_tokens=Some(10)`, `context_tokens=Some(10)`.
- Responses/Codex equivalent positive: `input_tokens=100`, `input_tokens_details.cached_tokens=30`, `output_tokens=20` => full-price input=70, read=30, context=100.
- Responses/Codex zero, malformed, overflow, absent, and clamping boundaries mirror Chat compatibility expectations.

## Streaming/SSE provider tests
- OpenAI SSE and Codex SSE final usage chunks containing cached-token details are retained and normalized in resulting `UsageInfo`.
- Absent cached-token details remain unreported where covered by compatibility tests.

## Existing regression/compatibility tests touched
- Existing OpenAI/Codex tests that asserted old context or cache-inclusive totals are updated to the normalized contract.
- Protocol fixture tests assert `tokens.total == input + output`, with cache/context carried separately.
- Existing fallback aggregation test remains a compatibility check for providers without explicit context occupancy.

## Deferred test areas
- New Anthropic behaviour beyond existing accepted wire fields.
- Domain/application cost/cache-hit/multi-call helpers beyond effects of normalized provider inputs.
- End-to-end structured log, `TurnEnd`, real session recording, API, and TUI rendering tests.
- Architecture grep automation and developer docs beyond this issue notes.

No incidental assertions: tests assert observable normalized fields and protocol values rather than private implementation paths beyond provider parser unit boundaries.
