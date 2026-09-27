# Context and recall

Quecto manages long-running sessions with a configurable sliding context window. Do not ask the user to manually compact the conversation.

## How it works

- Tool results and user/assistant messages are spilled to disk when created.
- Older content can collapse into compact recall stubs as the active window fills.
- Once a limit is crossed, older content collapses in one batch, down to
  about 75% of the limit where it can (small limits and pinned content can
  keep it higher), so the following turns keep the provider's prompt cache.
- Tool results are never collapsed before you have seen them.
- The model can call `recall("list")` to inspect the live spill index.
- The model can call `recall("<spill-id>")` to retrieve full spilled content.
- Recent turns are pinned so the active working tail is preserved.

## Defaults

Configured under `agents.defaults`:

| Field | Default |
|---|---:|
| `max_context_tokens` | `200000` |
| `context_collapse_after_tool_calls` | `50` |
| `context_collapse_after_messages` | `50` |
| `pin_recent_turns` | `2` |

The effective context budget is clamped to the active model's declared context window when known.

## Agent guidance

- Trust the stubs: they are recoverable pointers, not data loss.
- Use `recall("list")` when you need to find older spilled context.
- Use a specific spill id from a stub or the index when you need the full body.
- Prefer targeted recall over broad transcript reconstruction.
