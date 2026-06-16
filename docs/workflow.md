# Workflow V2

The workflow subsystem is a **UDS-only**, **opt-in**, native in-process runtime
that guides agents through structured development cycles using configurable
template libraries.

## Architecture

- **UDS-only**: workflow requires `quecto agent --mode uds --workflow`
- **Not available** in REPL or one-shot (`agent -m`) mode
- **Disabled by default**: no workflow engine, tool, prompt, or state unless
  explicitly enabled via `--workflow`
- **Template-library model**: multiple workflow templates (feature, fix,
  refactor, chore, or custom) — agents select one before starting
- **In-process engine**: `WorkflowEngine` owns all state; the UDS bus is
  the external read/broadcast interface, not the coordinator

## Startup flags

| Flag | Effect |
|------|--------|
| `--workflow` | Enable the workflow subsystem for this UDS session |
| `--no-workflow` | Explicitly disable workflow (clears `--workflow` and `--workflow-guards`) |
| `--workflow-guards` | Enable bash command guards (requires `--workflow`) |

All three flags require `--mode uds`. Using them without it produces an error.

### Typical invocation

```bash
# Built-in templates, guards enabled, named session
quecto agent --mode uds --workflow --workflow-guards -s my-session

# With a custom system prompt
quecto agent --mode uds --workflow --workflow-guards \
  --system "You are a senior engineer. Follow the workflow strictly." \
  -s feature-work
```

## Per-repo configuration

The workflow section lives inside `config.json`. By default quecto reads
`~/.quecto/config.json`, which applies globally. To scope workflow templates
to a specific repository, use `--config`:

```bash
# Use a repo-local config with project-specific workflow templates
quecto agent --mode uds --workflow --workflow-guards \
  --config ./my-repo/.quecto/config.json \
  -s my-session
```

This lets different repos define different template libraries, guard rules,
and nudge behavior. The `--config` flag overrides the entire config — provider
credentials and all agent defaults must also be present in the specified file.

> **Important:** The default exec isolation mode is `nsjail`, which runs bash
> commands inside a sandboxed container that only mounts the workspace directory.
> Tools like `gh`, `git push`, and anything that reads `~/.config/` or
> `~/.gitconfig` will fail because `$HOME` is not mounted. If your workflow
> needs Git/GitHub operations, add `"tools": { "exec": { "isolation": "native" } }`
> to your config file, or pass `--no-sandbox` and `--network` when launching
> the agent.

### Minimal per-repo config example

A repo-local config that uses OpenAI with custom workflow templates:

```json
{
  "providers": {
    "openai": {
      "api_key": ""
    }
  },
  "tools": {
    "exec": {
      "isolation": "native"
    }
  },
  "agents": {
    "defaults": {
      "model": "openai/gpt-5.5"
    }
  },
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
    "templates": [
      {
        "id": "feature",
        "label": "Feature",
        "description": "New capability with full TDD cycle.",
        "when_to_use": "Use for any new user-facing behavior.",
        "steps": [
          { "key": "scenarios", "label": "Update scenarios", "phase": "red" },
          { "key": "tests", "label": "Write failing tests", "phase": "red" },
          { "key": "red", "label": "Verify tests fail (RED)", "phase": "red" },
          { "key": "green", "label": "Implement (GREEN)", "phase": "green" },
          { "key": "refactor", "label": "Refactor", "phase": "refactor" },
          { "key": "verify", "label": "Verify tests pass", "phase": "green" },
          { "key": "commit", "label": "Commit and push", "phase": "ci_cd" }
        ],
        "guards": [
          {
            "commands": ["git commit", "git push"],
            "before_step_key": "commit",
            "message": "Complete RED-GREEN-REFACTOR before committing."
          }
        ]
      },
      {
        "id": "fix",
        "label": "Fix",
        "description": "Bug fix with reproduction test.",
        "when_to_use": "Use when behavior is broken.",
        "steps": [
          { "key": "repro", "label": "Reproduce the bug", "phase": "red" },
          { "key": "test", "label": "Write regression test", "phase": "red" },
          { "key": "fix", "label": "Implement fix", "phase": "green" },
          { "key": "verify", "label": "Verify fix", "phase": "green" },
          { "key": "commit", "label": "Commit", "phase": "ci_cd" }
        ],
        "guards": [
          {
            "commands": ["git commit"],
            "before_step_key": "commit",
            "message": "Verify the fix before committing."
          }
        ]
      }
    ]
  }
}
```

Invoke with:

```bash
quecto agent --mode uds --workflow --workflow-guards \
  --config ./my-repo/.quecto/config.json -s fix-auth-bug
```

## Configuration reference

Optional `workflow` section in `config.json`:

```json
{
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
    "selector_prompt": null,
    "templates": []
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `auto_continue` | boolean | `true` | Nudge the agent to continue with the next step after each turn |
| `completion_nudge` | boolean | `true` | Prompt the agent to close and cycle when all steps are done |
| `selector_prompt` | string | null | Custom prompt shown during template selection |
| `templates` | array | `[]` | Custom template definitions (empty = use built-in defaults) |

### Template fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique template identifier |
| `label` | string | yes | Display name |
| `description` | string | yes | Short description for template selection |
| `when_to_use` | string | no | Selection guidance for the model |
| `steps` | array | yes | Ordered step definitions (min 1, max 100) |
| `guards` | array | no | Guard rules for this template |

### Step fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | string | yes | Stable unique identifier within the template |
| `label` | string | yes | Human-readable description |
| `phase` | string | yes | Phase category (`red`, `green`, `refactor`, `ci_cd`, `review`, or custom) |
| `guidance` | string | no | Step-specific guidance injected into the prompt when this is the current step |

### Guard fields

| Field | Type | Description |
|-------|------|-------------|
| `commands` | array | Bash command patterns to block (e.g. `"git commit"`, `"git push"`) |
| `before_step_key` | string | Block until all steps before this key are done |
| `message` | string | Error shown when a blocked command is attempted |

### Validation rules

- Template `id` values must be unique
- Step `key` values must be unique within each template
- Each template must have at least one step (max 100)
- Guard `before_step_key` must reference an existing step key in the same template
- Max 32 templates per config

## Built-in templates

When `templates` is empty (or omitted), four defaults are loaded:

| ID | Label | Steps | Guards |
|----|-------|-------|--------|
| `feature` | Feature | 7 (scenarios → tests → RED → GREEN → refactor → verify → commit) | `git commit`/`git push` before commit step |
| `fix` | Fix | 6 (reproduce → tests → RED → GREEN → verify → commit) | `git commit`/`git push` before commit step |
| `refactor` | Refactor | 5 (safety net → baseline → refactor → verify → commit) | `git commit` before commit step |
| `chore` | Chore | 4 (scope → change → verify → commit) | `git commit` before commit step |

To override built-ins, define at least one template in `templates`. When any
custom templates are present, **only** the custom templates are available —
built-ins are not merged.

## Workflow modes

### Selector mode

Initial state. The agent must choose a template before checking steps. The
system prompt shows available templates with descriptions and `when_to_use`
guidance.

### Active mode

A template is selected and steps are being worked through. The system prompt
shows the template name, progress, current step with guidance, and guard
reminders.

### Complete mode

All steps in the template are checked. The agent is nudged to close the
current issue and begin a new cycle.

## The workflow tool

Available actions:

| Action | Description |
|--------|-------------|
| `status` | Show current progress (read-only) |
| `list_templates` | Show available templates with descriptions and when-to-use guidance |
| `select_template` | Activate a template and start a new run |
| `check` | Mark a step as done (enforces ordering) |
| `uncheck` | Unmark a step |
| `skip` | Mark a step as done without ordering enforcement |
| `reset` | Return to selector mode |
| `set_issue` | Record the active GitHub issue |
| `clear_issue` | Clear the active issue |
| `check_guards` | Evaluate active-template guards that match a supplied `command` |

### Template selection

```json
{"action": "select_template", "template": "fix", "issueNumber": 42, "issueTitle": "Login bug"}
```

If an issue was set in selector mode before selecting a template, it carries
over automatically. An explicit issue in `select_template` overrides any
previously set issue.

### Step progression

```json
{"action": "check", "step": 1}
```

Both integer and string-encoded step numbers are accepted. Steps are 1-indexed.

## System prompt injection

When workflow is enabled, the system prompt is rebuilt from live engine state
**before every LLM turn**:

- **Selector mode**: available templates, selector guidance, active issue
- **Active mode**: template name, progress, current step with guidance, guard
  reminders
- **Complete mode**: completion indicator and cycle-reset guidance

The workflow section is transient — it is not persisted in session history.

## Session persistence

`WorkflowRun` is persisted as first-class session metadata:

- `template_id`, `done` vector, and `active_issue` survive restarts
- If a persisted `template_id` no longer exists in the library, the engine
  recovers to selector mode
- Ordering gaps in the `done` vector are normalized on restore

## Guards

When `--workflow-guards` is set, template guard rules block guarded bash
commands until prerequisite steps are complete:

```
BLOCKED: Complete implementation and verification steps before commit/push.
Run workflow(action='status') to see current progress.
```

Guards only evaluate the **active template's** rules. Switching templates
changes which guards are active. Non-bash tools are never blocked.

> **Note:** Guards are a developer convenience, not a security boundary.
> Any user with config access can modify or remove guard rules.

## UDS protocol

### `get_state` response

When workflow is enabled, `get_state` includes a `workflow` field with the
full engine snapshot. When disabled, the field is absent.

### `workflow_state` event

Emitted on template selection, step mutation, issue mutation, reset, and
completion transitions. Payload includes mode, progress, steps, active
template, active issue, and available templates.

When workflow is disabled, no `workflow_state` events are emitted.

## Complete config examples

### Example 1: Built-in templates with guards (simplest setup)

```json
{
  "providers": {
    "openai": { "api_key": "" }
  },
  "agents": {
    "defaults": { "model": "openai/gpt-5.5" }
  },
  "workflow": {}
}
```

```bash
quecto agent --mode uds --workflow --workflow-guards -s my-session
```

This uses the 4 built-in templates (feature, fix, refactor, chore) with their
default guard rules. The empty `workflow: {}` uses all defaults.

### Example 2: Custom deployment workflow

```json
{
  "providers": {
    "openai": { "api_key": "" }
  },
  "agents": {
    "defaults": { "model": "openai/gpt-5.5" }
  },
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
    "selector_prompt": "Choose the workflow that best matches this deployment task.",
    "templates": [
      {
        "id": "deploy",
        "label": "Production Deploy",
        "description": "Full production deployment checklist with rollback plan.",
        "when_to_use": "Use for any production release.",
        "steps": [
          { "key": "changelog", "label": "Update CHANGELOG", "phase": "red", "guidance": "Document all user-facing changes since the last release." },
          { "key": "version", "label": "Bump version", "phase": "green" },
          { "key": "test", "label": "Run full test suite", "phase": "green" },
          { "key": "build", "label": "Build release artifacts", "phase": "ci_cd" },
          { "key": "stage", "label": "Deploy to staging", "phase": "ci_cd" },
          { "key": "verify_stage", "label": "Verify staging", "phase": "green" },
          { "key": "deploy_prod", "label": "Deploy to production", "phase": "ci_cd" },
          { "key": "verify_prod", "label": "Verify production", "phase": "green" },
          { "key": "announce", "label": "Announce release", "phase": "ci_cd" }
        ],
        "guards": [
          {
            "commands": ["kubectl apply", "helm upgrade", "docker push"],
            "before_step_key": "deploy_prod",
            "message": "Complete staging verification before production deploy."
          }
        ]
      },
      {
        "id": "hotfix",
        "label": "Hotfix",
        "description": "Emergency production fix with minimal process.",
        "when_to_use": "Use for critical production issues only.",
        "steps": [
          { "key": "repro", "label": "Reproduce in staging", "phase": "red" },
          { "key": "fix", "label": "Implement fix", "phase": "green" },
          { "key": "test", "label": "Targeted regression test", "phase": "green" },
          { "key": "deploy", "label": "Deploy hotfix", "phase": "ci_cd" },
          { "key": "verify", "label": "Verify in production", "phase": "green" }
        ],
        "guards": [
          {
            "commands": ["kubectl apply", "helm upgrade"],
            "before_step_key": "deploy",
            "message": "Run regression test before deploying hotfix."
          }
        ]
      }
    ]
  }
}
```

### Example 3: Workflow without guards (advisory only)

```bash
quecto agent --mode uds --workflow -s advisory-session
```

Without `--workflow-guards`, the workflow tool tracks progress and injects
prompt state, but no bash commands are blocked. Useful for advisory workflows
where process enforcement is not desired.

### Example 4: Disable auto-nudging

```json
{
  "workflow": {
    "auto_continue": false,
    "completion_nudge": false
  }
}
```

The agent tracks workflow state but does not autonomously continue to the next
step or prompt for issue cycling. The LLM only interacts with the workflow when
explicitly asked.

## Disabling workflow completely

Workflow is **off by default**. If you never pass `--workflow`, no workflow
subsystem is created and there is zero runtime overhead:

```bash
# No workflow — standard UDS agent
quecto agent --mode uds -s my-session
```

When workflow is disabled:

- No `WorkflowEngine` is created
- No `workflow` tool is registered (the LLM cannot see or call it)
- No workflow prompt section is injected into the system prompt
- No `workflow_state` events are emitted over the UDS bus
- `get_state` contains no `workflow` field
- No workflow run is loaded from or saved to the session
- No `WorkflowGuard` is registered — bash commands are never blocked by
  workflow rules

### Explicitly disabling with `--no-workflow`

If a wrapper script or alias passes `--workflow` and you need to override it:

```bash
# Override: explicitly disable even if --workflow appeared earlier
quecto agent --mode uds --workflow --no-workflow -s my-session
```

`--no-workflow` clears both `--workflow` and `--workflow-guards`. The last
flag wins — `--workflow` after `--no-workflow` re-enables the subsystem.

### Disabling guards only

To keep the workflow tool and prompt injection but remove all command blocking:

```bash
# Workflow enabled, guards disabled
quecto agent --mode uds --workflow -s my-session
```

Simply omit `--workflow-guards`. The agent tracks progress and injects prompt
state, but no bash commands are intercepted. Templates may still define guard
rules in their config — they are simply not enforced at runtime.
