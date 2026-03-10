# Workflow Automation

The workflow system provides a configurable step-by-step checklist that the
agent follows during development tasks. It enforces a structured process
(typically BDD/TDD Red→Green→Refactor) and prevents the agent from skipping
steps or running commands out of order.

## Overview

When enabled, the agent has access to a `workflow` tool that tracks progress
through a series of numbered steps. The workflow state is injected into the
system prompt so the LLM always knows where it is in the process. Guard rules
can block specific commands (like `git commit`) until the required steps are
completed.

## Configuration

Add a `workflow` section to your `config.json`:

```json
{
  "workflow": {
    "enabled": true,
    "auto_continue": true,
    "completion_nudge": true,
    "steps": [
      { "id": 1, "label": "Write failing tests", "phase": "red" },
      { "id": 2, "label": "Implement code", "phase": "green" },
      { "id": 3, "label": "Refactor", "phase": "refactor" },
      { "id": 4, "label": "Verify tests pass", "phase": "green" },
      { "id": 5, "label": "Commit and push", "phase": "ci_cd" }
    ],
    "guards": [
      {
        "commands": ["git commit", "git push"],
        "before_step": 4,
        "message": "Cannot commit until tests pass (step 4)"
      }
    ]
  }
}
```

### Configuration fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable the workflow tool and system prompt injection |
| `steps` | array | `[]` | Ordered list of workflow steps |
| `auto_continue` | boolean | `true` | After each agent run, nudge the agent to continue with the next incomplete step |
| `completion_nudge` | boolean | `true` | When all steps are done, prompt the agent to close the issue and start a new cycle |
| `guards` | array | `[]` | Rules that block commands before specific steps |

### Step fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | integer | Step number (1-based, must be unique and sequential) |
| `label` | string | Human-readable description shown in status output |
| `phase` | string | Phase category: `"red"`, `"green"`, `"refactor"`, `"ci_cd"`, `"review"` |

### Guard rules

Guards prevent the agent from running specific bash commands before reaching
a workflow step. This enforces process discipline — e.g., no committing before
tests pass.

| Field | Type | Description |
|-------|------|-------------|
| `commands` | array of strings | Commands to block (e.g., `"git commit"`, `"git push"`) |
| `before_step` | integer | Block these commands until this step is checked |
| `message` | string | Error message returned when a blocked command is attempted |

Guard matching uses binary + subcommand detection. `"git commit"` matches
`git commit`, `git commit -m "msg"`, `git commit --amend`, etc. It does not
match `git status` or `git log`.

## The workflow tool

When enabled, the agent has a `workflow` tool with these actions:

### `status`

Show all steps and current progress.

```json
{"action": "status"}
```

Returns a formatted checklist:

```
Workflow Progress (3/5 steps complete)
Active issue: #42 — Add feature X

  [✓] 1. Write failing tests (red)
  [✓] 2. Implement code (green)
  [✓] 3. Refactor (refactor)
  [ ] 4. Verify tests pass (green)
  [ ] 5. Commit and push (ci_cd)
```

### `check`

Mark a step as done.

```json
{"action": "check", "step": 4}
```

Steps should be completed in order. The tool allows checking any step, but
the system prompt and auto-continue nudges guide the agent to follow the
intended order.

### `uncheck`

Unmark a step (set it back to incomplete).

```json
{"action": "uncheck", "step": 3}
```

### `skip`

Mark a step as done even if previous steps are incomplete. Use when a step
is not applicable to the current task.

```json
{"action": "skip", "step": 3}
```

### `reset`

Reset all steps for a new development cycle. Clears all checkmarks but
preserves the active issue.

```json
{"action": "reset"}
```

### `set_issue`

Record the GitHub issue this cycle is working on.

```json
{"action": "set_issue", "issueNumber": 42, "issueTitle": "Add feature X"}
```

The issue number and title are shown in `status` output and injected into
the system prompt for context.

### `clear_issue`

Clear the active issue.

```json
{"action": "clear_issue"}
```

## System prompt injection

When workflow is enabled, the agent's system prompt is augmented with:

1. **Current progress**: Which steps are done, which are pending
2. **Active issue**: The issue number and title being worked on
3. **Phase context**: The current phase (red/green/refactor/ci_cd/review)
4. **Auto-continue nudge**: After each agent run, a message reminding the
   agent to continue with the next step

This gives the LLM full awareness of where it is in the development process
without needing to call the `status` action explicitly.

## Guard enforcement

Guards run as a pre-execution check on every `bash` tool call. When the
agent attempts to run a blocked command:

1. The guard parses the bash command to extract the binary and subcommands
2. It checks each guard rule against the parsed command
3. If a match is found and the required step hasn't been checked, the tool
   execution is blocked and the guard's error message is returned
4. The LLM sees the error and (typically) works on completing the required
   steps before retrying

### Guard parsing

The guard parser handles:

- Simple commands: `git commit -m "msg"` → binary=`git`, subcmd=`commit`
- Subshells: `(cd dir && git commit)` → detects `git commit`
- Pipes: `echo "msg" | git commit --file=-` → detects `git commit`
- Command chains: `git add . && git commit` → detects `git commit`
- Flag skipping: `git -C /path commit` → correctly identifies `commit` as subcmd

### Example guard config

```json
{
  "guards": [
    {
      "commands": ["git commit", "git push"],
      "before_step": 6,
      "message": "Complete refactoring (step 5) and verify tests (step 6) before committing"
    },
    {
      "commands": ["cargo publish", "npm publish"],
      "before_step": 15,
      "message": "Cannot publish until the PR is merged (step 15)"
    }
  ]
}
```

## BDD/TDD workflow example

A full 16-step BDD/TDD workflow used for quecto's own development:

```json
{
  "workflow": {
    "enabled": true,
    "steps": [
      { "id": 1, "label": "Update Scenarios / Add new features", "phase": "red" },
      { "id": 2, "label": "Write/update unit tests", "phase": "red" },
      { "id": 3, "label": "Ensure new/modified tests FAIL (RED)", "phase": "red" },
      { "id": 4, "label": "Implement code (GREEN)", "phase": "green" },
      { "id": 5, "label": "Refactor (perf, security, clean arch)", "phase": "refactor" },
      { "id": 6, "label": "Ensure tests still pass (GREEN)", "phase": "green" },
      { "id": 7, "label": "Commit", "phase": "ci_cd" },
      { "id": 8, "label": "Push", "phase": "ci_cd" },
      { "id": 9, "label": "Create PR", "phase": "ci_cd" },
      { "id": 10, "label": "Despatch reviewers (Arch, Security, Perf)", "phase": "review" },
      { "id": 11, "label": "Fix all valid review concerns", "phase": "review" },
      { "id": 12, "label": "Push changes to remote", "phase": "review" },
      { "id": 13, "label": "Reply to comments and mark resolved", "phase": "review" },
      { "id": 14, "label": "Run pre-merge hooks (real-LLM, machete, deny)", "phase": "ci_cd" },
      { "id": 15, "label": "Merge", "phase": "ci_cd" },
      { "id": 16, "label": "Move to local master and pull", "phase": "ci_cd" }
    ],
    "guards": [
      {
        "commands": ["git commit", "git push"],
        "before_step": 7,
        "message": "Cannot commit/push until implementation and tests are complete (steps 1-6)"
      }
    ]
  }
}
```

## Workflow lifecycle

```
1. Agent starts → workflow injected into system prompt
2. Agent calls workflow(set_issue) → records the issue
3. Agent works through steps, calling workflow(check) after each
4. Guards block premature commands (e.g., git commit before tests pass)
5. auto_continue nudges agent to continue after each run
6. All steps checked → completion_nudge suggests closing the issue
7. Agent calls workflow(reset) → starts a new cycle
```

## Disabling the workflow

Set `"enabled": false` in config.json (the default). When disabled:

- The `workflow` tool is not registered
- No workflow state is injected into the system prompt
- No guard rules are enforced
- The agent operates without any process structure

To disable at runtime without changing config, use:

```bash
quecto agent --disable-tool workflow -m "quick fix"
```

This removes the workflow tool but does not disable guards (which are
enforced at the tool registry level, not through the workflow tool).

## Persistence

Workflow state is stored in-memory for the lifetime of the agent process.
In UDS mode, the state persists across multiple prompts within the same
session. In one-shot mode (`quecto agent -m`), the state resets on each
invocation.

For persistent workflow tracking across agent restarts, use session
persistence (`-s <name>`) — the workflow state is included in the session
file.

## See also

- [UDS Protocol Reference](uds-protocol.md) — workflow state accessible via `get_state`
- [Disabling Tools](disable-tools.md) — `--disable-tool workflow` to remove the tool
