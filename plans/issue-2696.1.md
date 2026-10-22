<!-- quecto-plan schema=1 -->
# Plan 1 of issue-2696

| Field | Value |
|---|---|
| Task | issue-2696 |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:08:46Z |
| Updated | 2026-10-22T00:08:46Z |

## Approach
One self-contained child PR fixing #2696: preflight existing StartedRuns before exit board composition. Preserve started cleanup, refusal and warning semantics. Separate planning, red-test bundle, delivery and independent PR reviews; human merges. Target at most500 changed lines, hard cap900. Baseline ef17b93e0d5058e07fdd3e54f05e6c86f3e31243. Durable planning: /home/swq/.quecto/artifacts/issue-2696/planning/handoff.md. Fixed small pools with member_limit25, only one member builds at a time. Clean Architecture, affirmative guards, TDD/BDD. Required version bump in final commit; no process artifacts in PR.

## Task items
| Task item | Title |
|---|---|
| deliver | Deliver focused child PR |
| accept | Verify epic acceptance |

## Task item dependencies
| Task item | Depends on |
|---|---|
| accept | deliver |

## Criteria
| Task item | Criterion |
|---|---|
| deliver | Empty/no-launcher sessions skip board composition and git with no warning. |
| deliver | Started sessions retain cleanup and unreadable-board diagnostics. |
| deliver | Red-first tests, targeted gates and independently reviewed PR; no merge. |
| accept | Current-head independent review clean; request CI only thereafter. |
| accept | Focused diff within line limits; human merges. |
