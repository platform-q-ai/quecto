# Source-update validation plan (not executed)

Scope: later approved edits to `quecto-agentic-harness/src/domain/workflow/engine/templates.rs`.
No source/frozen evaluator changes or Cargo builds performed for this plan. Read
`goal.md` before the update. Keep baseline snapshots immutable; candidate artifacts
get new versioned paths. Only source-update after the goal's qualifying-run condition.

## Focused existing tests

Run from repository root AFTER an authorized source update; retain stdout/stderr,
exit codes, source commit/diff and candidate hash. Cargo still compiles the selected
library/test target if needed; these are filters, not promises of zero build cost.
First list matching tests if a command reports zero tests; zero is not a pass.

```sh
# Direct shipped-library shape/content/engine validation (3 tests).
cargo test -p quecto-agentic-harness --lib built_in_default_templates_
# Default fallback and existing generic-content safeguard.
cargo test -p quecto-agentic-harness --lib default_config_uses_bundled_templates_when_templates_empty
cargo test -p quecto-agentic-harness --lib bundled_templates_are_generic
# Engine validation/progress/guards/binding behavior using small explicit fixtures.
cargo test -p quecto-agentic-harness --lib domain::workflow::
# By-value spawn parse/active binding/invalid & oversized spec coverage.
cargo test -p quecto-agentic-harness --lib workflow_spec
# One directly relevant integration test (not the entire legacy fixture suite).
cargo test -p quecto-agentic-harness --test workflow_config_refactor_template runtime_default_templates_are_generic_after_template_globalization
```

Relevant source locations:
- `src/domain/workflow/engine/templates_tests.rs`: exact five IDs, engine accepts
  defaults, useful nonempty metadata/labels/phases/guidance. Does not fix step counts
  or wording; generally should survive legitimate guidance/step revisions unchanged.
- `src/domain/workflow_tests.rs`: default fallback plus generic-content test;
  validation cases for nonblank IDs/keys, duplicates, step/guard relationships.
  Most other tests use their own `planned_feature_template()`; do not rewrite those
  synthetic fixtures to imitate each new built-in.
- `src/interface/cli/agent_workflow_spec_tests.rs`: active binding, invalid/missing/
  oversized spec handling. Existing active-binding test asserts ID/mode/bound state,
  **not full content equality**; supplement with candidate equality below.
- `src/infrastructure/tools/tests/spawn_tests.rs`: by-value parsing and
  `workflow_spec_seeds_binding_before_first_monitor_event`. `workflow_spec` filter
  also selects related load/size/binding tests elsewhere in the library.
- `tests/workflow_config_refactor_template.rs:493`: runtime-default generic safeguard.
  Earlier tests there, `tests/workflow_config_template.rs` and `tests/workflow_docs.rs`
  mostly exercise `tests/common` canonical project-specific fixtures, not built-ins.
  Their fixed step counts/prose assumptions are not grounds for reshaping built-ins.
- `src/infrastructure/tools/workflow_tool*_tests.rs`: mostly explicit small template
  fixtures. Only add this filter if handoff/progression behavior is implicated:
  `cargo test -p quecto-agentic-harness --lib infrastructure::tools::workflow_tool`.

Existing generic-content tests use prohibited prose substrings; retain existing
safeguards, but do not extend them into assertions that new advice contains particular
phrases. Judge neutrality substantively. Do not add arbitrary fixed step counts or
an invented phase enum. `phase` is an unrestricted string; require nonblank meaningful
phase metadata and exact candidate agreement, not unique phase values (repetition is
valid). Step KEYS and template IDs must be unique.

## Candidate → serialized assignment → engine → source equality

Proposed focused new validation test (later source owner implements if desired), or
small trusted evaluator check linked against the library; not regex extraction of
Rust prose and not a new frozen task/oracle requirement:

1. Read the exact voted candidate JSON from its immutable versioned artifact. Normalize
   only its wrapper: `WorkflowSpec { template }` versus standalone `WorkflowTemplate`.
   Deserialize into domain types; use `WorkflowEngine::new` for semantic validation:
   1–100 steps, unique nonblank keys, guard target exists; library <=32, unique IDs.
   Require nonblank label/description/phase/guidance for shipped content. Empty guards
   remain the pilot condition unless a separately approved experiment changes them.
2. Reject unexpected candidate authoring fields or compare raw JSON to canonical
   serialized domain JSON so misspellings cannot silently disappear under permissive
   Serde parsing. Normalize only declared optional defaults (missing/null optional
   text, omitted/empty guards); do not ignore a lost required or unknown field.
3. Serialize `WorkflowSpec { template: candidate.clone() }` to bytes, assert length
   <= `MAX_WORKFLOW_SPEC_BYTES` (256 KiB), deserialize, and `assert_eq!` typed spec.
   This preserves ordered steps, phase/guidance and guards, while ignoring irrelevant
   JSON whitespace/key order. Record exact spawn-spec byte SHA-256 separately.
4. Initialize engine from the deserialized template, select it and bind as runtime
   does. Assert Active, bound, one template, and
   `engine.active_template() == Some(&candidate)`, not just matching ID/step count.
   Check another ID cannot be selected; reset remains bound/active with progress reset.
5. After approved source edit, call compiled `default_templates()`, find candidate ID,
   and `assert_eq!(source_template, &candidate)` for the WHOLE domain object. Compare
   all other built-ins to pre-update snapshots to catch unintended adjacent edits.
   Exact full-object equality here validates the approved artifact—not brittle prose
   fragments or superficial claims about behavior.
6. Run the focused tests above; inspect the source diff for only approved template
   changes. Any startup payload canonicalization must round-trip to the same object.
   Preserve raw candidate/spec/source hashes plus test outputs. When the worker binary
   is later rebuilt/deployed, record its identity; passing source tests alone does not
   prove a previously running binary includes the update. Runtime guidance evidence
   should continue to corroborate the assignment.

## Secret scan performed during this planning task

Static scan confined to `workflow-evaluation/`: 273 files examined for known API/Git
credential formats, JWT/bearer/private-key patterns and populated secret-like JSON
keys; symlinks flagged for review. **No concern paths found.** No matched values or
credentials printed or retained. See `secret-scan-report.json`. This heuristic check
is not a guarantee against arbitrary/encoded secrets. Credential-path mentions and
field names alone are not disclosures. No external credential store was inspected.
