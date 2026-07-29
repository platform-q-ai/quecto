# Workflow (deep dive)

You already have the `workflow` tool schema (actions, params). This page is when/how to use it — not a full template dump.

## Availability

- UDS-only. Default: **dormant** tool (opt in mid-session). `--workflow` starts guided mode; `--no-workflow` removes the tool; `--workflow-guards` enables guard checks.
- Spawned children: `workflow: true` / `workflow_guards: true`, or bind with spawn `workflow_spec` (exact template, Active mode).

## Parent usage

- Coding tasks that need sequence, verification, or review structure → prefer a child with workflow (see `docs {"name":"quick-start"}`).
- Templates in-repo often include: `feature`, `bugfix`, `refactor`, `remove`, `chore`, `adversarial-review`, `investigate`, `flake-hunt`, `plan`, `prd`. Confirm with `list_templates` — do not invent ids.
- Bind `workflow_spec` when exact steps must be observable/auditable; otherwise let the child `select_template`.

## Runtime notes

- Workflow **state is not** rewritten into the system prompt every step (cache-friendly). State arrives via tool results and idle nudges.
- Guards are a convenience, not a security boundary.
- Session persistence keeps template/progress/issue across restarts when sessions are enabled.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference / embedded config examples: `docs/workflow.md` in the repo
