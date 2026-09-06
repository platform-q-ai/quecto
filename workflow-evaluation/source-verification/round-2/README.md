# Authorized feature round-2 source update

Read goal.md and locked `validation-panel/feature-followup/{SUMMARY.md,summary.json}`.
Parent authorized source update after feature-base-003 qualified 92.5/PASS3. This is
an attributed locked panel result, not a score assigned by this source task.

Applied ONLY F1's exact feature/test_design guidance from
`validation-panel/round-2-feature/selected-proposal.json`, agreeing exactly with
`candidates/round-2/feature.json`. Whole source file verified equal to saved pre-edit
source plus one literal replacement. Feature refine and all other workflows unchanged.

Changed/additional crate files:
- src/domain/workflow/engine/templates.rs: one guidance string replacement.
- src/domain/workflow/engine/templates_tests.rs: rename helper to
  current_approved_candidates, explain mixed immutable versions, switch only feature
  include to round-2. Existing three test names and substantive checks unchanged.
- tests/fixtures/workflow-approved-round-2/feature.json: byte-identical canonical copy.
Historical round-1 fixtures and canonical candidates remain unchanged; hashes retained
in fixture-hashes.json and integrity.json. Frozen evaluator hashes reverified intact.

## Meaningful RED then GREEN

After switching fixture selection but BEFORE source edit:
`cargo test -p quecto-agentic-harness --lib approved_candidates_`
failed exit 101, 2 passed / 1 failed: full typed source-candidate mismatch at feature.
See red.log. No setup/compile failure misrepresented as this RED.

After single F1 guidance replacement, same command passed 3/3. Additional checks:
- `cargo test -p quecto-agentic-harness --lib domain::workflow::`: 63 passed.
- `cargo test -p quecto-agentic-harness --lib workflow_spec`: 25 passed.
- `cargo package -p quecto-agentic-harness --list --allow-dirty`: exit 0; all five
  historical round-1 fixtures and new feature round-2 fixture included.
- `rustfmt --edition 2024 --check` on both owned Rust files: exit 0.
- `git diff --check`: exit 0.
Exact commands/logs/exits in commands.json and sibling files. Test counts overlap.
No broad workspace build, extracted-package build, worker interaction or commit.

## Evidence limitations retained

The qualifying feature worker STILL did not run chronological pre-change executable
RED. Panel medians B2/D2/D3=2.5 reflect missed ordering and candid later recovery;
evaluator sensitivity replay and this source-test RED do not repair that worker's
history. Cross-panel score increase from 87.5 to 92.5 is not attributed solely to F1:
anchor interpretation/session variability and activation salience remain confounds,
as explicitly documented in locked feature-followup/SUMMARY.md. This source update
claims no new worker score, second/third qualifying run, or universal improvement.

Compiled source/current approved typed equality passes; installed parent binary not
claimed rebuilt. Bound experiments use complete candidate JSON. Parent owns commit
and subsequent task/score/cleanup lifecycle. Include the new crate fixture in commit.
