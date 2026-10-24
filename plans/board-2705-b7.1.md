<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b7

| Field | Value |
|---|---|
| Task | board-2705-b7 |
| Version | 1 |
| State | discarded |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:47Z |
| Updated | 2026-10-24T00:12:27Z |

## Approach
One serial PR after UDS/operations seam. Run operation under per-board lock in one SQLite transaction; durable request-ID result replay and remote UDS client. Time gate and explicit committer wiring in proper layers. Fresh red/green/independent review; hard900 all-in, targeted local verification/full CI.

## Task items
| Task item | Title |
|---|---|
| transaction | Transactional operation and write_id replay |
| remote | Remote operations client contract |

## Task item dependencies
| Task item | Depends on |
|---|---|
| remote | transaction |

## Criteria
| Task item | Criterion |
|---|---|
| transaction | Concurrent operations lose no accepted updates; same ID returns first answer without extra revision after dropped response/reopen. |
| transaction | Failures roll back all state; clock gate enforced. |
| remote | Local/remote reusable contract parity and explicit committer; transport retry bounded/idempotent. |
| remote | No client whole-board write decode/CAS workaround. |
