# Subagents (deep dive)

You already have `spawn` and `agent_cmd` tool schemas — use those for parameters and command enums. This page is coordination only.

## Required completion sequence

1. **Spawn** — returns when the socket is ready, not when work finishes.
2. **End the parent turn** (or do other non-blocking work). Do **not** poll or sleep.
3. **Next turn** — passive one-line completion note arrives automatically.
4. **Plain `get_messages`** (omit/null `count` and `before`) — the note is **not** the report.
5. Synthesize for the user; `get_subagents_all` only for inventory/cleanup afterward.

## Defaults

- Never poll `get_subagents` / `get_subagents_all` / `get_state` as a wait loop; never bash-sleep for the child.
- `get_state` = occasional live state/effort/model/progress (+ slim workflow identity/current step if selected), with `generation`. Pass `since` to get `{ "unchanged": true, "generation": N }` when nothing changed. plain `get_messages` = default unread report; explicit `count`/`before` = cursor-neutral committed transcript pages (may lag while busy; `snapshot: true`).
- `get_subagents_all` with `agent_id: "*"` is parent/session-wide inventory for top-level children. `get_subagents` must target a specific live subagent and lists only that agent's nested children (often `subagents: []`).
- Reviewers / non-editors: `read_only: true` (**not a hard sandbox** — child can still mutate via `bash`).
- Exact multi-step process: bind `workflow_spec` or `workflow: true` (see `docs {"name":"workflow"}`).

## Reuse

- Live idle child → `prompt`. Active → `steer` or `follow_up`. `agent_id` is only a display label: the same label after exit starts a fresh hidden UUID / clean session, not a resume.

## Container spawning (named container configs)

When a config file defines `container_configs`, `spawn` can place a child in an isolated container (with its own repository checkout) instead of a local process. Each named container config is **self-contained**: its repository and auth are baked into the config itself — there is no repo field, and where the parent is running is irrelevant.

- The spawn tool description lists the menu: `Available container configs: docker (default), quecto, ...` (session-start snapshot). Match the user's phrasing against it: "spawn the quecto container" → `container: {"mode":"new","container_config":"quecto"}`. Unambiguous match → just spawn. Ambiguous or unmatched → offer the roster, suggest the closest name (or the default), and confirm: "I can see `quecto`, `repoX` and `repoY` containers — assuming you mean `quecto` (the default)?" Selection errors also enumerate the available names.
- `container: true` — new container via the config labeled default. `{"mode":"new","container_config"?,"name"?}` — a named config, with an optional container name for later joins/kills. A config with no repository is a sandbox (empty workspace).
- New-container spawns load `container_configs` from a trusted config file: an explicit `config` argument in the spawn call wins; when omitted, the spawn falls back to the parent's own effective config path — so you normally need no `config` at all. Whichever path applies must be an absolute path. Joins (`mode: "existing"`) use the container's retained config and never need `config`.
- Success returns `environment_ref=C1` (session-scoped, never reused). The child is a normal subagent — the whole completion sequence above applies unchanged.
- Add a teammate to a running environment: `container: {"mode":"existing","ref":"C1"}` (or `"name"`). Members share the environment's workspace but keep their own agent identity.
- `agent_cmd get_containers` (`agent_id: "*"`) lists every environment with status (`running`/`empty`/`killing`/`stopped`/`cleanup-failed`), workspace, and members. `kill_container` with `ref` or `name` stops one: all members are terminated and the container config's kill operation runs exactly once; a failed kill is retryable by calling it again.
- When the final member exits, the environment tears itself down — no explicit kill needed for the happy path.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference (not embedded): `docs/subagents.md` in the repo
