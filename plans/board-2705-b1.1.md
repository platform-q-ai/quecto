<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b1

| Field | Value |
|---|---|
| Task | board-2705-b1 |
| Version | 1 |
| State | discarded |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:41Z |
| Updated | 2026-10-24T00:11:54Z |

## Approach
One serial PR after B3 and actual merge #2648. Extract git remote-revision adapter without behavioral regression; add canonical --board-id/--url and tested config resolution/no-git explicit path. Test new behavior red; review no-red rationale for pure extraction. Proper Clean Architecture, hard900 including tests/version.

## Task items
| Task item | Title |
|---|---|
| split | Split remote revision ownership |
| location | Resolve explicit board location |

## Task item dependencies
| Task item | Depends on |
|---|---|
| location | split |

## Criteria
| Task item | Criterion |
|---|---|
| split | GitRemoteRevisions separate from GitBoardStore and port contract passes; git characterization unchanged. |
| location | --board-id/--url canonical with precedence/error tests; explicit server path bypasses unnecessary git spawns. |
| location | Start gate #2648 merged independently verified. |
