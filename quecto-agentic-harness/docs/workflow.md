# Workflow V2

The workflow subsystem is a **UDS-only**, native in-process runtime. In normal
UDS sessions it is available as a dormant tool so users can opt into a workflow
mid-conversation. With `--workflow`, it actively guides agents through
structured development cycles using configurable template libraries from the
first turn.

## Parent coordination with workflows

Workflows give children (and, when useful, the parent) structured sequencing,
verification, evidence gates, and review structure. Prefer attaching a workflow
when the work is multi-step and process-shaped — especially coding tasks —
rather than relying on free-form prose instructions alone.

### When workflow helps

Delegate with a workflow when one or more of these apply:

- the task benefits from ordered steps (RED → GREEN → review → PR, investigate
  → report, etc.);
- verification, evidence gates, or review structure should be observable;
- an available template already encodes the right process for the work shape;
- the parent needs auditable progress (`workflow_state`, `get_state` workflow
  snapshot) without repeating the child's investigation.

Keep pure clarification, synthesis, final judgment, and short low-context work
in the parent without a workflow. See the `subagents` doc
(`docs {"name":"subagents"}`) for parent-vs-child routing and non-blocking
result recovery.

### Choosing how to attach a workflow

| Approach | When to use |
|----------|-------------|
| Plain child `task` (no workflow) | Substantial but focused or exploratory work that does not need a prescribed multi-step process |
| `spawn` with `workflow: true` | Work is workflow-shaped; the child inspects templates in its configuration and selects the best match |
| Instruct the child to `select_template` | A specific existing template is clearly appropriate |
| `spawn` with `workflow_spec` | Child must follow an **exact**, observable, auditable sequence — bind the full template rather than relying on prose to enforce steps |
| Parent `workflow` tool (dormant or `--workflow`) | The current session itself should track steps (user asked for a checklist, or the parent is the executor) |

For coding tasks, **prefer delegating** to a child that runs one of the
repository templates rather than improvising the full process in the parent:

| Template id | Shape |
|-------------|--------|
| `feature` | Behaviour-adding or behaviour-changing work |
| `bugfix` | Repro-first fixes |
| `refactor` | Zero-behaviour-change restructures |
| `remove` | Staged removals |
| `chore` | Small maintenance, docs, tooling |
| `adversarial-review` | Read-only PR review |
| `investigate` | Read-only diagnosis |
| `flake-hunt` | Intermittent CI/test failures |
| `plan` | Execution plans |
| `prd` | Design docs / proposals |

Classify by **issue shape**, not by how large the change feels: zero
behaviour-change restructures → `refactor`; new or altered observable behaviour
(and most maintenance) → `feature` (or the more specific ids above when they
fit). If behaviour change is mixed with a pure refactor, split the work — do
not run a mixed issue on a single mismatched template.

### Binding vs free selection

- **`workflow: true`** — workflow tool available; child picks a template from
  its library (selector mode). Good when the parent wants process structure but
  trusts the child to choose.
- **`workflow_spec`** — parent hands a fully inlined template by value; child
  starts **bound** in Active mode and cannot select another template. Use when
  exact step adherence matters (review finders, fixed checklists, custom
  sequences not in the library).
- **Instruct `select_template`** — middle ground: child has the library, parent
  names the template in the task briefing.

### Parent responsibilities while a workflowed child runs

- Keep the parent available; do not re-run the child's investigation.
- Use **passive completion notes**; the removed `agent_cmd await` command must
  not be used.
- Recover the report with plain `agent_cmd get_messages` (omit/null `count` and `before`), then verify
  and synthesize — a child's workflow completion is input to the parent's
  judgment, not a substitute for it.
- Track progress via forwarded `workflow_state` events, `get_subagents`
  workflow snapshots, or occasional `get_state` — not tight polling loops.
- Spawn reviewers and other non-editing children with `read_only: true`.
- At the end of coordinated work, inspect `get_subagents_all` and clean up
  stragglers.

### Example: focused read-only review workflow (`workflow_spec`)

Parents can bind a small review template when a full library template is heavier
than needed:

```json
{
  "id": "focused-review",
  "label": "Focused Review",
  "description": "Read-only single-dimension review with evidence-backed findings.",
  "steps": [
    {
      "key": "scope",
      "label": "Confirm assigned scope and review dimension",
      "phase": "review",
      "guidance": "Identify files, diff, commands, or documents in scope. Do not expand scope without evidence. Confirm this is a read-only review."
    },
    {
      "key": "inspect",
      "label": "Inspect the relevant code, tests, and docs",
      "phase": "review",
      "guidance": "Gather concrete evidence. Prefer file:line citations. Do not modify files or mutate local/remote state."
    },
    {
      "key": "analyze",
      "label": "Analyze only the assigned dimension",
      "phase": "review",
      "guidance": "Be skeptical. Report real, actionable issues only. Avoid style nitpicks unless they affect maintainability, correctness, security, or user outcomes."
    },
    {
      "key": "report",
      "label": "Return findings and confidence",
      "phase": "review",
      "guidance": "For each finding include severity, file:line when possible, problem, evidence, and a concrete fix. If no findings, say so explicitly and summarize what was checked."
    }
  ]
}
```

Spawn with `read_only: true` and `workflow_spec: { "template": { ... } }` so the
reviewer cannot use `write`/`edit` and must follow those steps. Remember
`read_only` is not a hard sandbox (`bash` remains).

## Architecture

- **UDS-only**: workflow availability requires `quecto agent --mode uds`
- **Not available** in REPL or one-shot (`agent -m`) mode
- **Dormant by default in UDS**: the workflow engine/tool/state are created, but
  selector-mode prompt text is not injected and the model is not pushed to start
  a workflow until a template is selected explicitly
- **Workflow-driven with `--workflow`**: selector/active workflow prompt text is
  injected from the first turn, so the model may choose and follow a template
  immediately
- **Fully disabled with `--no-workflow`**: no workflow engine/tool/state/prompt
- **Single built-in template model**: Quecto ships one repository workflow template (`feature`); custom configs may still define their own templates
- **In-process engine**: `WorkflowEngine` owns all state; the UDS bus is
  the external read/broadcast interface, not the coordinator
- **Core continuation**: auto-continue and completion nudges are generated by
  `quecto agent` itself after a template is selected. TUIs may toggle these
  settings, but they do not drive workflow progress.

## Startup flags

| Flag | Effect |
|------|--------|
| `--workflow` | Start in workflow-driven mode: the workflow tool is available and selector/active prompt text is injected immediately |
| `--no-workflow` | Fully disable workflow tool/state/prompt (clears `--workflow` and `--workflow-guards`) |
| `--workflow-guards` | Enable bash command guards when the workflow tool is available; does not by itself force workflow prompt injection |
| `--workflow-spec <path>` | Run the by-value template in the given spec file, **bound** in Active mode from the first turn (no template selection). Cannot be combined with `--no-workflow`. Usually set by the parent `spawn` tool, not by hand — see [Bound mode](#bound-mode) |

All flags require `--mode uds`. Using them without it produces an error.

### Typical invocation

```bash
# Conversational UDS agent: workflow tool/state available but dormant
quecto agent --mode uds -s my-session

# Later, ask the model explicitly:
# "Select the feature workflow and implement abc."

# Workflow-driven agent: prompt injection starts immediately
quecto agent --mode uds --workflow --workflow-guards -s my-session

# With a custom system prompt
quecto agent --mode uds --workflow --workflow-guards \
  --system "You are a senior engineer. Follow the workflow strictly." \
  -s feature-work

# Fully disable workflow if an integration must hide the tool entirely
quecto agent --mode uds --no-workflow -s my-session
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

> **Note:** `bash` commands run natively in the workspace and can reach
> `$HOME`, so tools like `gh` and `git push` work out of the box. To confine
> command execution (process, network, or resource limits), run Quecto inside
> a container.

### Minimal per-repo config example

A repo-local config that uses OpenAI with the Quecto workflow template:

```json
{
  "providers": {
    "openai": {
      "api_key": ""
    }
  },
  "agents": {
    "defaults": {
      "model": "openai/gpt-5.5",
      "max_context_tokens": 200000
    }
  },
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true,
    "selector_prompt": "Classify the issue by its SHAPE before selecting a template, then select exactly one: if the issue mandates zero behaviour change to code the project ships (refactor, consolidation, extraction, dedup, moving state, renames — acceptance criteria are structural/parity-only), select 'refactor'; for all other work — adding or altering observable behaviour, and maintenance work such as docs, CI, tooling or dependency changes — select 'feature'; if it mixes a behaviour change with a zero-behaviour-change refactor, STOP and report that the issue must be split — do not proceed on a mixed issue. Record which discriminator drove your classification.",
    "templates": [
      {
        "id": "feature",
        "label": "Feature",
        "description": "New capability with local hook verification, BDD/TDD, code review, then hand off the PR for human review (no auto-merge).",
        "when_to_use": "Use for behaviour-adding or behaviour-changing Quecto work (the issue's acceptance criteria describe new or altered observable behaviour) and for maintenance work such as docs, CI, tooling or dependency changes. Do NOT use for zero-behaviour-change code restructures — use 'refactor' for those.",
        "steps": [
          {
            "key": "hooks",
            "label": "Install/check local quality hooks",
            "phase": "setup",
            "guidance": "Run `scripts/install-hooks.sh`, then verify pre-commit, pre-push, and the git --no-verify wrapper are installed/active before editing code. Hook summary: `git commit` pre-commit runs staged-file hygiene, conditional BDD quality/tag checks, and `cargo fmt --check`; it does not run clippy or tests. `git push` pre-push runs fast quality rules, changed-package strict clippy, and architecture/repository invariants. Never bypass hooks with --no-verify. Done when all three hooks are installed and active."
          },
          {
            "key": "scenarios",
            "label": "Update Scenarios / Add new features",
            "phase": "red",
            "guidance": "First read the issue and comments with `gh issue view <N> --json title,body,comments`. Update BDD feature files and task-facing scenarios first, and identify explicit, checkable acceptance criteria for the change. BDD selection is by tag + sharding (no scenario-name filter unless implemented). The acceptance criteria identified here are the SAME checklist the later `conformance` step verifies against branch code, so make them concrete and testable. Write feature files to behaviourally-led BDD best practice: keep scenarios DECLARATIVE and behaviour-focused (describe the observable outcome from the user/stakeholder view, never implementation detail, UI selectors or incidental wiring); follow strict Given-When-Then discipline (Given = context/preconditions, When = the single triggering action, Then = the observable outcome) with one behaviour per scenario and one logical When; use ubiquitous/domain language so scenarios read as intent; do not smuggle multiple actions or assertions into conjunctive `And` steps; write reusable, well-abstracted, consistently-phrased steps; use Background only for genuinely shared context and Scenario Outline only for true data variations; scenario titles state the behaviour/intent (not \"test X works\"); and ensure every acceptance criterion maps to a scenario. Done when the scenarios are updated and the acceptance criteria are written down."
          },
          {
            "key": "tests",
            "label": "Write/update unit tests (run a quick smoke check; full CI runs after `merge-requested`)",
            "phase": "red",
            "guidance": "Write or update unit tests for the change. Use these crate names: kernel `cargo test -p quecto-agentic-harness --lib <name_substring>` (the package is `quecto-agentic-harness`; its lib target is named `quecto`, but `-p` takes the PACKAGE name); TUI `cargo test -p quecto-tui --lib <name_substring>`. Plain lib tests need no `--features`; render-harness-driven TUI BDD uses the integration `bdd` target with `--features test-harness`. Run a quick targeted smoke check to confirm tests compile; the full suite and coverage run on push. Write step tests and unit tests to the same discipline: assertions are BEHAVIOURAL (verify observable outcomes/contracts, not internal or private state), well-named, focused, DETERMINISTIC and ISOLATED (no cross-scenario state leakage or ambient/global mutation), and never HOLLOW/always-pass \u2014 each test must be able to fail, and must fail before the implementation exists (RED). Done when the new/modified tests compile and target the new behaviour."
          },
          {
            "key": "red",
            "label": "Ensure new/modified tests FAIL (RED) \u2014 quick targeted run only, not full suite",
            "phase": "red",
            "guidance": "Run a quick targeted check: only the new/modified targeted test to confirm it fails before implementation: `cargo test -p quecto-agentic-harness --lib <name_substring>` or `cargo test -p quecto-tui --lib <name_substring>`. Do not run the whole suite here. RED evidence is required per new Then step and per new test assertion, not per test target: every new assertion must individually be shown to fail before implementation \u2014 a tautology cannot produce that evidence, so it is rejected by construction. Done when the new tests fail for the expected reason. If it fails to fail \u2014 i.e. a new test passes before implementation \u2014 it is not exercising the new behaviour, so fix the test."
          },
          {
            "key": "bdd_review",
            "label": "Despatch three BDD review finders (Gherkin discipline, Falsifiability, Coverage)",
            "phase": "review",
            "guidance": "Dispatch three narrow BDD review finders in parallel, each bound to its OWN dedicated review workflow via `workflow_spec`/`template` (never this feature workflow), launched read-only: pass `read_only: true` to `spawn` (equivalent to `disable_tools: [\"write\", \"edit\"]`) so it cannot write or edit repo files while it retains `bash`, `read`, `grep`, `find` and `agent_cmd` to inspect the changes (defense-in-depth, not a hard sandbox, since bash remains). Spawned reviewers produce a one-line completion note at your next idle turn; read findings with `get_messages`. Give each finder the issue details, the changed BDD feature files, step tests and unit tests, and ONE angle: (1) Gherkin discipline \u2014 STRICT and uncompromising on BDD best practice: because BDD quality is foundational it must flag EVERY genuine best-practice deviation, not just egregious ones (while staying skeptical: report only real issues, never invalid or hallucinated findings), explicitly checking the checklist: DECLARATIVE/behavioural phrasing with no implementation detail in steps, strict Given-When-Then discipline, one behaviour per scenario, ubiquitous/domain language, reusable well-abstracted steps. (2) Falsifiability \u2014 for each assertion, name the implementation change that would make it fail; flag self-asserted (test-constructed) state, constant comparisons and type-level facts. (3) Coverage \u2014 full acceptance-criterion-to-scenario mapping plus boundary pinning: both sides of every numeric/size limit must be tested. There is no verify wave \u2014 the scope is small. Each finding must quote the offending line and give a concrete fix, plus file:line and severity. Finders must NOT modify code. Then address EVERY valid concern regardless of severity, not just high-priority ones: fix it, or explicitly DECLINE it with a documented rationale (reviewers can be wrong). All valid BDD concerns must be resolved before the GREEN step. Done when all three finders have returned and every valid concern is fixed or declined with rationale."
          },
          {
            "key": "green",
            "label": "Implement code (GREEN)",
            "phase": "green",
            "guidance": "Write the code needed to satisfy the failing tests. Do NOT worry about the size of a change \u2014 implement it in full. Done when the previously-failing tests pass."
          },
          {
            "key": "refactor",
            "label": "Refactor",
            "phase": "refactor",
            "guidance": "Tidy only what this change touches \u2014 naming, duplication, clarity. Keep it minimal; no speculative abstraction or unrelated cleanup."
          },
          {
            "key": "verify",
            "label": "Ensure tests still pass",
            "phase": "green",
            "guidance": "Run `cargo fmt` and targeted strict clippy locally first (for kernel: `cargo clippy -p quecto --all-targets -- -D warnings`), then re-run only the targeted tests and confirm GREEN. Do not manually re-run the whole suite before commit; authoritative CI after `merge-requested` is the full-suite mechanism. Respect the 750-line file cap and strict clippy before pushing. Done when targeted tests pass and fmt/clippy are clean. If it fails, fix the code \u2014 never silence clippy \u2014 before pushing."
          },
          {
            "key": "version_bump",
            "label": "Bump semver for every changed crate and sync version docs",
            "phase": "ci_cd",
            "guidance": "Bump the semver of every crate whose source changed in this PR before committing, so the bump goes through the same gate as the code. Determine which crates you actually modified (e.g. `git diff --name-only master...HEAD` plus unstaged changes) and bump the PATCH version in each such crate's Cargo.toml (MINOR for a notable feature). Do NOT bump crates you did not change. Keep version docs in lockstep: for the `quecto` kernel, update the README.md 'Current version: **x.y.z**' line and the matching assertion in `quecto-agentic-harness/tests/features/repo_docs.feature` so both equal the new version; for `quecto-tui`, update its Cargo.toml and any TUI version assertion; for any other crate, update whatever doc or test pins its version. Done when every changed crate's Cargo.toml and its version docs show the new, matching version and no unchanged crate was bumped."
          },
          {
            "key": "commit",
            "label": "Commit",
            "phase": "ci_cd",
            "guidance": "If on the default branch, create a feature branch first. Stage only the files you changed (`git add <paths>`); never `git add -A` or `git add .` \u2014 that sweeps untracked tooling (e.g. `.claude/`) and build artifacts (e.g. `target*/`) into the commit. Remember `git commit` runs the pre-commit structure/lint/fast-guard gate, not unit/BDD feature tests. Write a clear, descriptive commit message and include any commit trailers your harness requires. Done when only the intended files are staged and committed on a feature branch."
          },
          {
            "key": "push",
            "label": "Push through the fast pre-push gate",
            "phase": "ci_cd",
            "guidance": "Push in the foreground and wait for the fast pre-push gate: repository/BDD quality rules, formatting, strict Clippy for changed packages, and architecture/contract/repository invariants. Full tests, BDD, coverage, dependency policy, and mock E2E do not run on ordinary pushes; they run only after `merge-requested` is applied. If it fails, fix the reported issue and push again; never bypass the hook. The same gate can be run directly with `scripts/pre-push.sh`. Done when the latest push exits successfully."
          },
          {
            "key": "pr",
            "label": "Create PR",
            "phase": "ci_cd",
            "guidance": "Open the PR against the default branch with gh, with a clear title and a body that summarizes the change. Do not claim to co-author it. Opening or updating the PR does not start full CI. Done when the PR is open; request CI only after review is complete."
          },
          {
            "key": "reviewers",
            "label": "Despatch narrow parallel review finders, verify adversarially, post one review",
            "phase": "review",
            "guidance": "Precondition: reviewers MUST NOT be dispatched before a PR exists; if no PR number is available, stop and complete the Create PR step first. Use `gh pr view <PR> --json id,headRefOid,state,mergeStateStatus` and `gh pr diff <PR>`. The review runs as three waves. Wave 1 \u2014 narrow parallel finders: dispatch ALL default finder angles in a SINGLE parallel batch, each spawned read-only (`read_only: true`, i.e. `disable_tools: [\"write\", \"edit\"]`, so a finder cannot write or edit repo files but retains `bash`, `read`, `grep`, `find` and `agent_cmd` to fetch the diff and inspect the code; defense-in-depth, not a hard sandbox) and bound to its OWN `review-pr` workflow, given only the PR number, head commit SHA and ONE mechanical angle. Explicitly forbid passing a raw diff: finders must fetch the diff themselves from the PR number. Finders make no GitHub writes \u2014 they never post to GitHub; they only return structured findings. Default angles: line-by-line hunk scan seeded with diff-specific failure classes; Removed-behavior audit (for every deleted/replaced line, name the invariant it enforced and where the new code re-establishes it); Cross-file tracer (callers/callees/consumers of every changed symbol); Security (injection, path traversal, secret leakage, permission gaps introduced by the diff); Performance/efficiency; Reuse + altitude (does new code re-implement an existing helper; same-defect-class grep \u2014 does the defect being fixed exist elsewhere in the codebase; bandaid vs. mechanism); Clean architecture (layering and dependency direction respected; shared logic at the right altitude \u2014 a quirk or special case preserved for compatibility lives inside the shared mechanism, never as caller-side state in a consumer; no production code path that exists only for tests, e.g. cfg(test) forks or shims; every new public API item has a consumer outside its own module \u2014 flag speculative surface); Test falsifiability (for each new/changed test, name the implementation change that would make it fail; reject assertions on test-constructed state, constants, or type-guaranteed facts, and anything that would pass with the implementation reverted). Conformance to the issue acceptance criteria is NOT a finder angle \u2014 it needs whole-issue context and lives in the standalone conformance step. Each finding is structured as file:line, a one-line summary, and a concrete failure scenario (required \u2014 a finding without one is dropped). Wave 2 \u2014 adversarial verification: dedupe Wave 1 findings, then dispatch one read-only verifier per finding, in parallel, prompted to REFUTE it; each verdict is CONFIRMED, PLAUSIBLE or REFUTED and must quote the proving/disproving line. If Wave 1 returns no findings, skip Wave 2. Wave 3 \u2014 single post: the master posts exactly one submitted review via GitHub GraphQL (`addPullRequestReview` event COMMENT with comments array) carrying every surviving (non-REFUTED) finding as an INLINE review comment on the PR; if a line anchor is rejected, fall back to a review comment that still cites file:line. When multiple finders converge on the same line, treat that as a severity signal \u2014 note it on the finding. CRITICAL: SUBMIT the review, never leave it PENDING; verify submittedAt != null. Done when every surviving finding is posted inline in one submitted (non-PENDING) review on the PR."
          },
          {
            "key": "fix_reviews",
            "label": "Fix all valid review concerns",
            "phase": "review",
            "guidance": "Triage each inline finding \u2014 confirm it is genuinely valid before changing anything (reviewers can be wrong). Address EVERY valid concern regardless of severity \u2014 not just the high-priority ones: fix it in the same branch, or explicitly DECLINE it with a documented rationale. Track which findings you accept versus decline; you reply to all of them in the resolve step. Done when every valid finding regardless of severity has a fix in the branch or a documented decline."
          },
          {
            "key": "push_fixes",
            "label": "Push changes to remote",
            "phase": "review",
            "guidance": "Push the fixes; the fast pre-push gate runs again. Wait for it to pass before resolving threads. Any push removes a stale `merge-requested` label, so reapply it only after all review work is complete."
          },
          {
            "key": "resolve_threads",
            "label": "Reply to the reviewers comments on the PR and mark resolved (use graphql)",
            "phase": "review",
            "guidance": "Reply to EVERY review comment on the PR \u2014 for accepted findings note the fix and commit, for declined ones explain why \u2014 then resolve each thread with the GraphQL resolveReviewThread mutation. Thread ids come from the PR reviewThreads connection. Done when every review thread is resolved."
          },
          {
            "key": "conformance",
            "label": "Verify the PR meets every issue acceptance criterion",
            "phase": "review",
            "guidance": "Read the original issue and comments with `gh issue view <N> --json title,body,comments`, then systematically verify EACH acceptance criterion from the issue (the same checklist captured in `scenarios`) against the ACTUAL branch code, citing file:line evidence, including any documentation and protocol updates the criteria require. A criterion counts as met only when the evidence is in the code \u2014 never on the PR description claims; be skeptical. Output a per-criterion table marking each met/partial/unmet with evidence, then a final verdict line: `CONFORMANCE: PASS` only if every criterion is fully met, otherwise `CONFORMANCE: FAIL` listing gaps. On `CONFORMANCE: FAIL`, fix the gaps and re-verify before reporting \u2014 do NOT merge on a FAIL. Done when the verdict is `CONFORMANCE: PASS` with file:line evidence for every criterion."
          },
          {
            "key": "pre_merge",
            "label": "Request authoritative CI and report the PR (do not merge)",
            "phase": "ci_cd",
            "guidance": "Confirm the latest fast pre-push gate passed, all review comments are resolved, and the current head is final. Run `gh pr edit <n> --add-label merge-requested` to request authoritative CI for that exact revision, then wait with `gh pr checks <n> --watch`. If another commit is pushed, the label is removed automatically; finish review and reapply it only when ready. If CI fails, fix the failure, push, and request CI again. Do not merge or set auto-merge. Done when authoritative CI passes for the latest head and the PR is reported for human merge."
          },
          {
            "key": "cleanup",
            "label": "Clean up sub agents",
            "phase": "ci_cd",
            "guidance": "Reviewers that have already reported completion (you received their one-line completion notes) have exited and need no cleanup. Check `get_subagents` for any sub agent still running, and only then terminate the stragglers (use agent_cmd to abort, or kill each) so no orphaned sub agents remain."
          }
        ],
        "guards": [
          {
            "commands": [
              "git commit",
              "git push"
            ],
            "before_step_key": "commit",
            "message": "Complete hook setup and RED/GREEN work before committing."
          },
          {
            "commands": [
              "git merge",
              "gh pr merge"
            ],
            "before_step_key": "cleanup",
            "message": "This workflow does NOT merge \u2014 report the PR for review and stop. Before reporting, complete code review, pass the conformance gate (CONFORMANCE: PASS), and verify the pre-push gate genuinely passed (not bypassed with --no-verify). Never run gh pr merge / git merge or set auto-merge; report the PR as ready for review instead."
          }
        ]
      }
    ]
  },
  "tools": {
    "exec": {
      "isolation": "native"
    }
  }
}
```

Invoke with:

```bash
quecto agent --mode uds --workflow --workflow-guards \
  --config ./my-repo/.quecto/config.json -s feature-work
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
| `auto_continue` | boolean | `true` | Core agent nudges itself to continue with the next step after a workflow template is selected |
| `completion_nudge` | boolean | `true` | Core agent prompts itself to close/cycle when all selected-template steps are done |
| `selector_prompt` | string | null | Custom prompt shown during template selection |
| `templates` | array | `[]` | Custom template definitions (empty = use the single built-in Quecto workflow) |

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

### Reusable step references

A step entry can take three shapes:

| Shape | Example | Meaning |
|-------|---------|---------|
| Inline object | `{"key":"tests","label":"Write tests","phase":"red"}` | The step itself, as documented above |
| String reference | `"steps/shared"` | Load the step from a JSON file resolved relative to the config directory; `.json` is appended when the reference has no extension |
| Reference object | `{"ref":"steps/shared.json","phase":"review"}` | Load the referenced step, then apply the given `key` / `label` / `phase` / `guidance` overrides on top (overrides always win over the file's values) |

Referenced files must:

- stay inside the config directory — absolute paths, `..`, and symlink escapes are rejected
- be at most 64 KB
- contain a single complete step object using only the step fields above; unknown fields are rejected, and a referenced file cannot itself contain `ref` (no recursive references)

Backward compatibility: an inline step that already has `key`, `label`, and `phase` **and** carries fields outside the step-field table (e.g. an `owner` tag alongside a `ref` ticket marker) is kept as-is — the extra fields are ignored as metadata, not treated as a reference.

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

When `templates` is empty (or omitted), one default is loaded:

| ID | Label | Steps | Guards |
|----|-------|-------|--------|
| `feature` | Feature | 20-step planned Feature workflow (hooks → plan intake → semantic state-space → test design/review → RED → scoped implementation → refactor/harden → local review → verify → version bump → commit → push → PR → PR review/fixes → resolve threads → conformance → authoritative CI → cleanup) | `git commit`/`git push` before commit step; `git merge`/`gh pr merge` before cleanup step |

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
reminders. If `auto_continue` is enabled, the core agent runtime keeps nudging
itself through incomplete steps while workflow progress advances.

### Complete mode

All steps in the template are checked. If `completion_nudge` is enabled, the
core agent runtime nudges the agent to close the current issue and begin a new
cycle.

### Bound mode

When an agent is started with a by-value workflow assignment (`--workflow-spec`,
typically set by a parent's `spawn` `workflow_spec` — see the `subagents`
doc (`docs {"name":"subagents"}`)), the engine is **bound** to that single template:

- It starts directly in Active mode — no selector phase.
- `select_template` cannot switch to a different template, and `reset` only
  clears step progress (it never returns to Selector mode).
- On completion the nudge tells the agent to report its result and stop, rather
  than to reset and pick a new workflow.

This makes a parent's assignment authoritative: the child runs exactly the
workflow it was handed.

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
{"action": "select_template", "template": "feature", "issueNumber": 42, "issueTitle": "Login bug"}
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
- **Complete mode**: completion indicator and report-and-stop guidance

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

When a workflow template is selected, `get_state` includes only the slim
workflow identity and current step in its `workflow` field. When no template is
selected, or workflow is disabled with `--no-workflow`, the field is absent.
Full step lists, guidance, available templates, and automation details are not
part of `get_state`; use workflow commands/events for those details.

### `workflow_state` event

Emitted on template selection, step mutation, issue mutation, reset, and
completion transitions. Payload includes mode, progress, steps, active
template, active issue, and available templates.

When workflow is disabled with `--no-workflow`, no `workflow_state` events are emitted.

## Complete config examples

### Example 1: Built-in template with guards (simplest setup)

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

This uses the built-in templates — 'feature' for behavioural and
maintenance work, 'refactor' for zero-behaviour-change restructures —
routed by the classifier selector prompt, with their default guard
rules. The empty `workflow: {}` uses all defaults.

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
prompt state once workflow prompting is active, but no bash commands are blocked. Useful for advisory workflows
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

## Dormant workflow vs disabling workflow completely

Workflow is **available by default** in UDS mode. If you do not pass
`--workflow`, the `workflow` tool is registered, but `get_state` still omits
workflow state until a template is selected. The model is not shown
selector-mode instructions and is not pushed to start a workflow. The user may
explicitly ask the model to select a template later, for example: "select the
feature workflow and implement abc".

```bash
# Dormant workflow tool available — standard conversational UDS agent
quecto agent --mode uds -s my-session
```

In dormant mode:

- A `WorkflowEngine` is created
- The `workflow` tool is registered and may be called if the user asks for it
- Selector-mode workflow prompt text is not injected
- Once a template is selected, active workflow prompt context is injected on
  later turns
- No auto-continue or completion nudge starts a workflow by itself

Use `--no-workflow` only when workflow must be completely unavailable:

```bash
# No workflow tool/state at all
quecto agent --mode uds --no-workflow -s my-session
```

When workflow is disabled with `--no-workflow`:

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

`--no-workflow` clears both `--workflow` and `--workflow-guards` during parsing.
Use it when a wrapper or alias would otherwise expose workflow and you need the
model to have no workflow tool at all.

### Disabling guards only

To use workflow-driven prompting without command blocking:

```bash
# Workflow enabled, guards disabled
quecto agent --mode uds --workflow -s my-session
```

Simply omit `--workflow-guards`. The agent tracks progress and injects prompt
state once workflow prompting is active, but no bash commands are intercepted. Templates may still define guard
rules in their config — they are simply not enforced at runtime.
