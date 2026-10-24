<!-- quecto-task schema=1 -->
# B8: Remote start and end_run choreography

| Field | Value |
|---|---|
| Id | board-2705-b8 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:21Z |
| Updated | 2026-10-24T15:05:40Z |

## Description
Parent-owned next deliverable after parent B3 AND operator B7 have merged: start/end_run choreography across the wire. Consume operator B7 server-side one-transaction /op, blocking UDS/TCP remote client, clock gate and contention without duplicating them. RunFacts DTO/fact-adapter contracts; BeginBoardRun server use case, client launch/undo, end_run/fold-back/PR evidence. Client owns launch/evidence; server transaction/time, failure/lost-run parity. No deferred task redesign; use current owner task-completion rules. Fresh planning/red/delivery/independent current-head review phases; 750target1500max PRODUCTION lines/tests excluded, size once planning. Once CI green only unresolved High/Medium block merge; Low issues linked as follow-ups. Then reduced B13-lite and PocketP1P2, never deployment/cut-over/live board imports/fences/server or unit installation/start. All original host safety/resource constraints remain.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b7 |
