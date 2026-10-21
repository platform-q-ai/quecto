<!-- quecto-task schema=1 -->
# Move CLI policy and IO into application boundaries (#2634)

| Field | Value |
|---|---|
| Id | issue-2634 |
| Kind | epic |
| Status | claimed |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-21T23:59:17Z |
| Claim expires | 2026-10-22T01:59:17Z |
| Created | 2026-10-21T23:58:44Z |
| Updated | 2026-10-21T23:59:17Z |

## Description
Deliver GitHub issue platform-q-ai/quecto#2634 to master-targeted PRs without merging: reuse environment listing policy; move forgotten-record kill decisions/runtime ownership out of interface; move run-end wait policy into application service; move setup filesystem/current-directory access behind use-case port; remove redundant overlay validation. Follow Clean Architecture, affirmative guards and test-first delivery. Each PR independently buildable/green, target at most 500 changed lines, maximum 900. Human merges. Separate planning, red-test, delivery and independent review phases; at most one building swarm member at a time. No admission/credentials/human checkout modifications.
