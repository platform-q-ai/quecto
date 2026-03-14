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
| `steps` | array | `[]` | Ordered list of workflow steps (max 100) |
| `auto_continue` | boolean | `true` | After each agent run, nudge the agent to continue with the next incomplete step |
| `completion_nudge` | boolean | `true` | When all steps are done, prompt the agent to close the issue and start a new cycle |
| `guards` | array | `[]` | Rules that block commands before specific steps |

### Step fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | integer | Step number (1-based, must be unique and sequential) |
| `label` | string | Human-readable description shown in status output |
| `phase` | string | Phase category: `"red"`, `"green"`, `"refactor"`, `"ci_cd"`, `"review"`, or any custom string |

Custom phase names are displayed as-is in the status output. The built-in
phase names (`red`, `green`, `refactor`, `ci_cd`, `review`) are uppercased
in the display (e.g., `"red"` → `[RED]`).

### Guard rules

Guards prevent the agent from running specific bash commands before reaching
a workflow step. This enforces process discipline — e.g., no committing before
tests pass.

| Field | Type | Description |
|-------|------|-------------|
| `commands` | array of strings | Commands to block (e.g., `"git commit"`, `"git push"`) |
| `before_step` | integer | Block these commands until all steps before this one are checked |
| `message` | string | Error message returned when a blocked command is attempted |

Guard matching uses binary + subcommand detection. `"git commit"` matches
`git commit`, `git commit -m "msg"`, `git commit --amend`, etc. It does not
match `git status` or `git log`. Commands inside quoted strings are ignored —
`echo "please git commit"` does **not** trigger the guard.

> **Note:** Guards are a developer convenience, not a security boundary.
> Any user with config.json access can modify or remove guards.

## The workflow tool

When enabled, the agent has a `workflow` tool with these actions:

### `status`

Show all steps and current progress. Read-only — does not modify state.

```json
{"action": "status"}
```

Returns a formatted checklist with phase groupings, check marks, progress
count, and the current step indicator:

```
## Active Development Workflow
Progress: 3/5 steps complete.
Active issue: #42 — Add feature X

[RED]
  [✓] 1. Write failing tests

[GREEN]
  [✓] 2. Implement code
CURRENT STEP → 4. Verify tests pass [GREEN]

[REFACTOR]
  [✓] 3. Refactor

[CI/CD]
  [ ] 5. Commit and push
```

### `check`

Mark a step as done. Enforces ordering — all previous steps must be checked
first. Returns an error if a preceding step is unchecked.

```json
{"action": "check", "step": 4}
```

The `step` field accepts both integers and string-encoded numbers (e.g.,
`"step": "4"` works the same as `"step": 4`).

### `uncheck`

Unmark a step (set it back to incomplete). Does **not** enforce ordering —
you can uncheck any step regardless of later steps' state. This can create
ordering gaps; use `reset` for a clean restart.

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

Reset all steps for a new development cycle. **Clears both all checkmarks
and the active issue.** Use after completing a full cycle and before starting
work on a new issue.

```json
{"action": "reset"}
```

### `set_issue`

Record the GitHub issue this cycle is working on.

```json
{"action": "set_issue", "issueNumber": 42, "issueTitle": "Add feature X"}
```

The issue number and title are shown in `status` output and injected into
the system prompt for context. Issue titles longer than 500 characters are
automatically truncated at a character boundary.

The `issueNumber` field accepts both integers and string-encoded numbers.

### `clear_issue`

Clear the active issue without resetting step progress.

```json
{"action": "clear_issue"}
```

### `check_commit`

Check whether all configured guard rules are satisfied. Returns an error if
any guard rule's required steps are incomplete, or a success message if all
guards pass. This is a read-only check — it does not modify state.

```json
{"action": "check_commit"}
```

## System prompt injection

When workflow is enabled, the agent's system prompt is augmented with:

1. **Current progress**: Which steps are done, which are pending
2. **Active issue**: The issue number and title being worked on
3. **Current step**: A clear `CURRENT STEP →` indicator pointing to the next
   incomplete step
4. **Phase grouping**: Steps grouped by phase (`[RED]`, `[GREEN]`, etc.)
5. **Guard reminders**: If guards are configured, a warning listing blocked
   commands and which steps must be completed first

This gives the LLM full awareness of where it is in the development process
without needing to call the `status` action explicitly.

### Auto-continue nudge

When `auto_continue` is enabled (the default), after each agent run completes
with incomplete steps, the system injects a nudge message:

> Continue the workflow — next incomplete step is step 4 (Verify tests pass).
> Proceed with this step now, then call workflow(action="check", step=4).

### Completion nudge

When `completion_nudge` is enabled (the default) and all steps are checked,
the system prompts the agent to close the issue, pick a new one, reset the
checklist, and begin the next cycle.

## Guard enforcement

Guards run as a pre-execution check on every `bash` tool call. When the
agent attempts to run a blocked command:

1. The guard parses the bash command to extract the binary and subcommands
2. It checks each guard rule against the parsed command
3. If a match is found and the required step hasn't been checked, the tool
   execution is blocked with a `BLOCKED:` prefixed error message
4. The LLM sees the error and (typically) works on completing the required
   steps before retrying

Non-bash tools (read, write, edit, workflow, etc.) always pass through
guards unconditionally.

### Guard parsing

The guard parser handles:

- Simple commands: `git commit -m "msg"` → binary=`git`, subcmd=`commit`
- Subshells: `$(git commit -m x)` and backtick subshells → detects `git commit`
- Pipes: `echo "msg" | git commit --file=-` → detects `git commit`
- Command chains: `git add . && git commit` → detects `git commit`
- Multiline commands: `echo hello\ngit commit -m x` → detects `git commit`
- Flag skipping: `git -C /path commit`, `git --git-dir /tmp/repo commit`,
  `git --work-tree /path commit` → correctly identifies `commit` as subcmd
- Case-insensitive: `GIT COMMIT` matches `git commit`
- Quoted strings ignored: `echo "git commit"` does **not** trigger the guard
- Comment lines: Lines starting with `#` are ignored

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

A full 14-step BDD/TDD workflow used for quecto's own development:

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
      { "id": 14, "label": "Run pre-merge hooks (real-LLM, machete, deny)", "phase": "ci_cd" }
    ],
    "guards": [
      {
        "commands": ["git commit", "git push"],
        "before_step": 5,
        "message": "Cannot commit/push until implementation and tests are complete (steps 1-4)"
      }
    ]
  }
}
```

## Workflow lifecycle

```
1. Agent starts → workflow state injected into system prompt
2. Agent calls workflow(set_issue) → records the active issue
3. Agent works through steps, calling workflow(check) after each
4. Guards block premature commands (e.g., git commit before tests pass)
5. auto_continue nudges agent to continue after each run
6. All steps checked → completion_nudge suggests closing the issue
7. Agent calls workflow(reset) → clears all steps and active issue
8. New cycle begins at step 1
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

Only the dynamic state (done flags + active issue) is persisted. Step
definitions come from config.json and are not stored in the session file.
If steps are added or removed in config.json between sessions, the done
flags are padded or truncated to match the new step count.

## UDS events

When the workflow state changes, a `workflow_state` event is emitted over
the UDS event bus. The event payload contains:

```json
{
  "type": "workflow_state",
  "steps": [
    { "id": 1, "label": "Write tests", "phase": "red", "done": true },
    { "id": 2, "label": "Implement", "phase": "green", "done": false }
  ],
  "progress": { "done": 1, "total": 2, "percent": 50 },
  "activeIssue": { "number": 42, "title": "Feature X" }
}
```

Events are emitted for all mutating actions (`check`, `uncheck`, `skip`,
`reset`, `set_issue`, `clear_issue`). Read-only actions (`status`,
`check_commit`) do not emit events.

## Deprecated configuration

The following fields are deprecated and will be removed in a future version:

- `guard_commit` (boolean) — replaced by `guards` array
- `enforce_commit_after_step` (integer) — replaced by `guards[].before_step`

If `guard_commit: true` is detected in config.json with no `guards` array,
the system automatically migrates it to an equivalent guard rule and logs a
warning.

## See also

- [UDS Protocol Reference](uds-protocol.md) — workflow state accessible via `get_state`
- [Disabling Tools](disable-tools.md) — `--disable-tool workflow` to remove the tool
