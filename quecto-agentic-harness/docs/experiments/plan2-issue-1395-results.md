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
