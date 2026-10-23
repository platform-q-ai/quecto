<!-- quecto-task schema=1 -->
# B7: Transactional server operations and remote client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-23T16:47:59Z |

## Description
MVP stage5 epic #2705. ServeBoardOperation executes server-side operation under per-board lock in one SQLite transaction, atomic persisted write_id result replay; remote blocking client/RemoteBoardOperations contract via UDS, time gate and explicit committer composition. No client whole-board write decode/CAS retry workaround; transport retries idempotent. B7 owns request idempotency, not invented B2 private seam. No TCP required before B5-lite. One complete PR baseline; targeted red tests then green, independent review/CI; hard900 all changed lines, size just-in-time. All parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b4-b6 |
