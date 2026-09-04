# Sessions

Sessions persist conversation history so the agent remembers context across
prompts and restarts. They are the primary state mechanism for UDS agents.

## How sessions work

Each session is identified by a key in the format `<interface>:<name>`:

| Interface | Default key | Example |
|-----------|-------------|---------|
| CLI agent | `cli:default` | `cli:my-project` |
| REPL | `repl:repl_default` | `repl:experiments` |

Sessions are stored as files in `<base_dir>/sessions/`. The file contains
the full conversation history (system prompt, user messages, assistant
responses, tool calls and results). For thinking-capable models (Claude
Sonnet 4.5+, Opus 4.5+), extended thinking blocks and their cryptographic
signatures are also persisted, enabling correct multi-turn replay.

## Session modes

### Persistent (default)

```bash
# Uses session "cli:default"
quecto agent --mode uds

# Uses session "cli:my-project"
quecto agent --mode uds -s my-project
```

When the agent starts, it loads the session from disk (if it exists). All
messages are appended to the session during the run. The session is saved
after each prompt completes.

### Ephemeral

```bash
# No session loaded or saved
quecto agent --mode uds --no-session
quecto agent --mode uds -s -
```

The agent starts with an empty conversation. Nothing is persisted to disk.
Useful for one-off tasks or testing.

### Named sessions

Session names must contain only alphanumeric characters, hyphens, and
underscores:

- ✅ `my-project`, `feature_42`, `review2024`
- ❌ `../tmp/evil`, `my project`, `session@home`

## Session lifecycle in UDS mode

```
Agent starts
  │
  ├── Load session from disk (if exists and not ephemeral)
  │
  ├── Client sends prompt
  │     ├── User message added to history
  │     ├── LLM response added to history
  │     ├── Tool calls/results added to history
  │     └── Session saved to disk
  │
  ├── Client sends another prompt
  │     └── (same cycle, building on previous history)
  │
  ├── Client disconnects
  │     └── Session already saved (no additional save on disconnect)
  │
  └── Agent exits
        └── Socket file cleaned up
```

## Context management

As conversations grow, the agent manages context automatically:

### Context window

The agent tracks estimated token usage against an application-level context
budget. When the conversation exceeds `max_context_tokens` (configurable,
default `200000`), the agent applies context pruning:

1. **Spilling at creation**: Tool outputs *and* conversation (user/assistant)
   messages are written to a spill file when they are created, so anything
   later collapsed or dropped can still be recovered with the `recall` tool
2. **Tool output collapsing**: Once the session accumulates more than
   `context_collapse_after_tool_calls` tool calls, the oldest tool outputs are
   replaced with compact recall stubs. The trigger counts tool calls
   cumulatively across prompts within a session. Current config default: `50`.
   Set it to `4294967295` (`u32::MAX`) to disable collapse.
3. **Conversation message collapsing**: An independent dial,
   `context_collapse_after_messages`, keeps the most recent N conversation
   messages in full and replaces older ones with one-line recall stubs.
   Exempt: the system prompt, spill manifest, in-flight user prompt, and the
   `pin_recent_turns` most recent turns. Defaults to 50 (mirroring the
   tool-call collapse default); set to `4294967295` (`u32::MAX`) to disable.
4. **Demotion ladder**: When the conversation still exceeds the effective
   budget, messages are demoted down a ladder — full content is collapsed to
   recall stubs first (oldest first), and only if the budget is still
   exceeded are stubs removed entirely (their content stays on disk). Pinned
   and tail-pinned (`pin_recent_turns`, default `2`) content is never
   demoted; if the pinned set alone exceeds the budget, a
   `context_prune` warning is logged and the `ContextPruned` audit event
   records `budget_unmet`.

The effective budget is the smaller of `max_context_tokens` and the active
model's context window when the model registry declares one.

### Spill and recall

Tool outputs are spilled to `<base_dir>/spills/<session_key>.jsonl`. Once the
spill store is non-empty, the conversation carries a pinned, static guidance
message that points the model to `recall("list")`; it deliberately contains no
spill count, IDs, or previews, so the provider-visible prompt prefix remains
byte-identical as the spill set grows. `recall("list")` returns the complete
live index on demand, and the `recall` tool description advertises that route.

When a message is collapsed, the compact stub looks like:

```
[bash: ls -la (2450 tokens) — recall("turn5:bash:0")]
```

The agent can call `recall("turn5:bash:0")` to retrieve the original content
from the spill file, even after the original message has been collapsed or
dropped by the sliding window.

## Inspecting sessions

### Via UDS commands

```json
{"type": "get_state"}
```

Returns the session key, message count, and streaming status.

```json
{"type": "get_messages"}
```

Returns the newest bounded page of conversation history (#1061).

```json
{"type": "get_messages", "count": 5}
```

Returns the last 5 messages (omit `count` for the newest bounded history page; the response's `before`/`hasMoreBefore` fields page older history, #1061).

```json
{"type": "get_session_stats"}
```

Returns token usage, message counts, and cost estimates.

### Clearing history

```json
{"type": "clear_history"}
```

Clears all messages except the system prompt. Drains any pending follow-up
or steer messages. Fails if the agent is currently streaming. See
UDS Protocol Reference (`docs {"name":"uds-protocol"}`) for full details.

## Configuration

Session behavior is configured in `config.json` under `agents.defaults`:

```json
{
  "agents": {
    "defaults": {
      "max_context_tokens": 100000,
      "context_collapse_after_tool_calls": 3
    }
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `max_context_tokens` | `200000` | Application-level token budget before context pruning (clamped down to the model's declared context window when known) |
| `context_collapse_after_tool_calls` | `50` | Collapse the oldest tool outputs once the session exceeds N tool calls. Set to `4294967295` (`u32::MAX`) to disable |
| `context_collapse_after_messages` | `50` | Collapse the oldest conversation (user/assistant) messages to recall stubs once the session exceeds N live messages. Set to `4294967295` (`u32::MAX`) to disable |
| `pin_recent_turns` | `2` | How many most-recent turns the context ceiling never demotes or drops |

## See also

- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — `get_state`, `get_messages`, `get_session_stats`, `clear_history`
- Subagents (`docs {"name":"subagents"}`) — each subagent gets its own session
