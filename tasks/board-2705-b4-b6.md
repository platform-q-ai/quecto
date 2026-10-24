<!-- quecto-task schema=1 -->
# Operator B4 fulfilled: UDS board server merged #2719

| Field | Value |
|---|---|
| Id | board-2705-b4-b6 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:46Z |
| Updated | 2026-10-24T10:13:28Z |

## Description
External prerequisite fulfilled by operator PR2719, version0.107.436, per operator confirmation. quecto board-server serve over UDS; /v1/status and /v1/time, request limits, write deadline, connection lifetime cap, typed error codes, docs/board-server.md. Snapshot is explicitly deferred to B11; blocking client belongs to B7. Legacy id retains b4-b6; B6 was already merged PR2714 and B1 PR2715. Consume actual merged interfaces, never duplicate server/root/peer ownership. Git ProjectBoardStore works now, SQLite after B2. No agent implementation claim; archived as fulfilled dependency. No installation, real service/unit start, deployment or cut-over.

## Depends on
| Task |
|---|
| board-2705-b1 |
