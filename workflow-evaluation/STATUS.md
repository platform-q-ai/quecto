# Workflow improvement status

See ../goal.md (repository root goal.md) for the session objective.

## Completed

- Frozen baseline tasks and rubric, independently reviewed before execution.
- Five fresh-container low-effort baseline workers; full transcripts and artifacts retained.
- Independent verification and replay; all five acceptance oracles passed.
- All five baseline containers cleaned after evidence capture and hash verification.
- Three independent judge sessions scored each baseline; raw ballots preserved.

| Workflow | Latest locked score | Gate | Current candidate streak |
| --- | ---: | --- | --- |
| investigate | 97.5 revised base | PASS | 1 revised; v1/v2 archived, scoring underway |
| chore | 95 revised base | PASS | 1 revised; v1/v2 archived, scoring underway |
| bugfix | 92.5 revised base | PASS | 1 revised; v1/v2 archived, scoring underway |
| feature | 87.5 revised base | PASS gate, below target | 0; round-2 F1 approved; base-003 running |
| refactor | 100 baseline; 92.5 v1; 92.5 v2 | PASS | COMPLETE: 3 consecutive |

Aggregation: sum of per-criterion medians. All three judges voted PASS for each run; no frozen adjudication trigger fired. Evidence: panel/baseline-summary.json and panel/judge-*/initial-ballots.json.

## In progress

Round-1 proposals/votes locked: P3/P4/P6/P7/P8/P9 accepted; source and packaging-safe equality tests committed. Revised base panel results are in validation-panel/consolidated-summary.json. Investigate/chore/bugfix qualify; six fixed v1/v2 fresh-container runs archived/cleaned and independently scoring. Feature fell to 87.5 because worker chronological RED/prospective verification was absent; F1 was unanimously approved and round-2 base-003 is running, with no task coaching or rubric change. Refactor unchanged and complete with three consecutive qualifying tasks. All completed worker containers cleaned after capture/hash verification. Any future revision resets that workflow's streak.

Binding versus explicit baseline selection differs and is a possible execution-order confound; scores do not establish a causal template improvement.

## Limitations

Small deterministic Python-standard-library fixtures do not demonstrate universal engineering performance. Judge sessions use the same model family, each reviewing all five tasks. Baseline condition labels are visible; no fully blinded or independent-provider claim. Supplemental feature RED output was recovered post-run before container removal; transcript establishes execution chronology. Full launch/config/capture limitations are recorded in LAUNCH.md and panel-execution-ledger.json.
