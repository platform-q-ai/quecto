# PRD: Workflow Availability vs Workflow-Driven Agent Mode

## Problem

Today, `quecto agent --mode uds --workflow` enables workflow mode by registering the workflow tool and injecting workflow-selection instructions into the model’s system prompt.

This causes the model to immediately select a workflow template and start implementation on the first user message. That is acceptable for autonomous workflow sessions, but undesirable for normal conversational TUI sessions where the user may want to talk first and start a workflow later.

Currently, without `--workflow`, the workflow tool is unavailable, so a user cannot simply ask the model to “select the feature workflow and implement abc” unless the whole session was launched in workflow-forcing mode.

## Goal

Support three clear launch modes:

| Launch mode | Workflow tool available | Workflow prompt injected | Auto/nudge behavior | Intended use |
|---|---:|---:|---:|---|
| Normal TUI/UDS | Yes | No until template selected | No until template selected | Conversational session with optional workflow start |
| `--workflow` | Yes | Yes immediately | Yes after selection/progress | Autonomous workflow-driven implementation |
| `--no-workflow` | No | No | No | Completely disable workflow |

## Non-goals

- Do not remove or change existing autonomous `--workflow` behavior.
- Do not require users to start TUI with `--workflow` just to make workflow usable.
- Do not implement a full workflow selector UI as part of this change.
- Do not change workflow template semantics.

## User stories

### 1. Conversational TUI with optional workflow

As a user, I want to launch `quecto-tui` normally, chat with the agent, and later say:

> Select the feature workflow and implement abc

The model should then be able to call the `workflow` tool and start the template.

### 2. Autonomous workflow launch

As a user, I want to launch with `--workflow` when I explicitly want the agent pushed into workflow mode immediately.

The current behavior is acceptable here: the model sees workflow-selection instructions on first turn.

### 3. Workflow disabled

As a user or integration, I want to launch with `--no-workflow` to ensure the model has no workflow tool and receives no workflow context.

## Requirements

### R1: Register workflow tool by default for UDS/TUI

In normal UDS/TUI mode, the workflow tool should be available to the model even when `--workflow` is not passed.

This allows the user to explicitly request workflow use mid-conversation.

### R2: Do not inject selector-mode workflow prompt in normal mode

Normal mode must not append this kind of prompt before template selection:

```text
MODE: SELECT TEMPLATE
Call workflow(action="select_template", template="<id>") before checking steps.
```

This prompt is what causes the model to run off immediately.

### R3: Preserve `--workflow` behavior

When `--workflow` is passed:

- workflow tool is registered
- selector-mode workflow prompt is injected immediately
- model may select a template on first turn
- automation defaults remain enabled

### R4: Respect `--no-workflow`

When `--no-workflow` is passed:

- workflow tool is not registered
- workflow state is not created
- workflow prompt is never injected
- workflow automation commands should return “workflow is not active” or equivalent

### R5: Activate workflow prompt after template selection in normal mode

In normal mode, once a template is selected through the workflow tool, the dynamic workflow prompt should become active for subsequent model turns.

At that point, the model should receive:

```text
## Active Development Workflow
Template: ...
Progress: ...
CURRENT STEP → ...
```

### R6: Auto-continue and completion nudge only apply to active selected workflow

Automation should not start or select a workflow.

It may run only when:

- workflow state exists
- a template has been selected
- workflow mode is `Active` or `Complete`
- automation flags are enabled

## Proposed implementation model

Introduce a distinction between:

```text
workflow_available
workflow_prompt_active
workflow_disabled
```

Possible mapping:

```text
normal UDS/TUI:
  workflow_available = true
  workflow_prompt_active = false until template selected

--workflow:
  workflow_available = true
  workflow_prompt_active = true immediately

--no-workflow:
  workflow_available = false
  workflow_prompt_active = false
```

## Acceptance criteria

1. Launching normal `quecto-tui` exposes the workflow tool to the model.
2. Normal TUI first turn does not include selector-mode workflow instructions.
3. User can ask the model to select a workflow template, and the model can call the workflow tool.
4. After template selection, active workflow context appears in subsequent turns.
5. `quecto agent --mode uds --workflow` retains existing immediate workflow-driving behavior.
6. `quecto agent --mode uds --no-workflow` disables workflow entirely.
7. TUI workflow automation toggles do not start workflows by themselves.
8. Existing workflow tests continue passing, with new tests for the three launch modes.
