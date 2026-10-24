<!-- quecto-task schema=1 -->
# B13-lite reduced: Online backup and fence-command wiring

| Field | Value |
|---|---|
| Id | board-2705-b13-lite |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:32Z |
| Updated | 2026-10-24T07:17:00Z |

## Description
Reduced agent B13-lite for epic2705 after B8: consistent SQLite online backup/BackupBoard and CLI, plus wiring quecto board-server fence to the OPERATOR'S merged PR2718 pure idempotent fence-tree builder, including publication/push and idempotence. Verify wiring only in owned scratch fixtures, NEVER live fence/import/cutover/install/start. No separate git-side fence or rehearsal deliverable: operator owns format4/MOVED typed board_moved reads/writes, pure fence-tree builder and allowlisted parser; operator also owns B4 command/listener and T1 entry fixes. Consume merged interfaces without duplicate ownership. Completed consistent backup copies only, never copy live DB/WAL; Pocket templates use minute backup sync. No follower/promote/demote/SSE/private export timer. Human-approved2713 production750target1500max, tests/fixtures/snapshots/lock/generated excluded/reported separately; size once planning/rejudge only remediation checkpoint or maximum exceeded. Fresh red/green/independent exact-head review/CI; all owner safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b8 |
