# Context and recall

Quecto keeps long-running sessions within their context ceiling by itself,
built for a high prompt-cache hit rate. Do not ask the user to manually
compact the conversation.

## How it works

- Tool results and user/assistant messages are spilled to disk when created.
- Nothing earlier in the conversation is collapsed or edited in place: the
  context only grows at the end, so every request extends the one before.
- Once the request reaches the high mark (`context_high_tokens`, default
  `256000`), one cut takes it down to the low mark (`context_low_tokens`,
  default `70000`). The cut keeps the system messages, the first user message
  (the brief), the latest prompt and the newest whole exchanges that fit, and
  puts one archive stub right after the kept head (the system messages, the
  brief, and the latest prompt when it comes before the kept exchanges),
  before those exchanges:

```
[Context archive] 412 earlier messages of this session were archived to keep
the context small. recall("archive") lists them with their recall ids;
recall("<id>") reads any one in full.
```

- `recall("archive")` (or the id the stub names, `archive:2` after a second
  cut) lists every archived message with a preview and its recall id; each
  index starts with the previous archive's stub, so older archives chain.
- `recall("<id>")` reads one archived message in full.
- `recall("list")` shows the live spill index.
- The context ceiling still wins: under a lower `max_context_tokens`, model
  window or swarm ceiling, the high mark is the ceiling and the low mark
  scales with it.
- Only when no cut can bring a request under the ceiling (the brief alone is
  over it) does an emergency ladder stub, then drop, the oldest messages; its
  stubs read `[bash: ls -la (2450 tokens) — recall("turn5:bash:0")]`. Recent
  turns are pinned.
- Between cuts, earlier tool results stay in full; note what you need anyway,
  since the next cut archives them.
- Re-reading a file whose earlier result was archived or dropped returns its
  content, not the unchanged marker.

## Defaults

Configured under `agents.defaults`:

| Field | Default |
|---|---:|
| `max_context_tokens` | `300000` |
| `swarm_max_context_tokens` | `300000` |
| `context_high_tokens` | `256000` (env `QUECTO_CONTEXT_HIGH_TOKENS`) |
| `context_low_tokens` | `70000` (env `QUECTO_CONTEXT_LOW_TOKENS`) |
| `pin_recent_turns` | `2` (the emergency ladder's pinned turns) |

The old pruning keys (`context_mode`, `context_collapse_after_tool_calls`
and its old name `context_collapse_after_turns`,
`context_collapse_after_messages`, `context_collapse_large_result_tokens`,
`context_collapse_large_result_after_turns`) and the overrides
`QUECTO_CONTEXT_MODE`, `QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_TOKENS` and
`QUECTO_CONTEXT_COLLAPSE_LARGE_RESULT_AFTER_TURNS` were removed: a
configuration that still sets one is refused at load, naming every one with
the command that removes it (`quecto config unset agents.defaults.<key>
--global`, or `--local` for a trusted repo overlay; an untrusted overlay is
edited by hand, then `quecto config trust`); each can be unset on its own.

The effective context budget is clamped to what the active model's declared context window leaves the prompt when known (OpenAI with a declared output cap: the window less the cap, less 5% headroom; other providers and OpenAI entries without a cap: the window less what a request asks for, never under half the window), and to `swarm_max_context_tokens` once the process takes part in a swarm (from then on for the life of the process).

## Agent guidance

- Trust the stubs: they are recoverable pointers, not data loss.
- Use `recall("archive")` or `recall("list")` when you need older context.
- Use a specific id from a stub or an index when you need the full body.
- Prefer targeted recall over broad transcript reconstruction.
