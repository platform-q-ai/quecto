# Quecto Workflow Extension

A [Pi](https://github.com/badlogic/pi-mono) extension that enforces the BDD/TDD Red→Green→Refactor development workflow defined in [`AGENTS.md`](../../AGENTS.md) as an interactive checklist.

## Why

The Quecto project mandates a strict 15-step development workflow for every change. This extension makes that workflow visible, trackable, and enforced — both for you and for the LLM agent working alongside you.

## The Workflow

Steps are grouped into phases, color-coded in the TUI:

| Phase | Steps | What happens |
|-------|-------|-------------|
| 🔴 **RED** | 1. Update scenarios / add features | Write or update `.feature` files and BDD scenarios |
| | 2. Write/update unit tests | Add failing test cases |
| | 3. Ensure tests FAIL | Verify the tests actually fail before implementing |
| 🟢 **GREEN** | 4. Implement code | Write the minimum code to make tests pass |
| 🟡 **REFACTOR** | 5. Refactor | Clean up for performance, security, and architecture |
| 🟢 **GREEN** | 6. Ensure tests still pass | Verify nothing broke during refactor |
| 🔵 **CI/CD** | 7. Commit | `git commit` |
| | 8. Push | `git push` |
| | 9. Create PR | Open a pull request |
| ⚪ **REVIEW** | 10. Despatch reviewers | Request Architecture, Security, and Performance reviews |
| | 11. Fix review concerns | Address all valid feedback |
| | 12. Push changes | Push fixes to remote |
| | 13. Reply & resolve comments | Mark review threads as resolved |
| 🔵 **CI/CD** | 14. Merge | Merge the PR |
| | 15. Pull to local master | `git checkout master && git pull` |

## Features

### Interactive Checklist

Open with `/workflow` or `Ctrl+Shift+W`. Navigate with ↑↓ or j/k, toggle steps with Enter/Space, reset all with R, close with Esc.

```
───── Quecto Dev Workflow ─────────────────────
  BDD/TDD Red → Green → Refactor

  ████████░░░░░░░░░░░░░░░░░░░░░░ 4/15 (27%)

  RED
  ▸ ✓  1. Update Scenarios / Add new features
    ✓  2. Write/update unit tests
    ✓  3. Ensure new/modified tests FAIL (RED)
  GREEN
    ✓  4. Implement code (GREEN)
  REFACTOR
    ○  5. Refactor (perf, security, clean arch)
  ...

  ↑↓ navigate  ·  Enter/Space toggle  ·  R reset  ·  Esc close
```

### Progress Widget

A one-line progress bar appears above the editor once you start working:

```
Workflow ████░░░░░░░░░░░ 4/15 (27%) → Step 5: Refactor (perf, security, clean arch) [REFACTOR]
```

Hidden when no steps are checked (no clutter on fresh sessions).

### LLM Workflow Tool

The agent can call the `workflow` tool to track its own progress:

| Action | Description |
|--------|-------------|
| `status` | Show all steps and current progress |
| `check <step>` | Mark a step as done |
| `uncheck <step>` | Unmark a step |
| `reset` | Clear all steps for a new cycle |
| `skip <step>` | Mark done even if earlier steps are incomplete |

The system prompt is injected each turn with the current step and a reminder to follow BDD/TDD order. Step-specific instructions are included when the agent reaches certain steps — for example, step 10 injects concrete `subagent` tool usage showing how to dispatch the architecture, security, and performance reviewers in parallel.

### Git Commit Guard

When the LLM runs `git commit`, the extension checks that steps 1–6 (the core RED→GREEN→REFACTOR→GREEN cycle) are all marked done. If not, you get a confirmation dialog listing the incomplete steps. In non-interactive mode, the commit is blocked outright.

### Sharded BDD Guard

When the LLM runs `cargo test --test bdd` without shard environment variables (`QUECTO_BDD_SHARD_INDEX` / `QUECTO_BDD_SHARD_TOTAL`), the extension blocks the command and directs the agent to use `scripts/run-bdd-shards.sh` instead (24-way parallel). Exceptions are allowed for:

- Commands that already set shard env vars inline
- Commands routed through `run-bdd-shards.sh`
- Single-scenario debugging with `QUECTO_TAG` (e.g., `@focus`)

In interactive mode, you get a confirmation dialog to override. In non-interactive mode, the command is blocked outright.

### Auto-Continue

When the agent stops with incomplete workflow steps, you can have it automatically continue. Toggle with:

- **`/workflow-auto`** command
- **`Ctrl+Shift+A`** shortcut

When enabled, the extension detects when the agent finishes (`agent_end`) and, if at least one step is checked but not all are done, sends a follow-up message telling the agent to continue with the next incomplete step. This creates a loop where the agent keeps working through the workflow until all 15 steps are complete.

The nudge is skipped if no steps have been checked yet (to avoid pestering on fresh sessions where the workflow hasn't started).

### State Persistence

State is stored in two ways:

- **Tool result details** — Every `workflow` tool call snapshots the full checklist into the session. This handles branching correctly (fork, `/tree` navigation).
- **`appendEntry`** — Manual toggles from `/workflow` are persisted as custom session entries. On restore, the most recent source wins.

State survives session restarts, `/new`, `/resume`, `/fork`, and `/tree`.

## Installation

Already installed — the extension lives at:

```
quecto/.pi/extensions/quecto-workflow.ts
```

Pi auto-discovers it when you run `pi` from the `quecto` directory. Use `/reload` to hot-reload after edits.

## Future Ideas

- **Auto-verify RED/GREEN** — Run `cargo test` automatically when checking steps 3 and 6, and confirm the exit code matches expectations (non-zero for RED, zero for GREEN).
- **Strict ordering** — Hard-block out-of-order step completion instead of warning.
- **Guard git push** — Block `git push` if step 7 (Commit) isn't checked.
- **Auto-reset** — Automatically reset the checklist after step 15.
- **Quality gate integration** — Run `scripts/check-quality.sh` and `cargo clippy` as part of the refactor step verification.
