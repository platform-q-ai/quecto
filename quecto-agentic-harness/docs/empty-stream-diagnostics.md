# Empty-stream diagnostics

An assistant stream that terminates without text, tools or thinking is classified
`empty_stream`, not HTTP 503. This synthetic retryable classification retains the
existing retry budget and no-replay guard. `max_tokens` remains the existing
non-retryable output-limit path. Neither classification establishes overload.

Request observations add `started_unix_ms`, `finished_unix_ms` and bounded
`attempt_diagnostics`. Existing audit/session observation and swarm `request_usage`
JSON payloads carry these fields; no logging/configuration switch is changed.
Older observations deserialize with unavailable timestamps and no attempt records.
A request ID identifies the logical request; each nested attempt has its original
attempt number. UTC values are Unix milliseconds, not monotonic clocks; elapsed
milliseconds use a monotonic clock. Clock adjustments can reorder UTC values.

Transport details are available only for the existing admission-owned OpenAI,
Codex and Anthropic adapter paths with a request trace. Disabled admission,
unsupported adapters, admission refusal before dispatch and legacy observations
have no wire telemetry. Absence is unavailable evidence, not success or overload.

Privacy boundary: only status, typed terminal/error/incomplete enums, counters,
timestamps, termination and output-presence booleans are retained. Request IDs in
allowlisted headers are SHA-256 digests (never raw values). Retry/rate headers
accept only bounded unsigned integers; dates, compound durations, malformed and
unknown headers are omitted. Authorization, cookies, URLs, account IDs, provider
messages, bodies, prompt/output, tool arguments and arbitrary unknown event values
are never copied into these added records. Existing provider-facing error strings
are not a sanitized export surface and are unchanged by this diagnostic slice.

Retention is a bounded prefix of 16 transport records per logical request and the
existing 64 recent request observations. This is not a new durable wire-event log,
a full admission timeline, nor a reconstruction of prior failures. No provider
traffic/retry policy or runtime settings are changed to obtain diagnostics.

Terminal stream events are delivered after the owned transport is destroyed and
its diagnostic snapshot is published, so application observations cannot race
normal completion. Cancellation of the whole logical request may still snapshot
before a spawned adapter has observed receiver closure; absence/incompleteness in
that case must not be interpreted as an HTTP or overload signal. Attempt start
currently means permit acquired, not admission queue entry or socket send time.

The `empty_stream` error class is additive: consumers that exhaustively deserialize
error classes must be updated before reading new records. Stop reasons are mapped
to a closed enum; unknown provider reasons retain only `Unknown`, never their text.
