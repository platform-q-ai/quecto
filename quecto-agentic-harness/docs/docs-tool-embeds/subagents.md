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

## Container environments (script-managed spawning)

When a config file defines `container_scripts`, `spawn` can place a child in an isolated, script-managed environment (e.g. a container with its own repository checkout) instead of a local process:

- `container: true` — new environment via the default script set. `{"mode":"new","repo"?,"container_script"?,"name"?}` — explicit repository/script/name (omitted `repo` uses the parent checkout's origin URL).
- New-environment spawns (`true` / `mode: "new"`) load `container_scripts` from a trusted config file: an explicit `config` argument in the spawn call wins; when omitted, the spawn falls back to the parent's own effective config path — so you normally need no `config` at all. Whichever path applies must be an absolute path. Joins (`mode: "existing"`) use the environment's retained scripts and never need `config`.
- Success returns `environment_ref=C1` (session-scoped, never reused). The child is a normal subagent — the whole completion sequence above applies unchanged.
- Add a teammate to a running environment: `container: {"mode":"existing","ref":"C1"}` (or `"name"`). Members share the environment's workspace but keep their own agent identity.
- `agent_cmd get_containers` (`agent_id: "*"`) lists every environment with status (`running`/`empty`/`killing`/`stopped`/`cleanup-failed`), workspace, and members. `kill_container` with `ref` or `name` stops one: all members are terminated and the environment's kill script runs exactly once; a failed kill is retryable by calling it again.
- When the final member exits, the environment tears itself down — no explicit kill needed for the happy path.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference (not embedded): `docs/subagents.md` in the repo
