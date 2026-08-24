# Issue #1567 RED/falsifiability evidence

Targeted new parser assertions were added to `quecto-agentic-harness/src/infrastructure/providers/usage_tests.rs` and run before implementation.

Command:

```sh
cargo test -p quecto-agentic-harness cached_tokens -- --nocapture
```

Result: RED as expected (`exit code 101`). Failure summary is recorded below.

Expected failures observed:
- OpenAI Chat cached subset not normalized: `prompt_tokens` remained provider full prompt (`100`) instead of normalized full-price input (`70`).
- OpenAI Chat reported zero cache not preserved: `cache_read_tokens` was `None` instead of `Some(0)`.
- OpenAI Chat malformed cache detail preserved base usage but did not set explicit context from provider prompt (`None` vs `Some(100)`).
- OpenAI Chat cached > prompt did not clamp/subtract (`10` vs `0`).
- Responses/Codex cached subset not normalized: `prompt_tokens` remained provider full input (`100`) instead of `70`.
- Responses/Codex zero/malformed cases did not set provider-context `Some(100)`.
- Responses/Codex cached > input did not clamp/subtract (`10` vs `0`).

These failures prove the new behavioral parser assertions are not hollow and fail for the expected pre-implementation reasons.

A first attempted test filter used module path `infrastructure::providers::usage_tests` and matched zero tests; this was corrected by using the targeted substring filter `cached_tokens`.

Review-added characterization/falsifiability:
- Added Codex absent-`input_tokens_details` case to lock absent cache metadata as `cache_read_tokens: None` while preserving base input/output/context.
- Added/strengthened OpenAI absent-`prompt_tokens_details` case to lock absent cache metadata as `cache_read_tokens: None` while preserving base prompt/completion/context.
- These are compatibility characterizations that should pass after implementation. They are falsifiable because changing either parser to default missing `*_tokens_details.cached_tokens` to `Some(0)` would fail the absent-cache tests (`None` vs `Some(0)`).
- Verified with targeted absent-cache tests and full `usage::tests` during the local workflow.

No temporary mutation residue was introduced.
