# PRD: Composable, File-Based Workflow Templates & Cache-Safe Prompting

**Status:** Draft for review
**Scope:** `quecto-agentic-harness` workflow subsystem
**Related:** [`docs/workflow.md`](../workflow.md)

---

## 1. Problem

The workflow subsystem works well as a state machine (engine, tool, UDS
events, nudges, persistence) but has three structural problems, all verified
in the current code:

### P1 — Dynamic workflow state busts prompt caching

`agent.rs` installs a `system_prompt_provider` closure that re-renders the
system prompt before **every** turn, appending
`WorkflowEngine::prompt_snippet()`. That snippet embeds mutable state:
progress counts (`Progress: 3/20`), the current step, the active issue, and
per-step guidance blobs (up to ~3.3 KB each).

Every `workflow(action="check", ...)` therefore mutates the system prompt →
the provider-side cached prefix (system prompt + entire history) is
invalidated → caching is busted on every workflow step. Long workflow
sessions are exactly where prefix caching matters most, so this is the
worst-case interaction.

### P2 — Templates are monolithic and steps are copy-pasted

All templates live inline in one 44 KB `workflow-config.json`. The two
shipped templates (`feature`: 20 steps, `refactor`: 18 steps) share many
byte-identical steps maintained by copy-paste. There is no step library and
no way to reuse a step (e.g. "adversarial reviewers") across templates.

### P3 — No single source of truth

Tests (`workflow_config_template.rs`) pin **three copies** of the template
content against drift: `workflow-config.json`, `examples/config.json`, and
`.claude/workflows/feature.js`. This is a symptom of having no canonical
location; every guidance edit must be made in triplicate.

Additionally, per-repo template scoping today requires `--config`, which
replaces the *entire* config — forcing repos to duplicate provider
credentials just to customise workflows.

---

## 2. Goals

- **G1:** Workflow progression never invalidates the cached prompt prefix.
  The system prompt is static for the lifetime of a session.
- **G2:** Steps are reusable units: defined once, referenced by any number
  of templates.
- **G3:** Templates are individual files discovered dynamically from a
  folder; adding a template requires dropping in a file, not editing a
  central config or rebuilding.
- **G4:** Steps can be organised in freeform subfolders (e.g. `reviews/`,
  `git/`) that carry no semantic meaning to the engine.
- **G5:** One canonical on-disk location for workflow content; the
  triplicate-pinning tests collapse to checks against it.
- **G6:** Per-repo workflow customisation without duplicating provider
  config.

### Non-goals

- No changes to the engine's runtime state machine, the `workflow` tool
  protocol, UDS `workflow_state` events, persistence format, or the TUI.
  After load-time resolution, the engine sees exactly today's
  `WorkflowTemplate` with inlined steps.
- No recursive composition: step files cannot reference other files.
  Templates compose steps; steps are leaves.
- No mid-session template hot-reload (a `reload_templates` action is a
  trivial follow-up if wanted; explicitly out of scope here).
- No changes to `--workflow-spec` bound-mode semantics (by-value specs keep
  working unchanged).

---

## 3. Design

### 3.1 Cache-safe prompting (fixes P1)

**Remove dynamic workflow state from the system prompt entirely.**

- The system prompt gains at most a short **static** preamble ("a
  development workflow may be active; use the `workflow` tool; call
  `workflow(action="status")` to see current state") — or nothing, since
  the tool description already covers discovery. The preamble never changes
  during a session.
- Delete the workflow `system_prompt_provider` closure in `agent.rs` and
  the `append_workflow_prompt*` helpers in `interface/shared.rs` (net code
  removal).
- Dynamic state is delivered through **append-only** channels that do not
  invalidate the prefix:

  | State | New delivery channel |
  |---|---|
  | Template selection menu | `workflow(action="list_templates")` tool result; under `--workflow`, the tool description advertises selection and the first idle nudge pushes it if the model starts without selecting (see Decision 1) |
  | Current step + guidance | Returned in the tool result of `check` / `select_template` / `status` — the model receives the *next* step's guidance exactly when it advances |
  | Progress / issue context | Same tool results; also already broadcast as `workflow_state` UDS events for UIs |
  | Idle-boundary pushes | Existing auto-continue / completion nudges (`uds_workflow_nudge.rs`) — already appended user messages, already cache-friendly; extend wording to carry current step + guidance |

- Guidance blobs (up to 3.3 KB/step) are thus sent **once per step** and
  then live in cacheable history, instead of being re-sent in a mutating
  prefix every turn. This is both the caching fix and a per-turn token
  reduction.

### 3.2 File-based template & step library (fixes P2, P3, G2–G6)

**On-disk layout** (discovery root configurable via `workflow.dir`):

```
.quecto/workflows/
├── feature.json                   # one template per file; id = filename stem
├── refactor.json
├── hotfix.json
└── steps/                         # shared step library, freeform organisation
    ├── reviews/
    │   ├── adversarial-reviewers.json
    │   └── finder-waves.json
    ├── git/
    │   ├── branch.json
    │   └── pr.json
    └── verify/
        └── full-test-suite.json
```

**Template file** — `steps` entries are a union of inline objects and path
references:

```json
{
  "label": "Feature",
  "description": "Full feature development cycle",
  "steps": [
    "steps/git/branch",
    { "key": "implement", "label": "Implement", "phase": "green", "guidance": "..." },
    "steps/reviews/adversarial-reviewers",
    { "ref": "steps/verify/full-test-suite", "phase": "verify" }
  ],
  "guards": [ { "pattern": "git push", "message": "..." } ]
}
```

**Step file** — exactly one `WorkflowTemplateStep`:

```json
{
  "key": "adversarial_reviews",
  "label": "Adversarial reviewers",
  "phase": "review",
  "guidance": "Spawn N reviewer subagents with read_only: true ..."
}
```

**Step entry union** (`serde(untagged)`, ~40 lines):

1. **String** → path reference, relative to the workflow dir; `.json`
   extension optional.
2. **Object with `ref`** → path reference plus field overrides (`key`,
   `label`, `phase`, `guidance` each individually overridable). This is how
   a shared step is re-keyed or re-phased per template.
3. **Plain object** → inline step, exactly as today.

**Resolution rules:**

- Discovery and resolution happen **once, at engine construction**
  (`WorkflowEngine::new`). Only top-level `*.json` files in the workflow
  dir are templates; `steps/` and all subfolders are never scanned for
  templates. Template id = filename stem.
- Folder structure under the root is purely organisational; the engine
  never interprets it.
- After resolution, the in-memory model is today's `WorkflowTemplate` with
  fully inlined steps — nothing downstream changes.
- **Fail fast, loudly, at startup** with the offending file path:
  missing referenced file, unparseable JSON, **unknown fields**
  (`deny_unknown_fields` — typos are load errors, see Decision 2), a
  template with zero steps, and **duplicate step keys within a resolved
  template**. Duplicate keys
  are *not* auto-suffixed; reusing one step file twice in a template
  requires an explicit `{ "ref": ..., "key": "wave_2" }` override.
- No recursion: a step file that contains a `ref` is a load error.

**Config & precedence:**

- New config key `workflow.dir` (string path). Default resolution order:
  1. `workflow.dir` if set in config,
  2. `./.quecto/workflows` if it exists (repo-local, satisfies G6 — no
     credential duplication),
  3. `~/.quecto/workflows` if it exists,
  4. fall back to inline `workflow.templates` in config (kept working for
     backwards compatibility, tests, and `--workflow-spec`).
- If both a directory and inline templates are present, the directory wins
  and inline templates are ignored (with a startup warning), keeping the
  mental model simple: one source of truth per session.
- Canonical repo content moves from `workflow-config.json` into
  `quecto-agentic-harness/workflows/`. `workflow_config_template.rs`'s
  triplicate pinning is replaced by assertions against the canonical
  folder, and `examples/config.json` / `.claude/workflows/feature.js` are
  either deleted or generated from it.

---

## 4. Implementation plan (independently shippable slices)

| # | Slice | Contents | Risk |
|---|---|---|---|
| 1 | Step-entry union + path refs | `serde(untagged)` step entry, load-time resolution relative to the containing file's dir; works inside today's single config file first | Low — pure deserialization change; engine untouched |
| 2 | Directory discovery | `workflow.dir` key, precedence rules, loader building the template list from `*.json` files; migrate repo content into `workflows/`; collapse triplicate tests | Low — loader + test migration |
| 3 | Cache-safe prompting | Delete workflow system-prompt provider and `append_workflow_prompt*`; enrich tool results with next-step guidance; extend nudge wording; selector delivery via tool description + first idle nudge (Decision 1) | Medium — behavioural change to how the model receives workflow context; needs an e2e pass verifying models still follow workflows reliably, plus post-landing monitoring of template-selection compliance |

Slices 1–2 and slice 3 are orthogonal and can land in either order.

## 5. Acceptance criteria

1. With an active workflow, the rendered system prompt is byte-identical
   across turns of a session, including across `check` calls (test:
   snapshot the prompt before/after a check).
2. A step file referenced by two templates is defined exactly once on disk;
   editing it changes both templates on next agent start.
3. Dropping a new `foo.json` into the workflow dir makes template `foo`
   appear in `workflow(action="list_templates")` on the next session with
   no config edit.
4. A `ref` to a missing file, invalid step JSON, a recursive `ref`, or
   duplicate resolved step keys each fail agent startup with an error
   naming the offending file/key.
5. Existing configs with inline `workflow.templates` continue to load and
   behave identically (backwards compatibility).
6. `--workflow-spec` bound mode is unchanged.
7. Template guidance content exists in exactly one canonical repo location;
   drift tests assert against it alone.

## 6. Decisions (resolved)

1. **Selector delivery under `--workflow`: tool description + first idle
   nudge (no injected selector text).** The `workflow` tool's schema
   description advertises template selection; if the model starts executing
   without selecting a template, the existing nudge machinery pushes it at
   the first idle boundary. This is the purest cache-safe design with zero
   new mechanism. Known risk: turn one may run unguided with weaker models
   — we will **monitor behaviour** after landing and can add a one-time
   appended selector message later if compliance regresses (a small,
   backwards-compatible follow-up).
2. **Strict parsing (`deny_unknown_fields`) for step and template files; no
   version field.** Typos fail fast at startup (e.g. `"guidence"` is a load
   error, not silently ignored guidance). Trade-off accepted: files using a
   newer optional field require a current quecto — consistent with the
   fail-fast philosophy elsewhere in this PRD. Schema evolves via optional
   fields only; a version field can be introduced later as
   optional-with-default if a breaking change ever becomes necessary.
3. **1:1 migration only.** `feature` and `refactor` move into the new
   layout byte-for-byte at the *resolved template* level (shared steps
   factored into `steps/` files must resolve to identical templates,
   verified by the drift tests). No new templates ship in this change; the
   new structure makes them cheap follow-up PRs.
