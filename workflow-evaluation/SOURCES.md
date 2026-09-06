# Source discovery and JSON contract

Inspected branch: `chore/workflow-quality`; source revision:
`c7f6ac7d871bd1b7ddaa2d154900a9fddf811ead`.
These findings come from implementation source and source tests, **not filesystem
operating documentation**. No operating docs were opened. `goal.md` was read as
requested. Paths/line numbers below refer to that revision.

## Actual built-ins

All five built-ins are in **one Rust source file**, not five JSON files:

`quecto-agentic-harness/src/domain/workflow/engine/templates.rs`

`default_templates()` starts at line 11. Its calls define:

| Template | ID line | Ordered step keys | Steps |
|---|---:|---|---:|
| investigate | 14 | scope, inspect, verify, report | 4 |
| chore | 46 | scope, change, check, review, handoff | 5 |
| bugfix | 84 | reproduce, diagnose, fix, regression, handoff | 5 |
| feature | 122 | intake, test_design, implement, refine, validate, handoff | 6 |
| refactor | 166 | scope, characterize, refactor, parity, handoff | 5 |

The helper `template()` at line 206 supplies `id`, `label`, `description`,
`when_to_use`, and maps each tuple `(key,label,phase,guidance)` to a step. **Every
built-in has `guards: []`.** `phase` is display/classification text, not an executable
TDD state machine. Labels, descriptions, and guidance are full template content;
IDs or step counts alone do not prove runtime equality.

Engine wiring: `src/domain/workflow/engine.rs:4–10` uses `default_templates()` only
when resolved `WorkflowConfig.templates` is empty. The same file re-exports it at
line 748. `src/domain/workflow/engine/templates_tests.rs` asserts the five IDs,
engine usability, and nonempty content; it is not the source definition.

The paths below are relative to `quecto-agentic-harness/`:

* `src/infrastructure/config_discovery.rs:27–105`: explicit `workflow.dir` wins,
  then `<cwd>/.quecto/workflows`, then `<home>/.quecto/workflows`, then inline
  `workflow.templates`. A chosen directory with no top-level templates errors,
  rather than falling back. Baseline setup must eliminate accidental shadowing.
* `src/interface/tool_runtime.rs:330–419`: bound spec bypasses directory discovery,
  installs the single supplied template, selects it, then binds the engine.
* `src/domain/workflow/binding.rs` and `engine.rs:117–200`: binding prevents selecting
  a different template; reset clears progress but remains bound/active. Baseline
  selection and revised bound activation are deliberately different per goal.md;
  record this procedural confound and optionally run an unchanged-bound control.

`workflow-config.json`, config examples, local/global `.quecto/workflows`, and docs
are **not** the shipped default definition. A locally built binary could differ from
source: archive its version/hash and corroborate the full runtime template. Our
`templates/builtin-*.json` are evaluator snapshots extracted from the source tuples,
not production edits or claims that the running binary uses them.

## There is no need to invent a schema from examples

The authoritative **resolved JSON schema is Serde structs plus semantic validation**:

* `src/domain/workflow.rs:19–45`: `WorkflowTemplateStep`, `WorkflowGuardRule`,
  `WorkflowTemplate`.
* `src/domain/workflow.rs:54–88`: `WorkflowSpec` and `WorkflowConfig`.
* `src/domain/workflow/engine.rs:673–729`: semantic template validation.
* `src/infrastructure/tools/spawn.rs:316–323`: spawn spec deserialization;
  its tool schema at line 476 only advertises `workflow_spec.template` as an object,
  not its complete nested contract.

### Resolved template object (inline config or spawn spec)

| Field | Type | Required/default |
|---|---|---|
| id | string | required, trimmed value must not be empty; unique in library |
| label | string | required; engine does not enforce nonempty |
| description | string | required; engine does not enforce nonempty |
| when_to_use | string or null | optional, default None; omitted on serialization if None |
| steps | array of step objects | required; 1–100 |
| guards | array of guard objects | optional, default []; omitted on serialization if empty; null is not an array |

Step: `key: string`, `label: string`, `phase: string` required;
`guidance: string|null` optional, default None. Keys must be nonblank and unique
within a template. `phase` is an arbitrary string, **not an enum**. There is no
runtime nonempty validation for label/phase/guidance, though built-in source tests
expect useful nonempty content. Step index actions use **1-based** indices.

Guard: `commands: string[]`, `before_step_key: string`, `message: string` all
required. Target key must exist in that template. Engine does not require nonempty
commands/message. At most **32 templates**. Uniqueness is exact-string based after
checking nonblank identifiers; validation does not normalize identifiers.

Unknown fields on the domain structs are ignored (no `deny_unknown_fields`). This
is intentional for backward-compatible inline configs and forward-compatible specs.
Do not rely on an unknown `inputs`, `acceptance`, or `budget` property to have any
runtime effect.

Our `resolved-template.schema.json` is a **derived evaluator aid**, not an upstream
published schema. It captures shape/defaults and simple bounds; semantic key/ID
uniqueness and guard references still need the engine validator. Avoid claiming a
JSON Schema pass is runtime validation.

### Spawn by value

```json
{
  "workflow": true,
  "workflow_guards": false,
  "effort": "low",
  "container": true,
  "workflow_spec": {
    "template": {
      "id": "example",
      "label": "Example",
      "description": "A complete by-value assignment",
      "steps": [{"key": "inspect", "label": "Inspect", "phase": "analysis", "guidance": "Read relevant evidence."}],
      "guards": []
    }
  }
}
```

This example illustrates the shape, **not** a proposed replacement workflow. The
revised experiment uses the entire voted candidate as `workflow_spec.template`,
not an ID, path, or directory-file object. Step references are **not resolved in a
spawn spec**: materialize steps before binding. The spec starts active and bound;
no selection instruction is needed in a revised task body.

Max serialized spec size **256 KiB**: `domain/workflow.rs:57`, write-side enforcement
in `infrastructure/tools/spawn_launch_ports.rs:90`, read-side in
`interface/tool_runtime.rs:436–452`. The loader removes the temporary spec file after
reading (even before JSON parsing); archive the original JSON outside that lifecycle.
`workflow_guards:true` requires `workflow:true` (`tools/spawn.rs:310–313`). Pilot
runs hold guards false throughout, because shipped built-ins have none and this
experiment isolates guidance quality rather than enforcement changes.

### Directory-file schema is different

`src/infrastructure/config_discovery.rs:107–268`:

* Top-level `.json` files only, lexically sorted; filename stem is the template ID.
  **An explicit `id` property is rejected.** Subfolders are not template libraries.
* Allowed top-level keys: `label`, `description`, `when_to_use`, `steps`, `guards`.
  Unknown fields are rejected at template, step, reference, and guard levels.
* Steps may be complete inline objects, a string such as `"steps/check"`, or
  `{"ref":"steps/check", "label":"Overridden label"}`. Override fields are
  `key`, `label`, `phase`, `guidance` (not arbitrary metadata).
* File size max 256 KiB; max 32 files; 1–100 steps. Domain validation subsequently
  handles nonblank IDs/keys and guard references.

Reference resolution: `src/infrastructure/config.rs:504–675`. Relative to config
file's parent for inline config; relative to workflow directory for directory
files. Appends `.json` when extension omitted. Rejects absolute/parent traversal,
canonical paths escaping the base (including symlinks), recursive references,
unknown step-file fields; referenced step file max **64 KiB**. Domain inline
objects stay lenient; referenced step files and directory objects are strict.

`WorkflowConfig` defaults `auto_continue:true`, `completion_nudge:true`, optional
`selector_prompt`/`dir` None, templates []; these switches are outside the template
object and must be held fixed/recorded in an experiment.

## Guards and progress are not evidence of completed engineering

`src/infrastructure/tools/workflow_tool.rs:406–472` applies command guards to the
`bash` tool only. `src/infrastructure/tools/command_match.rs` implements a heuristic,
case-insensitive binary/subcommand token matcher with chain/subshell handling and
quoted-region stripping, **not** regex, shell glob, or a security sandbox. Non-bash
write tools are not covered. `before_step_key` requires all steps **strictly before**
that key done; it does not require the named step itself done. Guard messages are
diagnostics, not additional executable rules.

`src/domain/workflow/engine.rs:159–192`: `check` enforces earlier done bits; `skip`
sets a done bit without ordering validation; `uncheck` clears just that bit. Checks
and skips both count as done for guards; persisted booleans alone cannot distinguish
them. Current step is first unfinished; visible progress hides later done flags
until gaps close. Therefore retain action transcripts and substantive evidence,
not just final progress percentage. Engine handoffs/nudges do not verify tests.

## Pilot launch update
See LAUNCH.md: workflow false omits the selector-nudge flag, not the workflow tool.
Baseline explicitly selects after trusted parent provisioning. This replaces the
initial workflow-true launch illustration for the idle-start pilot.
