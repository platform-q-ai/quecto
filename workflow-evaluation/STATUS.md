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
| investigate | 97.5 / 97.5 / 97.5 | PASS | COMPLETE: 3 consecutive round-1 |
| chore | 95 / 97.5 / 97.5 | PASS | COMPLETE: 3 consecutive round-1 |
| bugfix | 92.5 / 87.5 / 87.5 | PASS gate, variants below target | 0; round-2 proposal/vote underway |
| feature | 92.5 round-2 base | PASS | 1; round-2 v1/v2 authorized |
| refactor | 100 / 92.5 / 92.5 | PASS | COMPLETE: 3 consecutive unchanged |

Aggregation: sum of per-criterion medians. All three judges voted PASS for each run; no frozen adjudication trigger fired. Evidence: panel/baseline-summary.json and panel/judge-*/initial-ballots.json.

## In progress

Investigate, chore, and refactor meet the consistency target. Six-run varied results are locked in validation-panel/batch-6d90a7/. Bugfix variants both failed the numeric threshold (87.5) despite correct final artifacts because pre-change reproduction was absent; round-2 proposal/vote cycle authorized. Feature round-2 base scored 92.5/PASS (feature-followup/summary.json), with chronology omissions and cross-panel anchor differences explicitly disclosed. Feature source update authorized; new round-2 v1/v2 trials use -002 run directories. All completed worker containers cleaned after evidence verification. Any template revision resets its streak.

Binding versus explicit baseline selection differs and is a possible execution-order confound; scores do not establish a causal template improvement.

## Limitations

Small deterministic Python-standard-library fixtures do not demonstrate universal engineering performance. Judge sessions use the same model family, each reviewing all five tasks. Baseline condition labels are visible; no fully blinded or independent-provider claim. Supplemental feature RED output was recovered post-run before container removal; transcript establishes execution chronology. Full launch/config/capture limitations are recorded in LAUNCH.md and panel-execution-ledger.json.
