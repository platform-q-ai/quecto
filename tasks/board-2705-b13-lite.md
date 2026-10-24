<!-- quecto-task schema=1 -->
# B13-lite reduced: Online backup, fence-command wiring and rehearsal

| Field | Value |
|---|---|
| Id | board-2705-b13-lite |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:32Z |
| Updated | 2026-10-24T03:02:17Z |

## Description
Reduced agent B13-lite for epic2705 after B8: consistent SQLite online backup/BackupBoard use case and CLI; wire quecto board-server fence command to OPERATOR'S pure idempotent fence-tree builder, including push and idempotence; board_moved rehearsal. Tests/push/rehearsal SCRATCHONLY, NEVER live fence/import/cutover/install/start. Operator exclusively owns git-store format4/MOVED typed board_moved reads+writes, pure fence-tree builder, and allowlisted MOVED parser: do NOT implement/duplicate those or T1 NBSP/command identity fixes. Consume merged operator interfaces. Completed consistent backup copies only, never copy live DB/WAL; Pocket templates use minute backup sync. No follower/promote/demote/SSE/private export timer. Human-approved2713 production750target1500max, tests/fixtures/snapshots/lock/generated excluded and separately reported, size once planning/rejudge only remediation checkpoint or maximum exceeded. Fresh red/green/independent review/head-bound CI; all owner safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b8 |
