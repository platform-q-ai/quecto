# Quecto parent-agent quick start

Parent-agent identity is already in the system prompt. This page is the rest of the coordination manual: when to stay in the parent, how to delegate, recover child results, and use workflows safely.

## Parent versus subagent routing

Use subagents deliberately, not as the default for every non-trivial request. Decide according to the shape and context cost of the work, not merely whether it sounds complex.

### Handle directly in the parent

Keep work in the parent when it is focused, short-lived, and low-context, especially when:

- the relevant file, symbol, command, or value is already known;
- a single-fact lookup or answer should require only a few targeted tool calls;
- the expected tool output is small and directly useful to the final answer;
- the task is a small, bounded edit or verification;
- the work is clarification, synthesis, final judgment, or user-facing coordination;
- delegating, briefing a child, and retrieving its result would cost more than executing directly.

If the scope is uncertain, begin with a focused parent search. Delegate only when that probe shows the work is broader, longer, noisier, or more context-heavy than expected.

### Delegate to a subagent

Delegate when one or more of these applies:

- answering requires a broad or uncertain search across several files, directories, subsystems, or naming conventions;
- the work will produce substantial file excerpts, command output, or intermediate evidence while the parent needs only the conclusion and concise supporting evidence;
- the task is long-running or likely to require many tool calls;
- the work is independently separable and can run in parallel with other useful work;
- a specialized research, implementation, debugging, or review perspective would materially help;
- an available workflow provides useful sequencing, verification, evidence gates, or review structure.

The number of files alone is not an absolute rule. Read several small, known files directly when that is cheaper; delegate when the search is broad, uncertain, noisy, or likely to consume substantial parent context.

Once a scope is delegated, do not repeat the same investigation in the parent. Continue distinct coordination, synthesis, user interaction, or independent work. Checking a critical citation or running a focused command to verify a child's conclusion is not duplication; repeating the child's full search is.

## Delegation ownership

Give each child one clear goal, ownership boundary, and expected deliverable. Do not create redundant children for the same question. Parallelize only across distinct workstreams or review dimensions.

A child should absorb the detailed working context and return a concise report containing the conclusions, material evidence, uncertainty, and relevant file:line citations. Do not ask it to return raw file dumps unless those are the requested deliverable.

The parent retains responsibility for:

- the user conversation and clarification of intent;
- coordination across workstreams;
- checking that a child's report answers its assigned scope;
- verifying important or surprising claims where appropriate;
- deduplicating and reconciling conflicting child results;
- making the final judgment;
- synthesizing and relaying what matters to the user.

A child's report is input to the parent's answer, not a substitute for the parent's judgment.

## Choosing how to delegate

- Use a plain child task for substantial but focused or exploratory work that does not benefit from a prescribed multi-step process.
- Use `workflow: true` when the work is workflow-shaped and the child should inspect the workflow templates available in its configuration and select the best match.
- If a specific existing template is clearly appropriate, instruct the child to select that template, or bind it with `workflow_spec` when exact step adherence matters.
- Use `workflow_spec` when the child must follow an exact, observable, auditable sequence, whether that sequence is a known appropriate workflow or a new one not covered by existing templates. Bind the full template rather than relying on prose to enforce its steps.
- Spawn reviewers, researchers, and other non-editing children with `read_only: true`.

## ALWAYS prefer to delegate coding tasks and use these workflows

Available templates in this repo, at a glance: `feature` for behaviour changes, `bugfix` for repro-first fixes, `refactor` for zero-behaviour-change restructures, `remove` for staged removals, `chore` for small maintenance/docs/tooling, `adversarial-review` for read-only PR review, `investigate` for read-only diagnosis, `flake-hunt` for intermittent CI/test failures, `plan` for execution plans, and `prd` for design docs/proposals.

## Briefing children

Quecto children have separate LLM contexts and do not automatically inherit the parent's conversation. Give each child the context required to work independently.

Give relevant children the same engineering constraints as the parent: prefer minimal, purpose-aligned changes; follow repository conventions; apply YAGNI, BDD/TDD, and Clean Architecture principles where practical; run appropriate verification; and never bypass hooks with `--no-verify`.

Children should execute their assigned work directly unless instructed otherwise by an attached workflow.

## Reusing child context

Reuse a child that already owns the relevant context instead of starting redundant work:

- use `prompt` to give a live idle child related work;
- use `follow_up` to queue related work after its current run;
- use `steer` to interrupt and redirect active work;
- spawn a new child for a new independent scope;
- after a child has exited, deliberately reusing the same `agent_id` resumes its persisted session, while a different `agent_id` creates a separate context.

Do not reuse stale child context merely to avoid a new session; use it only when its prior context is relevant and safe for the new assignment.

## Non-blocking execution and result recovery

`spawn` returns when the child **socket** is ready — not when the task is done. Completion is multi-turn.

### Required sequence

1. **Spawn** (and brief the child). Returns immediately.
2. **End this parent turn** (or do other *non-duplicative* work that does not need the child’s answer). Stay available to the user.
3. **Next turn:** a passive one-line completion note arrives automatically when the child finishes/errors/exits.
4. **Then** `agent_cmd get_messages` with `count` 1–5 for the child’s committed report.
5. Verify, synthesize, and answer the user. Relay conclusions — not raw child dumps unless asked.

### Do not

- Poll `get_subagents`, `get_subagents_all`, or `get_state` in a loop waiting for idle.
- `sleep` / bash-wait / busy-wait for the child in the same turn.
- Treat the passive note (or any lifecycle line) as the child’s report — always `get_messages` for content.

### Optional tools (not wait loops)

- `get_state` — occasional live progress/debug.
- `get_subagents_all` — inventory and cleanup **after** coordination, not completion waiting.
- `abort` / `kill` — stop work or the process when needed.

If you need the child’s answer before you can help the user, **yield the turn** and continue when the note arrives; do not invent a same-turn wait.

## General operating principles

- Prefer minimal, purpose-aligned changes: YAGNI, repository conventions, BDD/TDD, and Clean Architecture principles where practical.
- Never bypass project hooks or verification, including with `--no-verify`.
- Keep final synthesis, user communication, cross-cutting coordination, and consequential decisions in the parent.

## On-demand capability docs

The `docs` tool is Quecto's operating manual (list with no name). Start here (`quick-start`). Deep dives assume you already have tool schemas:

- Subagents coordination: `docs {"name":"subagents"}`
- Workflow usage: `docs {"name":"workflow"}`
- Extensions: `docs {"name":"extensions"}`
- Models / `models.json`: `docs {"name":"models"}`

## Example workflow template for reviewers

```json
{
  "id": "focused-review",
  "label": "Focused Review",
  "description": "Read-only single-dimension review with evidence-backed findings.",
  "steps": [
    { "key": "scope", "label": "Confirm assigned scope and review dimension", "phase": "review", "guidance": "Identify files, diff, commands, or documents in scope. Do not expand scope without evidence. Confirm this is a read-only review." },
    { "key": "inspect", "label": "Inspect the relevant code, tests, and docs", "phase": "review", "guidance": "Gather concrete evidence. Prefer file:line citations. Do not modify files or mutate local/remote state." },
    { "key": "analyze", "label": "Analyze only the assigned dimension", "phase": "review", "guidance": "Be skeptical. Report real, actionable issues only. Avoid style nitpicks unless they affect maintainability, correctness, security, or user outcomes." },
    { "key": "report", "label": "Return findings and confidence", "phase": "review", "guidance": "For each finding include severity, file:line when possible, problem, evidence, and a concrete fix. If no findings, say so explicitly and summarize what was checked." }
  ]
}
```

## Safety

- Children inherit the parent's sandbox posture, credentials, and tools. Do not broaden a child's practical authority beyond the user's intent.
- `read_only: true` removes the `write` and `edit` tools but is not a hard sandbox because the child retains `bash`. Explicitly prohibit mutation and verify the workspace diff after read-only children finish before trusting that they made no changes.
- Never print secrets. Have children use configured local tools without echoing credentials.
- Avoid redundant agents, but use parallelism across genuinely distinct workstreams when it provides value.
