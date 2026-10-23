<!-- quecto-task schema=1 -->
# B13-lite: Moved fence and online backup

| Field | Value |
|---|---|
| Id | board-2705-b13-lite |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:32Z |
| Updated | 2026-10-23T16:48:32Z |

## Description
MVP stage8 epic #2705. Format4 read/write fence and board moved answer; BackupBoard use case and SQLite online backup adapter/CLI; scratch rollback/cut-over rehearsal only. Ship completed consistent backup copies, not live DB/WAL; Pocket P2 templates set existing sync every minute. Never push fence to live quecto/board, import live board or cut over. No follower/promote/demote/SSE/redesign/private export timer requirements. One complete PR baseline hard900 all changed lines; fresh red/green/independent review; all parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b8 |
