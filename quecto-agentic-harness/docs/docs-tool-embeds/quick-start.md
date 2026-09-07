# Quecto agent quick start

Your system instructions define your role and when to delegate. This shared page covers tool mechanics and working practices, not a requirement to spawn agents. Open deep dives only when needed.

## Spawn and recover results

`spawn` returns when the child **socket is ready**, not when work is done.

Save the UUID returned by `spawn` for every child-targeted `agent_cmd.agent_id`; spawn labels are UI-only.

Required sequence:

1. Spawn and brief the child with goal, boundaries, and expected concise report.
2. End this turn, or do other non-blocking, non-duplicative work.
3. On the next turn, wait for the passive one-line completion note.
4. Then call plain `agent_cmd get_messages` with `count`/`before` omitted or null to receive the unread report.
5. Verify, integrate, and report results to your requester. The passive note is not the report.

Do **not** poll `get_subagents`, `get_subagents_all`, or `get_state` in a wait loop. Do not sleep/bash-wait for child completion.

First bare `get_messages` (omit/null `count` and `before`) returns the latest substantive assistant message, not the entire transcript. Subsequent bare calls return unread deltas across roles; when nothing new is available, `data` is `{ "unchanged": true }`. The report cursor advances only when the result is successfully delivered to the parent model, not merely fetched. Explicit non-null `count` and/or `before` requests are cursor-neutral history pages; `before` pages backward. Reports are bounded; a busy snapshot can lag the active turn.

## When delegating

- Give each child one clear goal, ownership boundary, and expected deliverable.
- Children have separate LLM contexts; brief them with needed context.
- Long sessions are auto-managed: older detail may collapse into recall stubs; use `recall("list")` if you need to recover it.
- Ask for concise conclusions, evidence, uncertainty, and relevant `file:line` citations.
- Spawn reviewers, researchers, and other non-editing children with `read_only: true`.
- `read_only: true` hides write/edit tools but is not a hard sandbox because `bash` can still mutate the workspace.
- Prefer minimal, purpose-aligned changes; follow repo conventions; verify appropriately; never bypass hooks with `--no-verify`.

## Workflow selection

For multi-step work, choose a workflow appropriate to the task. When spawning a child that needs a workflow, use `workflow: true`; use `workflow_spec` when the exact sequence must be observable/auditable. Confirm live template ids if unsure.

| Task shape | Template |
|---|---|
| Existing PR review | `adversarial-review` + `read_only: true` |
| Diagnosis/root cause only | `investigate` |
| Bug fix with repro | `bugfix` |
| Feature/change | `feature` |
| No-behavior restructuring | `refactor` |
| Small docs/tooling/config hygiene | `chore` |
| Deletion/removal | `remove` |
| Flaky CI/tests | `flake-hunt` |
| Execution plan | `plan` |
| Design doc / PRD | `prd` |

Do not mix these up: PR review uses `adversarial-review`; diagnosis without fixing uses `investigate`; behavior-preserving cleanup uses `refactor` or `chore`.

## Reuse and inventory

Reuse a live child only when it already owns relevant context:

- idle child: `prompt`
- active child: `steer` or `follow_up`

Inventory distinction:

- `get_subagents_all` with `agent_id: "*"` lists the parent/session-wide subagent inventory; use for inventory/cleanup, not waiting.
- `get_subagents` targets one specific live subagent and lists only that agent's nested children.

## Deep links

- `docs {"name":"subagents"}`: full delegation, lifecycle, reuse, safety, containers, inventory details.
- `docs {"name":"workflow"}`: full template guidance and workflow operation.
- `docs {"name":"context"}`: sliding context window, spill/recall, and long-running session behavior.
- `docs {"name":"extending"}`: route requests for new tools, model providers, and clients.
- `docs {"name":"extensions"}`: extensions.
- `docs {"name":"models"}`: model configuration.
