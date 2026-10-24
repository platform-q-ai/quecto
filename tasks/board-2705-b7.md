<!-- quecto-task schema=1 -->
# B7: Transactional /op and blocking UDS/TCP client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-24T15:05:02Z |

## Description
OPERATOR-OWNED per latest epic2705 lane split; parent agent must NOT start or duplicate B7. Server-side /op executes in ONE SQLite transaction with atomic persisted write_id replay; remote client includes BLOCKING UDS AND TCP, authoritative clock gate, contention. Consume merged B4PR2719/T2PR2720/T3PR2722 and B5-litePR2721 TCP/per-device-token interfaces; snapshot service remains B11 deferred. Coordinate actual B3 history/schema3/epochs interfaces without inventing whole-board decode/CAS retry. Parent lane is B3 now, then B8 after this B7 lands, reduced B13-lite(online backup+wire existing fence tree builder), then pocketP1P2 without deployment. 750target1500max PRODUCTION lines/tests excluded, sized once at planning. Once CI green only unresolved High/Medium block merge; Low follow-ups linked. All safety constraints remain: no live board import/fence/cut-over/deploy/services/units/relay/state/admission/credentials/protected playbook changes. At most2 building members, foreground pushes>=1500s.

## Depends on
| Task |
|---|
| board-2705-b5-lite |
