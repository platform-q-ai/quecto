# Issue #1567 semantic state-space matrix

Frozen contract for implemented parser/protocol slice:
- `UsageInfo.prompt_tokens` is normalized **full-price/non-cache billable input**. For OpenAI Chat/Responses where provider input includes cached tokens as a subset, adapters clamp cached input to provider prompt/input, subtract it for `prompt_tokens`, and set `context_tokens` to provider full prompt/input count.
- `cache_read_tokens: Some(0)` preserves provider-reported zero; absent/malformed optional cache detail remains `None` and must not discard base usage.
- `context_input_tokens()` is latest-call context occupancy.
- `tokens.total` means full-price input + output; cache/context buckets are separate.

Out of scope for this slice: new cache-hit ratio/session aggregation behaviour, structured log/TUI end-to-end changes, new Anthropic wire-shape support, and standalone developer documentation beyond issue notes.

| Invariant | Dimensions/equivalence classes | Representative cases | Expected observable outcome | Evidence |
|---|---|---|---|---|
| OpenAI Chat cached-token parsing | details absent / present nonzero / present zero / malformed / overflow / cached > prompt | prompt=100 cached=30 completion=20; zero cached; malformed string; absent details; prompt=10 cached=30 | nonzero => `prompt_tokens=70`, `cache_read_tokens=Some(30)`, `context_tokens=Some(100)`; zero => `Some(0)` and prompt unchanged; absent/malformed/overflow => cache `None`, base prompt/completion/context preserved; cached > prompt clamps read to prompt and billable prompt to 0 | provider parser tests; non-streaming and SSE tests |
| OpenAI Responses/Codex cached-token parsing | details absent / present nonzero / present zero / malformed / overflow / cached > input | input=100 cached=30 output=20; zero cached; absent details; input=10 cached=30 | same normalized semantics as Chat, using provider input as context | provider parser tests; non-streaming and SSE tests |
| Backward compatibility | no usage; prompt/completion only; input/output only | existing Chat/Codex payloads without cache detail | base usage preserved, no panic, cache remains `None`, context set from provider prompt/input where parser has it | regression tests |
| Clean architecture ownership | provider wire JSON names allowed only in provider parsers/tests and issue notes | grep/review for provider-native fields outside infrastructure/tests/notes | domain/application/interface code describes normalized fields only | local architecture review |
| Protocol total semantics | stats fixtures with cache buckets | input=50k, output=10k, cache_read=40k, cache_write=5k | `tokens.total == input + output`, not cache-inclusive; cache/context fields remain separate | protocol fixture tests |
| Streaming final usage | OpenAI/Codex final usage chunks with cached detail | final SSE usage includes cached tokens | resulting single `UsageInfo` is normalized consistently with non-streaming parser | targeted SSE tests |
