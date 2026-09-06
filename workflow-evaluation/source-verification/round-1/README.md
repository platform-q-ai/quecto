# Authorized round-1 source update — verification evidence

Applied only six approved guidance replacements from immutable candidate manifest:
investigate/report; chore/check, handoff; bugfix/regression, handoff; feature/refine.
Refactor source block byte-identical to HEAD. No IDs, step keys/order, phases, labels,
guards, descriptions or selection metadata changed. No commit made. Candidate and
frozen-evaluator hashes rechecked unchanged. goal.md read before update.

Changed source:
- quecto-agentic-harness/src/domain/workflow/engine/templates.rs
- quecto-agentic-harness/src/domain/workflow/engine/templates_tests.rs

## Test-first sequence

1. Added three tests covering full typed candidate/source equality; strict candidate
   field preservation except omitted empty guards; full WorkflowSpec serialization
   roundtrip/size; active binding/reset preservation; valid structure and rejection
   of duplicate/blank/empty steps and unknown guard target. Phases nonblank, not enum
   constrained or required unique. Existing source-content test still checks metadata.
2. Initial test-authoring compile error used nonexistent `check_step`; corrected to
   existing `check`. Retained log/exit under `test-authoring-compile-error.*`. This is
   NOT the meaningful RED.
3. `cargo test -p quecto-agentic-harness --lib approved_candidates_`: meaningful RED,
   2 pass / 1 fail, exit 101: complete source template differs from approved investigate
   candidate. `red.log` retains the actual mismatch before source guidance changes.
4. Applied exactly candidate guidance. Same command GREEN: 3 pass, exit 0,
   `green-candidates.log`.

## Focused validation (all exit 0)

```
cargo test -p quecto-agentic-harness --lib approved_candidates_              # 3 pass
cargo test -p quecto-agentic-harness --lib built_in_default_templates_        # 3 pass
cargo test -p quecto-agentic-harness --lib domain::workflow::                 # 63 pass
cargo test -p quecto-agentic-harness --lib workflow_spec                      # 25 pass
cargo test -p quecto-agentic-harness --test workflow_config_refactor_template runtime_default_templates_are_generic_after_template_globalization # 1 pass
rustfmt --edition 2024 --check quecto-agentic-harness/src/domain/workflow/engine/templates.rs quecto-agentic-harness/src/domain/workflow/engine/templates_tests.rs
git diff --check
```

Counts overlap (candidate/built-in tests also run in domain filter), not 95 distinct
tests. Raw logs, exits, command list and changed-step inventory are retained here.
No worker/container/panel interactions or broad workspace test/build run performed.

## Limitations and packaging

Equality tests `include_str!` the approved round-1 JSON artifacts using
CARGO_MANIFEST_DIR-relative paths. Parent must retain/add those candidate files when
committing these tests; they are intentionally immutable approval snapshots, not a
runtime dependency. Future approved revisions must update the test fixture version
explicitly rather than silently changing historical round-1 artifacts. A standalone
crate package without workspace evaluation artifacts cannot compile these cfg(test)
fixtures; normal non-test library builds are unaffected. Parent can choose a dedicated
in-crate immutable fixture copy later if standalone test packaging is required.

Typed compiled-source equality proves this checkout matches the candidates; it does
not establish that the existing globally installed/parent binary has been rebuilt.
Bound repeats receive approved full candidate objects and do not rely on installed
built-in text. No new experiment scores or consistency results are claimed here.
