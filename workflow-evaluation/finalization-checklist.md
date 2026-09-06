# Finalization checklist — prepare-only audit

Read goal.md again. This checklist is the only file changed in this task. No source
edits, agents, runtime queries, builds, scores or commits. Findings describe the
current on-disk snapshot; parent owns evolving scores and cleanup records.

## Feature round 2: wait for qualification AND parent authorization

- [ ] Do not update feature source until parent supplies locked score >=90, PASS
  gate and explicit authorization for round-2 candidate. Base-003 is underway per
  parent; no qualifying score inferred from votes or expected improvements.
- [x] Verified `candidates/round-2/feature.json` equals round-1 feature plus ONLY F1
  `test_design.guidance`, exactly `validation-panel/round-2-feature/selected-proposal.json`
  old→new. Votes recorded YES3/NO0/ABSTAIN0; no unresolved goal conflicts.
  Candidate SHA-256: `28a9cc688ab6e6b2dedc51b6913abb06d29f8ac0dcbfb0b167d176bb5200f2d3`.
- [ ] Preserve canonical round-1 and round-2 JSON and the five crate-local round-1
  fixtures unchanged. Add just one byte-identical fixture when authorized:
  `quecto-agentic-harness/tests/fixtures/workflow-approved-round-2/feature.json`.
  Assert its byte equality/SHA-256 against canonical round-2 feature and recheck
  historical round-1 fixture hashes. No duplicate copies of the other four required.
- [ ] Minimal current-fixture selection: rename test helper `approved_candidates()`
  to `current_approved_candidates()`; switch ONLY its feature include from round-1
  to round-2. Update the helper comment to explain mixed immutable version selection.
  Retain the existing three substantive test names/logic (or add `current_` to the
  names consistently if desired); do not create a manifest/fixture-loader framework.
  Round-1 feature remains historical evidence, not a current-source equality target.
- [ ] Before source edit, run `cargo test -p quecto-agentic-harness --lib approved_candidates_`
  and retain the meaningful RED: current candidate/source full typed equality differs
  at feature/test_design. Serialization/binding/schema checks should still pass.
- [ ] Replace ONLY feature/test_design guidance in `templates.rs` with F1 verbatim.
  Retain round-1 refine guidance and every other workflow unchanged. Verify whole-file
  equality against committed source plus this ONE exact substitution.
- [ ] Run same candidate filter GREEN, then `--lib domain::workflow::`, `--lib workflow_spec`,
  and `cargo package -p quecto-agentic-harness --list --allow-dirty`; confirm round-1
  and round-2 fixture paths included. Run rustfmt --check on owned files and git diff
  --check. Record exact commands/exits/logs under source-verification/round-2/.
  No broad workspace build needed. Full typed equality must cover all five CURRENT
  templates, ordered steps, metadata/guidance/guards and bound-spec roundtrip/reset.
- [ ] Parent commits approved source/tests/new fixture plus evidence. Existing installed
  binary need not be assumed rebuilt: bound workers receive full approved candidate.

## Status/results corrections (parent-owned, not edited here)

- [ ] `STATUS.md` table says “Panel baseline score” but investigate/chore/bugfix/feature
  cells show revised-base scores. Separate baseline versus current candidate run IDs,
  hashes, scores/gates and streaks; point each to its actual locked summary.
- [ ] Replace “v1/v2 underway” with “completed, captured and cleaned; panel scoring
  pending” for investigate/chore/bugfix (parent report and coordinator summary agree).
  Do not increment their streaks until final ballots/aggregation lock.
- [ ] Feature status is stale (“second revision proposal/vote underway”): F1 selection
  is locked and round-2 base-003 is underway. Preserve round-1 base-002 87.5/PASS as
  NONQUALIFYING and streak reset; no erasure or blending of candidate versions.
- [ ] Refactor remains three qualifiers: base 100, v1 92.5, v2 92.5, PASS. Same unchanged
  candidate hash across its run metadata. Do not force a cosmetic source revision.
- [ ] `validation-panel/consolidated-summary.json` currently locks six EARLIER results
  (refactor v1/v2 + four revised bases), not the six new investigate/chore/bugfix varied
  runs. Its “ALL_SIX_RUNS_FINAL_LOCKED” status must remain scoped; add an explicit
  overall results ledger or link a new summary after current scoring, not overwrite
  historical ballots/summary. `panel-execution-ledger.json` is baseline-only, not a
  comprehensive results/cleanup ledger. No separate all-round results file was found.
- [ ] STATUS aggregation text references only baseline ballots and says no trigger
  fired. Scope that sentence to the appropriate panel; point later rounds to their
  actual initial/final ballots, adjudications (if any), and consolidated summaries.

## Cleanup mapping gap: parent refactor runs

Six new varied runs have detailed cleanup.json and
`runs/round-1-qualified-v1-v2-coordinator-summary.json`: preserved archive hashes,
kill responses, runtime absence and recorded UTC. Earlier coordinator-owned revised
bases also have cleanup.json. Parent refactor v1/v2 have committed evidence but
**no run-local cleanup.json or authoritative launch-ref mapping found**:

| Run | UUID from archived state/socket | Runtime container from provision |
|---|---|---|
| refactor-v1-001 | 95c0229a-5aee-4b87-ac6f-82a02f004137 | quecto-env-4vx2od84Q3 |
| refactor-v2-001 | bb1d321a-e34b-4d8b-b825-1a4819dfe6c7 | quecto-env-u7r55uGYfn |

- [ ] Parent backfills those two cleanup/launch records from retained parent tool
  evidence, including session/owner, UUID, runtime name, returned C-ref, kill result
  and archive-before-cleanup provenance. C-refs are session-local: coordinator varied
  runs already use C6–C11, so do NOT infer refactor mapping from those numbers.
  Earlier parent message reported refactor-v1 as C6; reconcile with original response
  rather than treating it as globally unique. Refactor-v2 C-ref not established here.
- [ ] If original kill output/time cannot be recovered, record parent-reported cleanup
  and timestamp=null with explicit limitation. Capture `container_killed:false` means
  the helper did not kill; it neither disproves nor records later parent cleanup.
  Never invent a runtime recheck, successful kill record, or exact timestamp.

## Commit/evidence closure

- [x] Source/packaging tests committed in `6b539c64`; revised-base/refactor worker
  evidence committed in `4de969b2`. Refactor v1/v2 have 51/50 tracked files respectively.
- [ ] At audit, `validation-panel/` (including consolidated results, raw/refined ballots,
  feature F1 proposals/votes and recovery evidence) is UNTRACKED despite STATUS relying
  on it. Preserve/commit after parent review; do not declare all scoring evidence
  committed merely because worker artifacts were committed.
- [ ] Also untracked: `candidates/round-2/`; six investigate/chore/bugfix v1/v2 run trees;
  their preparation/launch/cleanup coordinator summaries; feature base-003 and prepared
  feature v1/v2 trees. Distinguish prepared-only variants from executed/scored runs.
  Add completed evidence and vote/candidate artifacts deliberately; retain live-run
  changes until finalized as appropriate. Do not sweep unrelated `.quecto/` into commit.
- [ ] For final report, reconcile each run UUID/container/candidate/task/score/gate and
  cleanup provenance, preserve failures and invalid launch attempts, verify artifact
  hashes and frozen inputs, and report exact three-run sequence per workflow. No
  completion claim until all required unchanged-candidate streaks actually qualify.
- [ ] Keep small-suite, same-model judges, partial blinding, binding/selection-mode,
  chronological worker-vs-evaluator replay, and installed-binary limitations explicit.

Ready to implement the single feature update and packaging-safe fixture switch once
qualification and parent authorization arrive; nothing has been staged or changed
outside this checklist by this task.
