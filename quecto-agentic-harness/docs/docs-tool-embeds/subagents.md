# Subagents (deep dive)

You already have `spawn` and `agent_cmd` tool schemas — use those for parameters and command enums. This page is coordination only.

## Defaults

- Prefer **passive completion notes** after spawn; use `await` only when the next parent action in the **same turn** must block on the child.
- Lifecycle notes / `await` results are **not** the child's report. Recover work with `agent_cmd get_messages` (`count` 1–5 for the final assistant report).
- `get_state` = live phase/tools/progress. `get_messages` = committed transcript (may lag while busy; `snapshot: true`).
- Reviewers / non-editors: `read_only: true` (**not a hard sandbox** — child can still mutate via `bash`).
- Exact multi-step process: bind `workflow_spec` or `workflow: true` (see `docs {"name":"workflow"}`).

## Recovery checklist

1. Spawn (returns when the socket is ready).
2. Do other parent work; wait for the passive note (or `await` if required).
3. `get_messages` with small `count` → synthesize for the user.
4. `get_subagents_all` and clean up when coordination ends.

## Reuse

- Live idle child → `prompt`. Active → `steer` or `follow_up`. Same `agent_id` after exit resumes the session; new id = new context.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference (not embedded): `docs/subagents.md` in the repo
