<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b8

| Field | Value |
|---|---|
| Task | board-2705-b8 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:54Z |
| Updated | 2026-10-23T16:49:54Z |

## Approach
One serial PR after TCP/token stage. Run facts and stated adapters contracted; server handles transactional begin/end while client owns launch/evidence and undo. Preserve local parity and owner Review-only merged completion policy, no task redesign. Fresh red/green/review, hard900 all-in and CI.

## Task items
| Task item | Title |
|---|---|
| facts | Contract run facts and adapters |
| runs | Remote run lifecycle and rollback |

## Task item dependencies
| Task item | Depends on |
|---|---|
| runs | facts |

## Criteria
| Task item | Criterion |
|---|---|
| facts | Evidence facts validate safely, self-asserted semantics explicit; missing/invalid/stale facts fail correctly. |
| runs | Start success/launch failure/fold-back/end_run lost/merged cases preserve intended atomicity and client launch ownership. |
| runs | Merged PR completion only from Review; no host launch or deployment in tests. |
