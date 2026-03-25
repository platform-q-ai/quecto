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

## Configuration

Optional `workflow` section in `config.json`:

```json
{
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
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

## Built-in templates

When no custom templates are configured, four defaults are available:

| ID | Label | Steps | Description |
|----|-------|-------|-------------|
| `feature` | Feature | 7 | New capability: scenarios → tests → RED → GREEN → refactor → verify → commit |
| `fix` | Fix | 6 | Bug fix: reproduce → tests → RED → GREEN → verify → commit |
| `refactor` | Refactor | 5 | Cleanup: safety net → baseline → refactor → verify → commit |
| `chore` | Chore | 4 | Maintenance: scope → change → verify → commit |

Each template includes guard rules that block `git commit`/`git push` until
prerequisite steps are complete.

## Workflow modes

### Selector mode

Initial state. The agent must choose a template before checking steps.

### Active mode

A template is selected and steps are being worked through.

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
| `check_guards` | Evaluate all active-template guards |

### Template selection

```json
{"action": "select_template", "template": "fix", "issueNumber": 42, "issueTitle": "Login bug"}
```

If an issue was set in selector mode before selecting a template, it carries
over automatically.

### Step progression

```json
{"action": "check", "step": 1}
```

Both integer and string-encoded step numbers are accepted.

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

## UDS protocol

### `get_state` response

When workflow is enabled, `get_state` includes a `workflow` field with the
full engine snapshot. When disabled, the field is absent.

### `workflow_state` event

Emitted on template selection, step mutation, issue mutation, reset, and
completion transitions. Payload includes mode, progress, steps, active
template, active issue, and available templates.

When workflow is disabled, no `workflow_state` events are emitted.

## Custom templates

```json
{
  "workflow": {
    "templates": [
      {
        "id": "deploy",
        "label": "Deploy",
        "description": "Production deployment checklist.",
        "when_to_use": "Use for production releases.",
        "steps": [
          { "key": "pre_check", "label": "Run pre-deploy checks", "phase": "red" },
          { "key": "deploy", "label": "Deploy to production", "phase": "green" },
          { "key": "verify", "label": "Verify deployment", "phase": "green" },
          { "key": "announce", "label": "Announce release", "phase": "ci_cd" }
        ],
        "guards": [
          {
            "commands": ["kubectl apply", "helm upgrade"],
            "before_step_key": "deploy",
            "message": "Run pre-deploy checks before deploying."
          }
        ]
      }
    ]
  }
}
```

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
| `commands` | array | Bash command patterns to block |
| `before_step_key` | string | Block until all steps before this key are done |
| `message` | string | Error shown when a blocked command is attempted |
