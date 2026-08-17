# Thinking traces

Quecto exposes display-safe model thinking separately from assistant answer text.

## Live UDS event

New clients may receive an additive event:

```json
{"type":"thinking","text":"display-safe reasoning summary"}
```

This event is distinct from `token`. Existing `token`, `tool_execution_start`, `tool_execution_end`, `turn_start`, `turn_end`, and `agent_end` events keep their existing meanings. Clients that do not understand `thinking` should ignore it and continue processing later events.

## Message recovery

Recovered assistant messages may include additive display-safe thinking:

```json
{
  "role": "assistant",
  "content": "final answer",
  "visibleThinking": [{"type":"thinking","text":"display-safe reasoning summary"}]
}
```

Large ranged or collapsed responses may omit `visibleThinking` and set `visibleThinkingTruncated: true` to keep recovery frames bounded. Provider-private replay data such as Anthropic signatures, encrypted reasoning, redacted payloads, and OpenAI continuity data is never exposed in UDS/API/TUI output.

## TUI

The TUI renders visible thinking as a separate `Thinking:` section before the assistant answer in live and recovered transcripts, including focused child/subagent transcripts. Answer text remains separate. Press `Ctrl+Y` to hide or show thinking; hiding displays a `Thinking...` placeholder and does not delete stored content. The visibility preference is remembered for later TUI sessions.

## Non-interactive output

Non-interactive/one-shot output remains answer-only by default; thinking traces are surfaced through the additive protocol and TUI display paths.
