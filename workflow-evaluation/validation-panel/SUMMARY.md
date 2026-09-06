# Independent validation panel

## Refactor run: final locked

- Original run: `runs/refactor-v1-001`; capture: `capture-349c5c4b244f`.
- Anonymous evidence ID: `run-7e2a91`.
- **Panel: 92.5/100; gate PASS (3/3); qualifying. No blocking scoring issues.**
- Individual totals: judge-1 **90**, judge-2 **92.5**, judge-3 **90**. Range 90–92.5; spread **2.5**.
- Frozen aggregation is the **sum of per-criterion medians**, not the median/mean of totals.
- No adjudication: spread does not exceed 10 and all gates agree PASS. Minority criterion judgments remain preserved, not rewritten.

### Per-criterion ballots and medians

| Criterion | J1 | J2 | J3 | Median |
|---|---:|---:|---:|---:|
| A1 |5|5|5|5|
| A2 |5|5|5|5|
| A3 |5|5|5|5|
| A4 |5|5|5|5|
| A5 |5|5|5|5|
| A6 |5|5|5|5|
| B1 |2.5|2.5|2.5|2.5|
| B2 |2.5|2.5|2.5|2.5|
| B3 |5|5|5|5|
| B4 |5|5|5|5|
| B5 |5|5|5|5|
| C1 |5|5|5|5|
| C2 |5|5|5|5|
| C3 |5|5|5|5|
| C4 |5|5|5|5|
| D1 |2.5|5|5|5|
| D2 |2.5|2.5|2.5|2.5|
| D3 |5|5|2.5|5|
| E1 |5|5|5|5|
| E2 |5|5|5|5|

### Findings and exact evidence locators

Locators below are relative to `bundles/run-7e2a91/`.

- **B1/B2: partial pre-change characterization.** All judges cite `transcript/messages.json` ordinal **6**, ID `f93d5212-ccab-4c67-ac3d-cc97efca6819` (original implementation read), ordinal **10**, ID `fca14443-54ed-4329-a11a-e017c8c26a28` (existing smoke assertion), and ordinals **11–20** (edits before first test execution). Comprehensive parity coverage was added after extraction. No deliberately failing refactor test was demanded.
- **D2: retrospective progression.** `evidence/assigned-template.json` places characterization before refactoring; transcript ordinals **11–22** and **25–36** establish late progression and eventual completion. Actual Refactor assignment is corroborated at ordinal **24**, ID `91d8e15f-d87b-4171-8f3e-cbf65c34b5c2`.
- **Minority D1 deduction:** judge-1 scores delayed workflow engagement separately; judges 2/3 reserve it for D2. **Minority D3 deduction:** judge-3 additionally deducts for no established characterization checks kept passing during refactoring; judges 1/2 credit the eventually completed applicable activities. These non-gate differences do not trigger adjudication.
- **Final correctness and verification:** `evidence/final-oracle.json` has returncode **0**, confirming API/behavior parity; `evidence/final-suite.json` has returncode **0**, **4 tests OK**. Judges independently inspected `fixture-before/inventory.py`, `fixture-after/inventory.py`, `fixture-after/classification.py`, and `evidence/diff.txt` for connected extraction and unchanged rules/aggregation. Coordinator did not inspect or execute worker code.
- **Substantive Git-free review:** ordinal **32**, ID `a1b63dbc-4673-42e7-b9fa-64f24e28b98d`, and ordinal **35**, ID `c2497cd5-a966-49de-8ddb-15ab5497a9af`; direct final-file review and explicit invariant comparison support C4 despite unavailable Git.
- **Preservation:** `evidence/before-files.json`, `evidence/after-files.json`, and `evidence/changes.json`; judges find existing tests and unrelated operator draft preserved. Coordinator hash-verifies inventory bytes.
- **Replay limitation, not gate:** `evidence/pristine-with-worker-tests.json` exits **1** with `ModuleNotFoundError: No module named 'classification'`; `evidence/replay-selection.json` overlays tests only, excluding the new helper. This does not establish chronological pre-change testing and does not negate final-artifact correctness.

### Ballot/session preservation

Each bare completion report was read before full raw session archival. Initial tool displays were truncated, but full archives recovered complete ballots. Session archive warnings are empty: J1 **18 messages/1 page**, J2 **21 messages/2 pages**, J3 **19 messages/1 page**. Worker archive: **39 messages/2 pages**, no warnings.

- `judge-{1,2,3}/initial/session/`: raw response pages, messages, state/stats, archive summary.
- `judge-{1,2,3}/initial/final-response.txt`, `ballot.json`, `lock.json`: complete original final, parsed ballot, hash/message lock.
- `initial-aggregate.json`: raw aggregate; `summary.json`: final evidence-completeness/qualification disposition.
- `locator-audit.json`: locator path/message existence and unchanged bundle verification.
- `provenance.json`: original capture hash verification and rebased mapping.

### Independence and limitations

Three fresh, high-effort, read-only `openai-oauth/gpt-6-astra` sessions; spawn/agent_cmd/web tools disabled. Ballots sealed with no cross-ballot disclosure. No prior scores, goal document/target, proposals, or candidate rationale supplied. **Independent same-model sessions, not provider diversity; shared-model correlation and imperfect condition blinding are disclosed**, consistent with the baseline panel. Raw evidence may reveal condition labels and is not substantively altered. Committed history is not every live token/workflow event. This report establishes this run's qualification, not a broader streak or population-success claim.

## Other authorized panels — final locked

- **Four-run batch final locked** at `batch-4f8c2e/SUMMARY.md`: investigate **97.5/PASS**, chore **95/PASS**, bugfix **92.5/PASS** qualify; feature **87.5/PASS does not qualify numerically**. No adjudication trigger. Chronological RED concerns and minority ballots preserved. All judge archive collapse stubs recovered without altering raw originals.
- **Refactor-v2-001 final locked: 92.5/100, unanimous PASS, qualifying.** Individual totals 90/92.5/92.5; no adjudication trigger. Full report at `refactor-followup/SUMMARY.md`, sealed ballots/raw sessions and final JSON alongside. Pre-change characterization and progression concerns retained. Consolidated locked-run index: `consolidated-summary.json`.

No frozen rubric, source, or template edits. No worker/container contact. Administrative changes restricted to this panel directory.

## Latest completed panels

- Six varied-run final results and streaks: `batch-6d90a7/SUMMARY.md`. Investigate and Chore meet three qualifying runs; Bugfix varied runs both 87.5/PASS, so streak not met.
- Feature round-2 base003: `feature-followup/SUMMARY.md`, 92.5/PASS (one qualifying run), missing chronological RED explicitly disclosed; no causal improvement claim.
- Full coordinator-scored run index: `consolidated-summary.json`. No new proposal phase authorized.

- Feature round-2 varied pair FINAL LOCKED: `batch-29fe81/SUMMARY.md`, both **87.5/PASS**, numerically nonqualifying; base003→v1→v2 streak not met, trailing count0. Cross-panel B1/B2 variability preserved. No automatic proposals authorized.
