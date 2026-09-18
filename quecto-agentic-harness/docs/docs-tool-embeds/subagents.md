# Subagents (deep dive)

You already have the `spawn` and `agent_cmd` schemas; this page is coordination only. Save the UUID `spawn` returns and use it as `agent_cmd.agent_id` for every child-targeted command. Spawn labels are UI-only; `"*"` is for inventory/container commands.

## Required completion sequence

1. **Spawn** — returns when the socket is ready, not when work finishes.
2. **End the parent turn** (or do other non-blocking work). Do **not** poll or sleep.
3. **Next turn** — a passive one-line completion note arrives.
4. **Plain `get_messages`** (omit/null `count` and `before`) — the note is **not** the report.
5. Synthesize for the user; `get_subagents_all` only for inventory/cleanup.

## Report semantics

First bare `get_messages` returns the latest substantive assistant message, not the whole transcript. Later bare calls return unread deltas across roles; with nothing new `data` is `{ "unchanged": true }`. The cursor advances only when the result reaches the parent model. Explicit non-null `count` and/or `before` are cursor-neutral history pages; `before` pages backward. Reports are bounded; a busy snapshot can lag.

## Defaults

- Never poll `get_subagents` / `get_subagents_all` / `get_state` as a wait loop; never bash-sleep for a child.
- `get_state` = occasional live state/effort/model/progress (+ slim workflow identity/step if selected), with `generation`; `since` returns `{ "unchanged": true, "generation": N }` when nothing changed.
- `get_subagents_all` (`agent_id: "*"`) is session-wide inventory of top-level children; `get_subagents` lists one live child's nested children only.
- Reviewers / non-editors: `read_only: true` (**not a hard sandbox**: `bash` can still mutate).
- Exact multi-step process: `workflow_spec` or `workflow: true` (`docs {"name":"workflow"}`).

## Reuse

- Live idle child → `prompt`. Active → `steer` or `follow_up`. Use the UUID, not the label.

## Ending children

- `agent_cmd kill` (by UUID) asks the child to shut down over its control connection; it settles its own children likewise. Result `graceful` / `fallback` / `already-exited`; worst case ≈ 19 s.
- Children end with their launcher; restore is history only.

## Container spawning

With `container_configs` configured, `spawn` can place a child in an isolated container: a **fresh clone of the config's `--repo` at its default branch** (no `--repo`: empty sandbox). The parent's working tree, branch and uncommitted changes are NOT inside; to work on a branch, push it and tell the child to fetch/checkout. Each config is self-contained (repo and auth baked in, no repo field).

**Run a subagent in this repo's container** (six steps):
1. `agent_cmd {"agent_id":"*","command":"get_container_configs"}` → `container_configs[]` with `name`, `default`, `source` (`overlay` = repo-bound, `global`), `repository`, `problem`, `joinable` (= the spawn description's `Available container configs:` line).
2. `spawn {"agent_id":"…","task":"…","container":true}` — this repo's `standard` entry when the roster shows one (no global default overrides it; none? `quecto container init`), else the labelled default; or `"container":{"mode":"new","container_config":"<name>","name"?}` (`name` for later joins/kills).
3. Read `container_config=<name>` in the result; confirm it is the repo meant.
4. On failure the error quotes the script's stderr; run `quecto container doctor` here, apply the remedy, retry.
5. `overlay_withheld: true` / `container: true refused` → the overlay is untrusted, refused or unparseable; `diagnostics` says which (`quecto config trust` if untrusted), then retry.
6. Follow the completion sequence above; `get_containers` lists the environment (`ref` for joins).

- Match the user's phrasing to a config name; if ambiguous, offer them.
- **This repository's container**: effective configs = the global file's plus the cwd's trusted `.quecto/config.json` overlay, merged entry-wise; a repo's `standard` entry is its default by rule, else an overlay `"default": true` un-defaults global entries. Runbook: `docs {"name": "container-runtime"}`; hand-rolled: `quecto config set --local container_configs.<name> '{"default":true,…}'`; undo: `config unset --local`. Only for runs started without `--config`, never inside a container child.
- An untrusted overlay is not applied: `container: true` is **refused** when it declares `container_configs`, is unparseable, fails the trust checks or is a symlink; one without `container_configs` launches the global default with a warning. A named `container_config` launches from the global set, diagnostic in result.
- New-container spawns read the effective configuration at every spawn; you normally need no `config`. An explicit `config` replaces both layers (absolute path). Joins use the retained config.
- Success returns `environment_ref=C1 container_config=<name>` (ref durable, never reused); the child is a normal subagent.
- Add a teammate to a running environment: `container: {"mode":"existing","ref":"C1"}` (or `"name"`); refs from `get_containers`.
- `agent_cmd get_containers` (`agent_id: "*"`) lists every environment with status (`running`/`empty`/`killing`/`stopped`/`cleanup-failed`/`retained`), workspace, and members, plus other sessions' (`restored: true`, `session`): join an `empty`/`retained` one with `mode: existing`; kill it when done. `kill_container` with `ref` or `name` stops one: members are terminated, the config's kill runs once; the result carries the ref and up to 20 member ids (`omitted_agents` on overflow); failed kills are retryable. Leftovers: `quecto container ls|kill|gc`.
- When the last member of an ordinary environment exits, it tears itself down. A swarm container is `retained` after the run ends (`metadata.retained` says why); it needs `kill_container`.

## Running a bounded swarm

Read `docs {"name":"swarm"}` before directing a swarm. The swarm runs in the
container the coordinator was spawned into: pick the config (step 1 above) and
launch the coordinator with `{"mode":"new","container_config":"<name>"}`;
only the official isolated-PID adapter can host a swarm (host-local scripts
and the host cannot). Workers are spawned by the coordinator with
`container` omitted. Workflow eligibility follows swarm participation, not
containerization: an ordinary container agent may run with `workflow: true`,
guards and a bound spec. Never enable them for swarm workers: those launches
are rejected, a join into a container whose run exists fails before inference,
an agent running a workflow cannot create a run, the workflow tool refuses
inside a swarm, and members that joined before the run existed fall under the
same rule once it does (create the run before spawning workers).
Give the coordinator the goal, constraints, command/review acceptance criteria,
member limit (itself included) and deadline. It calls `swarm` `op=create` before
spawning workers; the external master supervises and is not a member.

For approval/clarification, block the affected task and yield, or use a durable
`swarm_control` pause for a whole-run wait. A blocked run is terminal. Resume a
paused run before replying with `prompt` (idle) or `steer` (busy); retrieve its
control receipt and action report (transport acceptance does not prove handling).

For progress, ask the coordinator to inspect `swarm` `op=summary`; retrieve its
report with `agent_cmd` `get_report` (`export_raw:true` for raw artifacts).
Generic agent state is not the task board. A terminal coordinator permits
read-only swarm reports and supervisor-channel export only. Request the final
revision, criterion evidence and blockers; preserve artifacts (under
`artifact_base`) before teardown.
Typed `agent_cmd` `swarm_control` actions `pause`, `resume`, `close`, `extend`, `status`, `usage_budget` bypass the model queue; every end of a swarm run is a pause only these controls resume or close.

## See also

- Full reference (not embedded): `docs/subagents.md` in the repo
