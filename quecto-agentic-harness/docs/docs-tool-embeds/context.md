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
- A newer full swarm `summary` supersedes the older ones: they collapse to
  recall stubs, and the newest always stays in full.
- In a swarm member, a large tool result (over 2000 estimated tokens)
  collapses to its recall stub once you have seen it for 3 turns, whatever
  the count dials say (off for other agents unless
  `context_collapse_large_result_tokens` is set; setting only the turns
  switches nothing on outside a swarm). A recalled result or one with
  images stays in full. Note what you need from a large output while it is
  in full, or recall it.
- In every agent, re-reading a file whose earlier result was collapsed or
  dropped returns its content, not the unchanged marker.

## Defaults

Configured under `agents.defaults`:

| Field | Default |
|---|---:|
| `max_context_tokens` | `200000` |
| `swarm_max_context_tokens` | `48000` |
| `context_collapse_after_tool_calls` | `50` |
| `context_collapse_large_result_tokens` | unset (`2000` in a swarm) |
| `context_collapse_large_result_after_turns` | unset (`3`) |
| `context_collapse_after_messages` | `50` |
| `pin_recent_turns` | `2` |

The effective context budget is clamped to what the active model's declared context window leaves the prompt when known (OpenAI with a declared output cap: the window less the cap, less 5% headroom; other providers and OpenAI entries without a cap: the window less what a request asks for, never under half the window), and to `swarm_max_context_tokens` once the process takes part in a swarm (from then on for the life of the process).

## Agent guidance

- Trust the stubs: they are recoverable pointers, not data loss.
- Use `recall("list")` when you need to find older spilled context.
- Use a specific spill id from a stub or the index when you need the full body.
- Prefer targeted recall over broad transcript reconstruction.
