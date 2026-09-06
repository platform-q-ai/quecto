# Authorized bugfix round-2 update — exact F3

Read goal.md, selected-proposal.json and locked bugfix-followup/summary.json.
Parent authorized after bugfix-base-003 locked 100/PASS3, with actual chronological
RED reported by the independent panel. No scores assigned/revised by this task;
qualification does not itself prove causal improvement or a three-run streak.

Changes only:
- `quecto-agentic-harness/src/domain/workflow/engine/templates.rs`: exact F3
  bugfix/reproduce guidance from candidates/round-2/bugfix.json and
  validation-panel/round-2-bugfix/selected-proposal.json.
- `templates_tests.rs` alongside it: switch current helper's bugfix include to round-2
  and update version-selection comment. Existing tests/substantive assertions unchanged.
- New `quecto-agentic-harness/tests/fixtures/workflow-approved-round-2/bugfix.json`,
  byte-identical canonical copy. All five round-1 and feature round-2 fixtures retained.

## Test-first evidence

After fixture include change BEFORE source edit:
`cargo test -p quecto-agentic-harness --lib approved_candidates_` → exit 101,
2 passed / 1 failed; complete source/candidate mismatch specifically bugfix.
Retained `red.log` and `red.exit`; meaningful mismatch, not a setup failure.

Applied the single literal F3 substitution. Same command → 3 passed, exit 0.
Additional commands, all exit 0 (exact argv in commands.json):
- `cargo test -p quecto-agentic-harness --lib domain::workflow::` → 63 passed.
- `cargo test -p quecto-agentic-harness --lib workflow_spec` → 25 passed.
- `cargo package -p quecto-agentic-harness --list --allow-dirty` → all seven immutable
  versioned fixtures included (5 round-1, feature/bugfix round-2).
- `rustfmt --edition 2024 --check` on templates.rs/templates_tests.rs.
- `git diff --check`.
Counts overlap. No broad workspace build or extracted-package test attempted.

Integrity: working templates.rs equals source-before.rs plus ONLY F3 guidance
replacement. Prior six crate-fixture hashes and frozen evaluator hashes unchanged;
new crate copy matches canonical candidate. See fixture-hashes.json and integrity.json.
Canonical candidates, selected proposal, other source and feature guidance preserved.

No commit, worker/container interaction, rubric modification or fabricated metrics.
Parent owns commit and remaining validation/cleanup. Installed agent binary not
claimed rebuilt; bound experiments use exact candidate objects. Include the new crate
fixture with the source/test commit. Raw logs retained in this directory.
