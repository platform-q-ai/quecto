<!-- quecto-task schema=1 -->
# B7: Transactional server operations and remote client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-24T03:03:02Z |

## Description
Agent B7 after B4-only server (legacy board-2705-b4-b6). Operator B6 external merge gate is now SATISFIED: PR2714 merged2026-10-24T02:30:59Z, commit5e2eca19285f040750d91ace51d6cfbb70b8b5cd, primary GitHub verified; confirm integrated interface when reached, never rebuild operator B6. ServeBoardOperation executes per-board operation in ONE SQLite transaction with atomic persisted write_id result replay; remote blocking client/RemoteBoardOperations via UDS, time gate and explicit committer. Use operator B1 GitRemoteRevisions/--board-id/--remote-url and B6 interfaces without duplicate ownership. Operator also owns T1 entry fixes and git-store format4/MOVED/fence builder/parser; don't implement those. No client whole-board-write decode/CAS retry workaround; transport retries idempotent. B7 owns request replay, no TCP required before B5-lite. Fresh red/green/independent review/required head-bound CI; human-approved2713 target750/max1500production, tests/fixtures/snapshots/lock/generated excluded/reported, size once planning/rejudge only remediation checkpoint/max exceeded. All safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b4-b6 |
