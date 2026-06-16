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
default `300000`), the agent applies context pruning:

1. **Sliding window**: Older non-pinned messages are dropped until the session
   is under budget (system prompt, first user message, and spill manifest are
   preserved)
2. **Spilling**: Tool outputs are written to a spill file when they are created,
   so dropped outputs can still be recovered with the `recall` tool
3. **Optional tool output collapsing**: Tool outputs older than
   `context_collapse_after_turns` turns are replaced with compact recall stubs
   before the sliding window drops them. Current config default: `50`. Set it
   to `4294967295` (`u32::MAX`) to disable collapse.

### Spill and recall

Tool outputs are spilled to `<base_dir>/spills/<session_key>.jsonl`. When a
message is collapsed, the compact stub looks like:

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

Returns the full conversation history.

```json
{"type": "get_messages_tail", "count": 5}
```

Returns the last 5 messages.

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
[UDS Protocol Reference](uds-protocol.md#clear_history) for full details.

## Configuration

Session behavior is configured in `config.json` under `agents.defaults`:

```json
{
  "agents": {
    "defaults": {
      "max_context_tokens": 100000,
      "context_collapse_after_turns": 3
    }
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `max_context_tokens` | `300000` | Application-level token budget before context pruning |
| `context_collapse_after_turns` | `50` | Collapse tool outputs older than N turns. Set to `4294967295` (`u32::MAX`) to disable |

## See also

- [Getting Started](getting-started.md) — quickstart guide for UDS agent integration
- [UDS Protocol Reference](uds-protocol.md) — `get_state`, `get_messages`, `get_session_stats`, `clear_history`
- [Subagents](subagents.md) — each subagent gets its own session
