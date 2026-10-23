<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b13-lite

| Field | Value |
|---|---|
| Task | board-2705-b13-lite |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:57Z |
| Updated | 2026-10-23T16:49:57Z |

## Approach
One serial PR after remote runs. Format4 moved fence/read-write answer plus application backup use case/SQLite online backup CLI. Scratch rollback rehearsal and consistent completed copies for existing sync; no live fence/import/cut-over. Red-first/independent review, hard900 all-in.

## Task items
| Task item | Title |
|---|---|
| fence | Format4 moved fence compatibility |
| backup | Consistent online backup and rollback |

## Task item dependencies
| Task item | Depends on |
|---|---|
| backup | fence |

## Criteria
| Task item | Criterion |
|---|---|
| fence | Old/read/write paths fail safely and updated clients render board moved answer using scratch fixtures only. |
| backup | Concurrent writes yield integrity-valid completed backup; never copy active SQLite/WAL as backup. |
| backup | Scratch import/fence/switch/rollback rehearsal validated; no follower/redesign/promote scope. |
