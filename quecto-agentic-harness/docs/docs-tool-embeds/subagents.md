# Subagents (deep dive)

You already have `spawn` and `agent_cmd` tool schemas — use those for parameters and command enums. This page is coordination only.

Save the UUID returned by `spawn` and use it as `agent_cmd.agent_id` for every child-targeted command. Spawn labels are UI-only; `"*"` is for inventory/container commands.

## Required completion sequence

1. **Spawn** — returns when the socket is ready, not when work finishes.
2. **End the parent turn** (or do other non-blocking work). Do **not** poll or sleep.
3. **Next turn** — passive one-line completion note arrives automatically.
4. **Plain `get_messages`** (omit/null `count` and `before`) — the note is **not** the report.
5. Synthesize for the user; `get_subagents_all` only for inventory/cleanup afterward.

## Report semantics

First bare `get_messages` (omit/null `count` and `before`) returns the latest substantive assistant message, not the entire transcript. Subsequent bare calls return unread deltas across roles; when nothing new is available, `data` is `{ "unchanged": true }`. The report cursor advances only when the result is successfully delivered to the parent model, not merely fetched. Explicit non-null `count` and/or `before` requests are cursor-neutral history pages; `before` pages backward. Reports are bounded; a busy snapshot can lag the active turn.

## Defaults

- Never poll `get_subagents` / `get_subagents_all` / `get_state` as a wait loop; never bash-sleep for the child.
- `get_state` = occasional live state/effort/model/progress (+ slim workflow identity/current step if selected), with `generation`. Pass `since` to get `{ "unchanged": true, "generation": N }` when nothing changed. Plain `get_messages` = default unread report; explicit `count`/`before` = cursor-neutral transcript pages (may lag while busy; `snapshot: true`).
- `get_subagents_all` with `agent_id: "*"` is parent/session-wide inventory for top-level children. `get_subagents` must target a specific live subagent and lists only that agent's nested children (often `subagents: []`).
- Reviewers / non-editors: `read_only: true` (**not a hard sandbox** — child can still mutate via `bash`).
- Exact multi-step process: bind `workflow_spec` or `workflow: true` (see `docs {"name":"workflow"}`).

## Reuse

- Live idle child → `prompt`. Active → `steer` or `follow_up`. Use the child’s returned UUID, not its UI label.

## Ending children

- `agent_cmd kill` (by UUID) asks the child to shut down over its control connection; it settles its own children the same way. Result `graceful` / `fallback` / `already-exited`; an idle child ends in milliseconds, worst case ≈ 19 s.
- Children end with their launcher. Restore is history only: re-spawn what you need.

## Container spawning (named container configs)

With `container_configs` configured, `spawn` can place a child in an isolated container (its own repository checkout) instead of a local process. Each named config is **self-contained**: repository and auth are baked into it — no repo field; where the parent runs only decides which overlay applies.

- Menu: `quecto config get --effective container_configs` (the spawn description carries no roster). Match the user's phrasing to a name: "spawn the quecto container" → `container: {"mode":"new","container_config":"quecto"}`; if ambiguous, offer the names and confirm.
- **This repository's container**: effective configs = the global file's plus the working directory's trusted `.quecto/config.json` overlay, merged entry-wise (an overlay `"default": true` un-defaults the global entries), so `container: true` in a bound repository lands in its config's container. Bind: `quecto config set --local container_configs.<name> '{"default":true,…}'`; undo: `quecto config unset --local container_configs.<name>`; a foreign overlay needs `quecto config trust` (untrusted = ignored, stderr says so). Runbook: `docs {"name": "config"}`.
- `container: true` — new container via the config labeled default. `{"mode":"new","container_config"?,"name"?}` — a named config, with an optional container name for later joins/kills. A config with no repository is a sandbox (empty workspace).
- New-container spawns read the effective configuration fresh at every spawn — you normally need no `config`. An explicit `config` argument replaces both layers (like `--config`) and must be an absolute path. Joins (`mode: "existing"`) use the container's retained config and never need `config`.
- Success returns `environment_ref=C1` (session-scoped, never reused). The child is a normal subagent — the completion sequence above applies.
- Add a teammate to a running environment: `container: {"mode":"existing","ref":"C1"}` (or `"name"`). Members share the workspace but keep their own agent identity.
- `agent_cmd get_containers` (`agent_id: "*"`) lists every environment with status (`running`/`empty`/`killing`/`stopped`/`cleanup-failed`/`retained`), workspace, and members. `kill_container` with `ref` or `name` stops one: all members are terminated and the config's kill operation runs exactly once; the JSON result carries the environment ref and up to 20 terminated member ids/names (`omitted_agents` reports overflow); a failed kill is retryable.
- When the final member of an ordinary environment exits, the environment tears itself down — no explicit kill needed. A swarm container is the exception: it is `retained` after the run ends (`metadata.retained` says whether the run ended or lost its coordinator) so the end state can be inspected, and needs an explicit `kill_container`.

## Running a bounded swarm

Read `docs {"name":"swarm"}` before directing a swarm. Workflow eligibility
follows swarm participation, not containerization: an ordinary container agent
(new or joined) may run with `workflow: true`, guards and a bound spec exactly
like a host-local agent. Never enable them for swarm workers; those launches are
rejected, a join into a container whose run exists fails before inference, and an
agent running a workflow (guards, bound spec or selected template) cannot create
a run; the workflow tool refuses inside a swarm, and members that joined
before the run existed are covered by the same rule once it does (create the
run before spawning workers). Use the configured container launch
above; there is no separate swarm daemon or swarm-specific image.
Give the coordinator the goal, constraints, command/review acceptance criteria,
fixed member limit (including itself) and deadline. It calls `swarm` `op=create`
before spawning local workers into that shared checkout. The external master
supervises the coordinator and does not count as a member.

For approval/clarification, block the affected task and yield, or use durable
`swarm_control` pause for a whole-run wait. A blocked run is terminal. Resume a
paused run before replying with `prompt` when idle or `steer` when busy. Retrieve
its control receipt, explicit acknowledgment and action report; transport
acceptance alone does not prove handling.

For progress, ask the coordinator to inspect `swarm` `op=summary`; retrieve its
report with `agent_cmd` `get_report`. Add `export_raw:true` for retained raw artifacts. Generic agent state is not the task board.
A terminal coordinator permits read-only swarm reports and supervisor-channel export; do not ask
for Python inbox/ack execution after completion. Request the final revision,
criterion evidence and any blockers, and preserve artifacts before teardown.
Artifact references resolve against the returned container `artifact_base`.
Typed `agent_cmd` `swarm_control` actions `pause`, `resume`, `close`, `extend`, `status`, and `usage_budget` bypass the model queue; every end of a swarm run is a pause only these controls resume or close. Swarm events and a dashboard remain follow-on work. Read the swarm manual for receipt and budget semantics.

## See also

- Manual index: `docs {}`
- Full human reference (not embedded): `docs/subagents.md` in the repo
