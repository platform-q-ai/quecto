# Revised-base batch — final locked

**Three qualifying runs; feature numerically nonqualifying. All gates unanimously PASS.**

| Run | J1 | J2 | J3 | Sum of criterion medians | Gate | Result |
|---|---:|---:|---:|---:|---|---|
| investigate-base-002 | 97.5 | 95 | 97.5 | **97.5** | PASS 3/3 | QUALIFYING |
| chore-base-002 | 95 | 92.5 | 95 | **95.0** | PASS 3/3 | QUALIFYING |
| bugfix-base-002 | 92.5 | 87.5 | 92.5 | **92.5** | PASS 3/3 | QUALIFYING |
| feature-base-002 | 87.5 | 85 | 90 | **87.5** | PASS 3/3 | NONQUALIFYING_SCORE_BELOW_90 |

No adjudication trigger: per-run total spreads are 2.5, 2.5, 5, and 5 (none >10); no gate disagreement. Individual ballots remain unchanged. A minority total below 90 does not itself trigger adjudication or alter the frozen sum-of-criterion-medians rule. No blocking scoring issues.

## All 20 per-criterion medians

| Criterion | Investigate | Chore | Bugfix | Feature |
|---|---:|---:|---:|---:|
| A1 | 5 | 5 | 5 | 5 |
| A2 | 5 | 5 | 5 | 5 |
| A3 | 5 | 5 | 5 | 5 |
| A4 | 5 | 5 | 5 | 5 |
| A5 | 5 | 5 | 5 | 5 |
| A6 | 5 | 5 | 5 | 5 |
| B1 | 5 | 5 | 2.5 | 2.5 |
| B2 | 5 | 5 | 2.5 | 0 |
| B3 | 5 | 5 | 5 | 5 |
| B4 | 5 | 2.5 | 5 | 5 |
| B5 | 5 | 5 | 5 | 5 |
| C1 | 5 | 5 | 5 | 5 |
| C2 | 5 | 5 | 5 | 5 |
| C3 | 5 | 5 | 5 | 5 |
| C4 | 5 | 5 | 5 | 5 |
| D1 | 5 | 5 | 5 | 5 |
| D2 | 2.5 | 2.5 | 2.5 | 2.5 |
| D3 | 5 | 5 | 5 | 2.5 |
| E1 | 5 | 5 | 5 | 5 |
| E2 | 5 | 5 | 5 | 5 |

## Evidence locators and limitations

Each locator is relative to the indicated anonymous bundle under `bundles/`. Full per-criterion locators/rationales are in each sealed `judge-*/initial/ballot.json`; minority judgments are also retained verbatim in `initial-aggregate.json` and `summary.json`.

### Investigate — run-c8d041 — 97.5/PASS

- Correct bounded configuration diagnosis; `evidence/final-oracle.json` exit 0 corroborates effective 8123, fallback 9000, default 8000.
- D2 median 2.5: explicit scoping/progression recorded after initial diagnosis/handoff. `transcript/chronological.txt` ordinals **5–16, 17–27**, particularly **20–27**; `evidence/assigned-template.json`.
- Judge-2 minority D1=2.5 for initial progression bypass/reminder; other judges reserve this for D2.
- Live listener state and upstream origin of the environment override remain unproved and properly disclosed; transcript ordinals **24, 26, 30**. No workspace files changed (`evidence/changes.json`, before/after hash inventories).

### Chore — run-1e96b3 — 95/PASS

- `evidence/final-oracle.json` exit 0 corroborates the documented example's correct CSV. No parser fallback required; absent unittest/pristine paths are inapplicable, not failures.
- B4 median 2.5: worker verifies header, execution/source alignment, and retained prose, but not data rows/comma-containing-name quoting. `transcript/chronological.txt` ordinals **7, 18–19**; `evidence/acceptance.json`. Evaluator row checks cannot earn worker execution credit.
- D2 median 2.5: edit and initial handoff before explicit scope; ordinals **8–16, 18–29**. Judge-2 minority D1 deduction retained.
- Rejected taskless typo launch was discarded before provisioning/prompt, not scored. Original record remains at `runs/chore-base-002/rejected-idle-launch.json`; sanitized deviation supplied identically to all judges.

### Bugfix — run-f7502a — 92.5/PASS

- `evidence/final-oracle.json` exit 0 covers 8 interval cases/input preservation; `evidence/final-suite.json` exit 0, **6 tests OK**. Pristine overlay exits 1 with **2 target assertion failures**, corroborating sensitivity.
- B1/B2 medians 2.5: source defect observed before editing, but meaningful regression failures demonstrated only **after implementation**, using reconstructed original behavior. `transcript/chronological.txt` ordinals **6, 10–18, 23–24**. The later reproduction is substantive, not chronological pre-fix RED.
- D2 median 2.5: fix precedes reproduce/diagnose progression; ordinals **11–20, 23–36**.
- Minority judge-2 B2=0 (strict missing pre-implementation failing-check reading) and D1=2.5 retained. Its total 87.5 is not averaged away or silently excluded; frozen criterion medians still yield 92.5.

### Feature — run-63ab9e — 87.5/PASS — NOT QUALIFYING

- Final behavior is correct within contract: `evidence/final-oracle.json` exit 0 confirms limit/legacy compatibility; `evidence/final-suite.json` exit 0, **7 tests OK**. Pristine overlay exits 1 with missing-limit TypeErrors at the intended API, corroborating test sensitivity only.
- B1 median 2.5: original signature/source read but no executable baseline before implementation. B2 median **0**: no meaningful worker RED anywhere; missing `python` executable is unrelated setup failure. `transcript/chronological.txt` ordinals **6, 10–18**; `evidence/pristine-with-worker-tests.json`.
- D2/D3 medians 2.5: required prospective verification design replaced with retrospective test descriptions after implementation; `evidence/assigned-template.json`, transcript ordinals **23–29, 33–37**.
- Minority judge-3 B2=2.5, C4=2.5, D3=5 retained; C4 concern is incomplete whole-change/test/workspace review (ordinals **29, 33–35**). Judge-2 additionally deducts D1. Judge-3's total 90 does not make the panel qualify.
- Other limit types/non-string names are outside contract and were not demanded. Gate PASS does not override the numeric threshold.

## Preservation and recovery

All three bare reports read before archival. Raw judge sessions: J1 **171 messages/9 pages**, J2 **154/8**, J3 **115/6**. Final ballots are complete. Initial raw pages include **44 collapsed tool messages for J1 and 23 for J2**; every one was recovered using read-only `get_message` with saved raw supplemental responses and byte-length checks. Original archives remain untouched; `session/messages-recovered.json` and `session/recovery-summary.json` supplement them. J3 needs no recovery. No unresolved archive loss, worker archive warning, bundle hash mismatch, or locator issue remains. Recovery utility initially assumed character lengths, then was corrected to protocol UTF-8 byte counts; raw responses preserved and reused, no evidence or ballot changes.

- `judge-{1,2,3}/initial/session/`: full original raw sessions and supplementary recovery.
- `judge-{1,2,3}/initial/ballot.json`, `final-response.txt`, `lock.json`: sealed outputs/hashes.
- `initial-aggregate.json`: arithmetic, spreads/gates, criterion ranges and minority judgments.
- `summary.json`: final qualification/evidence-completeness disposition.
- `locator-audit.json`: hash/locator checks and explicit collapse recovery resolution.
- `provenance.json`, `preregistration.json`, `sessions.json`: mappings, fixed prompts/models, session identities.

## Independence / scope

Three fresh high-effort read-only `openai-oauth/gpt-6-astra` sessions, each scoring the same four anonymous bundles in the same randomized order; separate criterion ballots per run, no cross-judge votes exposed before lock or afterward. No earlier scores, target/goal document, proposals, or candidate rationale supplied. **Same-model session independence, not provider diversity. Imperfect condition blinding from unchanged raw evidence is disclosed**, as in the baseline panel. Within-session batch context and common model may correlate judgments. Committed history is not every live workflow broadcast; unseen actions are not credited.

Parent reported archival/hash verification followed by worker-container cleanup. Coordinator independently reverified inventory/bundle hashes and retained evidence but did not inspect or execute worker code, contact workers/containers, or edit source/templates/frozen rules. No proposal/vote phase conducted. These are individual-run qualification decisions, not a workflow streak or population success claim.
