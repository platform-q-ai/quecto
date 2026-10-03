# Empty-stream diagnostics

An assistant stream that terminates without text, tools or thinking is classified
`empty_stream`, not HTTP 503. This synthetic retryable classification retains the
existing retry budget and no-replay guard. `max_tokens` remains the existing
non-retryable output-limit path. Neither classification establishes overload.

## An empty reply to tool results ends the turn (#2434)

Whether an empty reply is an error depends on what it answers. The latest user
message or tool result in the request decides; system messages and the model's
own messages are not something to answer, so they are ignored:

- **A user message** (a prompt, a steer, a follow-up, a harness note or the
  loop's own feedback) must be answered. An empty reply to it is `empty_stream`,
  retried, then an `agent_error`, as above.
- **Tool results** may be answered with nothing: the model acted and has nothing
  to add. Text of whitespace alone counts as nothing. For example, a background tool may say "started — end your turn now".
  A completed reply with no text, no tool call and no visible thinking (encrypted
  reasoning alone counts as nothing) that stopped at `end_turn` ends the turn at
  once. There is no retry and no error, and the run ends with `agent_end`. It is
  a final answer with no text. No empty assistant message is recorded (some
  providers refuse an empty assistant message when the conversation is sent
  again), so the conversation ends on the tool results, as it does at the tool
  iteration limit.

A reply after tool results may hold visible reasoning (Anthropic thinking, a
Codex summary, OpenAI-compatible `reasoning_content`) and no text. That is not
nothing, so it is recorded as a final answer with no text. No provider is ever
sent empty assistant text:

- Anthropic keeps a signed thinking block without a text part. It leaves out
  an assistant message with nothing else to send.
- OpenAI chat leaves out an assistant entry with no text and no call.
- Codex/Responses sends no empty answer and no reasoning item that leads to
  nothing.

A prompt after a turn that ended with no reply follows the tool results as its
own user turn. On Anthropic that is a second consecutive user turn, which the
API accepts.

Only a clean stop ends the turn. After tool results, an empty reply stopped at
`max_tokens` was cut off, so it keeps the `stop_reason=max_tokens` error.
Reasoning alone at the output limit is still asked again once and then fails
(#2124). An empty reply that names no stop reason is not known to have finished,
so it stays `empty_stream` and is retried. A 200 body with no event at all is
a transport failure, not a reply, and stays `empty_stream` whatever it answers.

The request whose reply ended the turn this way is recorded with outcome
`succeeded` and `"ended_empty_after_tools": true` on its `RequestObservation`
(in `request_observed` audit records and the recent request diagnostics). The
field is absent when false and in older records. The harness also logs
`ended_empty_after_tools` at info level (target `agent_loop`). The rule applies
to the streamed and the whole-reply paths of every provider: Codex/Responses,
OpenAI chat, OpenAI-compatible and Anthropic each deliver such a reply as a
whole reply with stop reason `end_turn`.

Request observations add `started_unix_ms`, `finished_unix_ms` and bounded
`attempt_diagnostics`. Existing audit/session observation and swarm `request_usage`
JSON payloads carry these fields; no logging/configuration switch is changed.
Older observations deserialize with unavailable timestamps and no attempt records.
A request ID identifies the logical request; each nested attempt has its original
attempt number. UTC values are Unix milliseconds, not monotonic clocks; elapsed
milliseconds use a monotonic clock. Clock adjustments can reorder UTC values.

Transport details are available for the OpenAI, Codex and Anthropic adapter
paths with a request trace (admission-owned, or observed beside the request). Disabled admission,
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
error classes must be updated before reading new records.

The `stalled` error class (#2210) is additive in the same way, in both
`AuditEvent::ProviderError.class` and `RequestObservation.error_class`: a reply
the harness abandoned because the provider stopped sending. So are two
`attempt_diagnostics.termination` values: `Idle`, a streaming reply that sent
nothing for the stream idle limit (300 s), and `TimedOut`, a non-streaming reply
that did not arrive within the reply total limit (20 min). Both limits are
described under "Stalled replies" in `runtime-models-providers.md`. Stop reasons are mapped
to a closed enum; unknown provider reasons retain only `Unknown`, never their text.

#2210 adds, in the same additive way: the `output_capped` error class (a reply
stopped at its output cap, never retried); two `attempt_diagnostics.termination`
values, `OutputCapped` (the harness abandoned the attempt at its output cap)
and `Interrupted` (the request ended — a deadline, an abort, a shutdown — while
the attempt was in flight, recorded from what it had streamed); and the
`attempt_diagnostics.output_bytes` counter, the bytes of output (text,
thinking, refusal and tool-call argument deltas) the attempt streamed, which
older records omit and read as `0`. Since #2210 the Codex and Anthropic
adapters also observe their attempts when no admission gate owns them, as the
OpenAI-compatible path has since #2151, so every streaming path of a traced
request records its attempts; a whole non-streaming Anthropic reply still
records none.
