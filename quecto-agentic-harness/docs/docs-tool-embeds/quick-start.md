# Quecto parent-agent quick start

Parent-agent identity is already in the system prompt. This page is the hot-path reminder for coordination; open deep dives for details.

## Route the work

Handle directly in the parent when the task is focused, short-lived, low-context, already localized, or requires user-facing synthesis/judgment.

Delegate to a subagent when the work is broad, noisy, long-running, independently parallelizable, review-shaped, or likely to produce lots of intermediate evidence while the parent only needs conclusions.

Once delegated, do not repeat the same investigation in the parent. Verify critical citations or surprising claims, then synthesize.

## Spawn and recover results

`spawn` returns when the child **socket is ready**, not when work is done.

Required sequence:

1. Spawn and brief the child with goal, boundaries, and expected concise report.
2. End this parent turn, or do other non-blocking, non-duplicative work.
3. On the next turn, wait for the passive one-line completion note.
4. Then call plain `agent_cmd get_messages` with `count`/`before` omitted or null to receive the unread report.
5. Verify, synthesize, and answer the user. The passive note is not the report.

Do **not** poll `get_subagents`, `get_subagents_all`, or `get_state` in a wait loop. Do not sleep/bash-wait for child completion.

## Delegation defaults

- Give each child one clear goal, ownership boundary, and expected deliverable.
- Children have separate LLM contexts; brief them with needed context.
- Long sessions are auto-managed: older detail may collapse into recall stubs; use `recall("list")` if you need to recover it.
- Ask for concise conclusions, evidence, uncertainty, and relevant `file:line` citations.
- Spawn reviewers, researchers, and other non-editing children with `read_only: true`.
- `read_only: true` hides write/edit tools but is not a hard sandbox because `bash` remains.
- Prefer minimal, purpose-aligned changes; follow repo conventions; verify appropriately; never bypass hooks with `--no-verify`.

## Workflow selection

For multi-step coding, diagnosis, planning, or review, prefer a child with `workflow: true`; use `workflow_spec` when the exact sequence must be observable/auditable. Confirm live template ids if unsure.

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

## Common loops

When the user says to "loop review/fix until the PR is clean" or similar, use an adversarial review ↔ bugfix loop:

1. Run an `adversarial-review` child with `read_only: true` against the PR.
2. If it reports real findings, run a `bugfix` child to fix them.
3. Re-run `adversarial-review` on the updated PR/diff.
4. Repeat until review finds no blocking issues, or until remaining issues are explicitly accepted/deferred.

Keep roles separate: reviewers do not edit; fixers do not waive findings. The parent adjudicates whether findings are real, whether fixes are sufficient, and when the PR is clean enough to merge.

## Reuse and inventory

Reuse a live child only when it already owns relevant context:

- idle child: `prompt`
- active child: `steer` or `follow_up`
- exited child: same `agent_id` label starts a fresh session, not a resume

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
