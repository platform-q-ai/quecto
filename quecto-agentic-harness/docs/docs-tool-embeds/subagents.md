# Subagents (deep dive)

You already have `spawn` and `agent_cmd` tool schemas — use those for parameters and command enums. This page is coordination only.

## Required completion sequence

1. **Spawn** — returns when the socket is ready, not when work finishes.
2. **End the parent turn** (or do other non-blocking work). Do **not** poll or sleep.
3. **Next turn** — passive one-line completion note arrives automatically.
4. **`get_messages`** (`count` 1–5) — the note is **not** the report.
5. Synthesize for the user; `get_subagents_all` only for inventory/cleanup afterward.

## Defaults

- Never poll `get_subagents` / `get_subagents_all` / `get_state` as a wait loop; never bash-sleep for the child.
- `get_state` = occasional live phase/tools/progress. `get_messages` = committed transcript (may lag while busy; `snapshot: true`).
- Reviewers / non-editors: `read_only: true` (**not a hard sandbox** — child can still mutate via `bash`).
- Exact multi-step process: bind `workflow_spec` or `workflow: true` (see `docs {"name":"workflow"}`).

## Reuse

- Live idle child → `prompt`. Active → `steer` or `follow_up`. `agent_id` is only a display label: the same label after exit starts a fresh hidden UUID / clean session, not a resume.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference (not embedded): `docs/subagents.md` in the repo
