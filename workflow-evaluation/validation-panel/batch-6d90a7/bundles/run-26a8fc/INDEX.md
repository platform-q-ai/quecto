# Anonymous evidence bundle: run-26a8fc

## Read all substantive evidence
- `task.txt`: exact task contract; `evidence/acceptance.json`, `evidence/oracle.py`: frozen task acceptance/oracle.
- `evidence/RUBRIC.md`: frozen v1.0 criteria and gates.
- `evidence/assigned-template.json`: exact assigned template, bound at spawn; corroborate actual transcript guidance.
- `fixture-before/`, `fixture-after/`, `evidence/changes.json`, `evidence/diff.txt`: original and final artifacts and scope.
- `transcript/messages.json`, `transcript/page-*.json`: complete raw committed messages; cite `transcript/messages.json` by exact ordinal/message ID. No redundant chronological rendering is included.
- `evidence/transcript-index.json`, `evidence/worker-final.txt`: index and final handoff.
- `evidence/final-oracle.json`, `evidence/final-suite.json`, `evidence/pristine-with-worker-tests.json` and matching copy/allocation records: retained evaluator outcomes. Absent/inapplicable checks are documented by capture status, not presumed failures.
- `evidence/status.json`, `evidence/replay-selection.json` when present, `transcript/archive-summary.json`: limitations and warnings.
- `run-context.json`: execution settings and deviations.
- `source-map.json`: full original inventory mapping, capture-directory rebased to evidence/ with original hashes.
- `bundle-index.json`: rebased hash inventory of all bundled evidence.

## Interpretation and limits
Read-only and documentation task alternatives apply. The evaluator oracle is a floor, not the entire contract. Pristine replay is post-run corroboration, not proof of chronological worker RED; establish chronology from the transcript. Do not execute worker modules or contact containers.
Source-map entries preserve original hashes and bytes; substantive raw evidence is unchanged. No prior scores, goal document, rationale, proposals, revision history, or other judge ballots are supplied. Absolute paths in raw records can disclose condition labels: blinding is imperfect. Ignore such labels when scoring. Same model family across independent sealed sessions is not provider diversity.
Parent reports complete archival/hash verification followed by cleanup. Status records predate cleanup. Committed history is not every live workflow broadcast; inspect raw tool fields and archive warnings, and do not infer unseen execution. Missing metrics remain unknown.
