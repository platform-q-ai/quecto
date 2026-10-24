<!-- quecto-task schema=1 -->
# B7: Transactional /op and blocking UDS/TCP client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-24T10:14:18Z |

## Description
Agent B7 only after B3 AND operator-owned B5-lite TCP/Tailscale/per-device-token delivery actually merge. Operator B4 is merged PR2719/version0.107.436: board-server serve UDS /v1/status,/v1/time, request limits/write deadline/connection lifetime cap/typed errors/docs. Snapshot route explicitly deferred to B11. B1 PR2715, B6 PR2714 and git fence PR2718 already merged. Consume actual server/transport/auth/config/BoardOperations interfaces; never duplicate B4/B5 listener, peer/token or T1/format4-MOVED/parser/pure-fence-builder ownership. ServeBoardOperation executes /op in ONE SQLite transaction with atomic persisted write_id result replay. B7 ALSO owns the blocking remote client over BOTH UDS AND TCP, time gate and explicit committer; no client whole-board-write decode/CAS retry workaround, transport retries idempotent. Latest owned lane B2>B3>B7(after operator B5-lite)>B8>reduced B13-lite>PocketP1P2(no deployment). Fresh red/green/independent exact-head review and green CI; accepted epic production750target1500max with exclusions separately reported. No live import/fence/cut-over/service installation/start or deployment; all owner safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b5-lite |
