# Quecto Workflow Extension

A [Pi](https://github.com/badlogic/pi-mono) extension that adds a native
workflow checklist to Pi for Quecto's development process.

It is implemented in [`quecto-workflow.ts`](./quecto-workflow.ts) and models the
current **14-step** Quecto workflow used by the extension itself.

## Why

The extension gives both the human operator and the LLM a shared, persistent
view of development progress:

- an interactive checklist UI
- a `workflow` tool the LLM can call directly
- prompt injection with current workflow state
- guardrails around `git commit`
- optional auto-continue and completion nudges

## Current workflow

The extension tracks these 14 steps:

| # | Phase | Step |
|---|---|---|
| 1 | RED | Update scenarios / add new features |
| 2 | RED | Write/update unit tests (quick smoke check only) |
| 3 | RED | Ensure new/modified tests fail (RED) |
| 4 | GREEN | Implement code (GREEN) |
| 5 | CI/CD | Commit |
| 6 | CI/CD | Push (pre-push runs tests/linting) |
| 7 | CI/CD | Create PR |
| 8 | REVIEW | Despatch sub agents in parallel as reviewers |
| 9 | REVIEW | Fix all valid review concerns |
| 10 | REVIEW | Push changes to remote |
| 11 | REVIEW | Reply to reviewer comments and mark resolved |
| 12 | CI/CD | Run pre-merge hooks |
| 13 | CI/CD | Merge |
| 14 | CI/CD | Move to local master and pull |

## Features

### Interactive checklist

Open the checklist with either:

- `/workflow`
- `Ctrl+Shift+W`

Controls:

- `↑` / `↓` or `j` / `k` — move selection
- `Enter`, `Space`, or `x` — toggle selected step
- `R` — reset all steps
- `Esc` or `Ctrl+C` — close the checklist

### Workflow widget

When the workflow is active, the extension shows a widget above the editor with:

- active issue number/title
- progress through the 14 steps
- the current phase
- the next incomplete step

### LLM workflow tool

The agent can call the `workflow` tool to manage progress itself.

Supported actions:

| Action | Description |
|---|---|
| `status` | Show current progress |
| `check` | Mark a step done |
| `uncheck` | Unmark a step |
| `skip` | Force-complete a step out of order |
| `reset` | Clear the workflow for a new cycle |
| `set_issue` | Set the active issue number/title |
| `clear_issue` | Clear the active issue |

The tool persists workflow state in tool result details so the checklist can be
reconstructed across session restores and branch navigation.

### System prompt injection

Before each turn, the extension injects workflow context into the system prompt,
including:

- active issue information
- progress summary
- current required step
- reminders to use the `workflow` tool
- completion guidance when all steps are done

### Git commit guard

When the agent tries to run `git commit`, the extension checks whether the
pre-commit workflow steps are complete. If not, the command is blocked (or
confirmed interactively, depending on Pi runtime behavior).

### Sharded BDD guard

The extension also blocks unsharded `cargo test --test bdd` runs and tells the
agent to use `scripts/run-bdd-shards.sh`, unless the command already specifies
shards or is doing focused scenario debugging.

### Auto-continue and completion nudges

Two optional helpers are included:

- `/workflow-auto` or `Ctrl+Shift+A` toggles auto-continue
- `/workflow-nudge` or `Ctrl+Shift+N` toggles the completion nudge

Auto-continue sends a follow-up instruction after `agent_end` when work has
started but the checklist is not complete yet. The completion nudge fires once
when all steps are complete and reminds the agent to close the current issue,
reset the workflow, and pick the next task.

## Installation

The extension lives in this repository at:

```text
quecto/.pi/extensions/quecto-workflow.ts
```

Pi auto-discovers it when you run Pi from the `quecto` repository. Use
`/reload` after editing it.

## OSS Fusion panel

The repository also carries the Perme8 Pi fusion panel extension at:

```text
quecto/.pi/extensions/oss-fusion.ts
```

It registers:

- the `oss_fusion` tool for LLM-triggered multi-model panel runs
- the `/fusion` command for user-triggered panel runs
- an `oss-fusion` message renderer for synthesized results

Supported modes are `readonly`, `full`, `sandbox`, and `patch-chain`. The
extension defaults to readonly mode and can be configured with environment
variables such as `OSS_FUSION_MODELS`, `OSS_FUSION_JUDGE_MODEL`,
`OSS_FUSION_SYNTHESIZER_MODEL`, and `OSS_FUSION_MODE`.

## Relationship to Quecto's native workflow engine

This extension is separate from Quecto's built-in UDS workflow engine described
in [`docs/workflow.md`](../../docs/workflow.md):

- **this extension** is a Pi-side checklist and guard system
- **Quecto workflow V2** is the native in-process workflow runtime exposed via
  the UDS `workflow` tool

Both are workflow-related, but they are distinct implementations with different
integration points.
