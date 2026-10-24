<!-- quecto-task schema=1 -->
# B7: Transactional server operations and remote client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-24T07:16:57Z |

## Description
Agent B7 only after B3 and OPERATOR-owned B4 UDS server have merged. Operator B1 PR2715 and B6 PR2714 are merged; git-side B13 PR2718 is merged. Consume their actual interfaces, never duplicate B1/B6/B4 listener/status/time/snapshot/peer-credential or T1/format4-MOVED/parser/pure-fence-builder ownership. ServeBoardOperation executes per-board operation in ONE SQLite transaction with atomic persisted write_id result replay; remote blocking client/RemoteBoardOperations over UDS, time gate and explicit committer. Use --board-id/--remote-url and existing BoardOperations. No client whole-board-write decode/CAS retry workaround; transport retries are idempotent. B7 owns request replay; TCP follows in B5-lite. Fresh red/green/independent exact-head review and required CI; human-approved2713 production750target1500max, tests/fixtures/snapshots/lock/generated excluded/reported, size once planning/rejudge only remediation checkpoint or maximum exceeded. All owner safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b4-b6 |
