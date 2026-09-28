# Claude Code stream-json fixtures

- `rt`, `guards`, `mid` and `kill` (`*.stream.jsonl`) are real captures from spike #2264. They were recorded with `claude` 2.1.280 and haiku, then trimmed. Session ids, uuids and operator paths are replaced by placeholders; every field is kept.
- `errors.stream.jsonl` is **synthesized**, not captured. Its result lines (`error_max_turns`, `error_max_budget_usd`, `error_during_execution`) follow the result schema in the `claude` 2.1.280 binary: `errors[]`, no `result`, `is_error: true`.
- `not_logged_in.stream.jsonl` is **synthesized** (#2286): a not-logged-in turn (the assistant `error: authentication_failed`, a `result` with `subtype: success` yet `is_error: true`, `terminal_reason: api_error`, 401), then a turn that completes over the same process. The mock `claude` replays it to prove an API error does not end the process.
