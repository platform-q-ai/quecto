<!-- quecto-task schema=1 -->
# B7: Transactional server operations and remote client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-24T00:12:21Z |

## Description
Agent MVP B7 after B4 (legacy board-2705-b4-b6 is now B4 only). EXTERNALGATE: operator-owned B6 BoardOperations MUST actually be MERGED before starting B7; establish primary GitHub evidence, and if absent notify operator instead of implementing B6. ServeBoardOperation executes server operation under per-board lock in one SQLite transaction, atomic persisted write_id result replay; remote blocking client/RemoteBoardOperations via UDS, time gate and explicit committer. Integrate operator B1 --board-id/--remote-url and B6 interfaces without duplicate ownership. No client whole-board write decode/CAS retry workaround; transport retries idempotent. B7 owns request idempotency, not a B2 private seam. No TCP required before B5-lite. Fresh red tests/delivery/independent review/CI; human-approved2713 sizing target750/max1500PRODUCTION lines, tests/fixtures/snapshots/lockfiles/generated excluded and separately reported; size once at planning, rejudge only remediation checkpoint or maximum exceeded. All parent safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b4-b6 |
