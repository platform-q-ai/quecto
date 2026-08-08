# Plan2 evaluation results — issue #1395

Five independent `plan2` runs used `openai-oauth/gpt-5.5` at low effort. Each run created a separate evaluation issue. Independent judges then evaluated each issue against source issue #1395 using the same model and effort.

| Rank | Candidate | Score | Verdict |
|---:|---|---:|---|
| 1 | [#1417](https://github.com/platform-q-ai/quecto/issues/1417) | 95/100 | PASS |
| 2 | [#1416](https://github.com/platform-q-ai/quecto/issues/1416) | 90/100 | PASS |
| 3= | [#1415](https://github.com/platform-q-ai/quecto/issues/1415) | 88/100 | PASS |
| 3= | [#1418](https://github.com/platform-q-ai/quecto/issues/1418) | 88/100 | PASS |
| 5 | [#1419](https://github.com/platform-q-ai/quecto/issues/1419) | 78/100 | FAIL |

## Findings

- **Winner:** #1417 had the strongest requirements traceability, architectural boundaries, ADR handling, and verification structure.
- **Useful element from #1416:** characterize the existing environment-membership failure path before inventing another failure seam.
- **Failure in #1419:** it narrowed #1395's permitted outcomes by excluding branch removal and making a production failure seam mandatory.

## Workflow change

The experiment identified this general planning invariant:

> Preserve every valid resolution permitted by the source issue unless repository evidence or an explicit recorded human decision eliminates it. Do not turn an example, suggestion, or candidate approach into a mandatory implementation choice.

This invariant was added to `plan2` in commit `8368bcb7`.

## Version history

### V1 — initial workflow plus outcome-preservation invariant

Workflow commits:

- `395dc0be` — add the experimental `plan2` template
- `8368bcb7` — preserve every source-permitted resolution

Results:

| Rank | Candidate | Requirements /25 | Architecture /25 | ADR /20 | Executability /30 | Total | Verdict |
|---:|---|---:|---:|---:|---:|---:|---|
| 1 | [#1417](https://github.com/platform-q-ai/quecto/issues/1417) | 25 | 24 | 19 | 27 | 95 | PASS |
| 2 | [#1416](https://github.com/platform-q-ai/quecto/issues/1416) | 24 | 23 | 18 | 25 | 90 | PASS |
| 3= | [#1415](https://github.com/platform-q-ai/quecto/issues/1415) | — | — | — | — | 88 | PASS with minor reservations |
| 3= | [#1418](https://github.com/platform-q-ai/quecto/issues/1418) | 24 | 22 | 19 | 23 | 88 | PASS |
| 5 | [#1419](https://github.com/platform-q-ai/quecto/issues/1419) | 17 | 24 | 18 | 19 | 78 | FAIL |

Aggregate: mean **87.8**, hard-gate pass rate **4/5**, 95+ rate **1/5**.

The #1415 judge's final abbreviated report did not preserve dimension scores, so those cells remain unknown rather than reconstructed.

### V2 — finding ledgers, holistic validation, and delivery verification

Workflow commit: `2c082f13`.

Changes under test:

- specialist finding ledgers and fresh-verifier loops;
- dedicated source-conformance review;
- two consecutive clean holistic validation rounds;
- post-delivery artifact verification;
- no PASS by absence or fabricated artifact.

Results:

| Rank | Candidate | Requirements /25 | Architecture /25 | ADR /20 | Executability /30 | Total | Verdict |
|---:|---|---:|---:|---:|---:|---:|---|
| 1 | [#1426](https://github.com/platform-q-ai/quecto/issues/1426) | 25 | 24 | 19 | 28 | 96 | PASS |
| 2= | [#1423](https://github.com/platform-q-ai/quecto/issues/1423) | 25 | 24 | 20 | 25 | 94 | PASS |
| 2= | [#1425](https://github.com/platform-q-ai/quecto/issues/1425) | 25 | 24 | 19 | 26 | 94 | PASS |
| 2= | [#1427](https://github.com/platform-q-ai/quecto/issues/1427) | 25 | 24 | 19 | 26 | 94 | PASS |
| 5 | [#1424](https://github.com/platform-q-ai/quecto/issues/1424) | 24 | 23 | 18 | 26 | 91 | PASS |

Aggregate: mean **93.8**, hard-gate pass rate **5/5**, 95+ rate **1/5**.

Recurring judge findings:

- verification evidence was sometimes less concrete than currently knowable;
- plans sometimes blurred a direct failure with a neighboring transaction-stage failure;
- #1424 promoted an unproven interpretation into a third executable path instead of resolving or blocking it;
- some regression coverage broadened beyond a named acceptance criterion or invariant;
- fresh agents still inherited the draft's framing, so two clean rounds were not fully independent analyses.

### V3 — independent planning baseline

Workflow commit: `46ee0ecc`.

This version adds a draft-blind, independently verified and frozen planning baseline before AC or plan drafting. Later reviews compare against that baseline, classify unresolved choices as human blockers, bounded discovery, or speculative options to remove, and close findings only when the underlying risk is eliminated rather than textually accommodated.

Results under the same judging basis and rubric as V2:

| Rank | Candidate | Requirements /25 | Architecture /25 | ADR /20 | Executability /30 | Total | Verdict |
|---:|---|---:|---:|---:|---:|---:|---|
| 1 | [#1432](https://github.com/platform-q-ai/quecto/issues/1432) | 25 | 24 | 19 | 28 | 96 | PASS |
| 2 | [#1429](https://github.com/platform-q-ai/quecto/issues/1429) | 25 | 24 | 19 | 27 | 95 | PASS |
| 3 | [#1431](https://github.com/platform-q-ai/quecto/issues/1431) | 25 | 24 | 19 | 26 | 94 | PASS |
| 4 | [#1430](https://github.com/platform-q-ai/quecto/issues/1430) | 24 | 23 | 19 | 27 | 93 | PASS |
| 5 | [#1428](https://github.com/platform-q-ai/quecto/issues/1428) | — | — | — | — | 92 | PASS |

Aggregate: mean **94.0**, hard-gate pass rate **5/5**, 95+ rate **2/5**, range **92–96**.

The #1428 replacement judge's abbreviated final report did not preserve dimension scores, so those cells remain unknown rather than reconstructed.

Comparison with V2:

- mean: **93.8 → 94.0**;
- hard-gate pass rate: **5/5 → 5/5**;
- 95+ rate: **1/5 → 2/5**;
- minimum score: **91 → 92**.

Recurring findings:

- bounded discovery is valid, but the decision artifact and objective selection criteria should be explicit before implementation;
- planned verification commands may remain indicative where the exact implementation path is intentionally unresolved;
- any proposed production failure mechanism must be a justified invariant, not a convenient test seam;
- inspection plus targeted tests may be the appropriate evidence when proving the absence of a behavior.

### Invalid V3 judging attempts

The first V3 judging pass incorrectly added baseline-artifact availability to the judging basis. Previous rounds judged only the delivered issue/plan against source and repository evidence, and the baseline is an internal workflow mechanism rather than a required real-world deliverable. Scores **94, 92, 89, 92, 92** from that pass are invalid and excluded.

A first replacement judge for #1431 also made a category error by evaluating the planning issue as completed implementation and requiring adapter tests to exist already. Its **72/100 FAIL** is invalid and excluded. A fresh planning judge produced the recorded **94/100 PASS**.

## Model experiment note

A weaker-model experiment was attempted with DeepSeek and Grok. DeepSeek could not run because its provider was unavailable in the container environment. Grok runs suffered provider 503s and produced missing or fabricated artifacts; they were excluded from workflow-quality scoring. No rollback decision should use those invalid samples.

## Rollback reference

To compare or restore workflow behavior:

- Initial experiment template: `395dc0be`
- V1 outcome-preservation behavior: `8368bcb7`
- V2 validation-loop behavior: `2c082f13`
- V3 independent-baseline behavior: `46ee0ecc`
