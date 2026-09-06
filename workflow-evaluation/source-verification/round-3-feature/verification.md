# Round 3 feature source verification

Read goal.md, approved round-3 candidate/manifest, and feature-base-fresh panel summary, provenance and ballots. Candidate and new immutable crate-local fixture SHA256: `da0fb1e5b36e933a60c4d7b1d532e7cd1b375719985469f08eb558aebce8bb43`.

Qualification: feature-base-004 is the approved first qualifying run. Recomputed sum of per-criterion medians from the three ballots: **90/100**, all gates PASS; individual totals 87.5, 87.5, 90 (not a median-of-totals qualification). Provenance records the assigned-template hash above. This does not establish causal improvement or three consecutive fresh qualifying runs. Panel limitations include same-model-family judging, late workflow engagement, missing worker chronological RED and incomplete worker scope review.

Changes: added `quecto-agentic-harness/tests/fixtures/workflow-approved-round-3/feature.json` byte-for-byte; switched current feature equality include from round 2 to round 3; replaced only feature intake guidance with exact approved text. All other source bytes, including the sixth adversarial-review workflow, preserved. Historical round-1/round-2 and adversarial-review fixtures unchanged against HEAD.

Executed in order:
- RED: `cargo test -p quecto-agentic-harness --lib approved_candidates_match_complete_source_templates` after fixture/include update, before source replacement: exit 101, 1 failed. Assertion specifically reported `source differs from approved feature`: old intake versus approved pre-change planning guidance; no setup failure.
- GREEN: `cargo test -p quecto-agentic-harness --lib domain::workflow::engine::templates::tests`: exit 0, 7 passed. Covers complete candidate equality, schema/engine validation, invalid-key rejection, serialization and bound-template/reset preservation, six-workflow inventory, and adversarial-review contract/order.
- `cargo fmt --all -- --check`: exit 0.
- `cargo package -p quecto-agentic-harness --list --allow-dirty`: exit 0; lists new round-3 fixture, all historical round-1/round-2 fixtures and adversarial-review fixture. Warning: manifest lacks documentation/homepage/repository metadata.
- Python hash, ballot-median and exact intake-only HEAD comparison assertions: passed. Historical fixture diff empty.
- `git diff --check`: passed.

Limitations: focused library tests only; no full suite, package build/install, new worker experiment or panel rerun. Package listing is not package compilation. Existing untracked evaluation/session artifacts left untouched. No commits or workers. Evaluation edits confined to this verification directory; goal's broader consistency target remains unclaimed.
