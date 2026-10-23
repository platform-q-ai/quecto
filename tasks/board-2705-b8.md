<!-- quecto-task schema=1 -->
# B8: Remote start and end_run choreography

| Field | Value |
|---|---|
| Id | board-2705-b8 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:21Z |
| Updated | 2026-10-23T16:48:21Z |

## Description
MVP stage7 epic #2705. RunFacts DTO and stated fact adapters with contracts; BeginBoardRun server use case and client launch/undo choreography; end_run/fold-back and PR merge evidence over wire. Client owns launch/evidence collection; server owns transaction/time. Preserve failure and lost-run parity. Do not implement deferred task redesign; merged PR completes task only from Review per owner policy. One independently green PR baseline hard900 inclusive tests/version; fresh red/green/review phases and parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b5-lite |
