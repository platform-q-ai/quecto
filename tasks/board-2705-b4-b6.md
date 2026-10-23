<!-- quecto-task schema=1 -->
# B4+B6: UDS board server and BoardOperations port

| Field | Value |
|---|---|
| Id | board-2705-b4-b6 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:46Z |
| Updated | 2026-10-23T16:47:46Z |

## Description
MVP stage4 epic #2705. BoardOperations application port, LocalBoardOperations and reusable contract; CLI/tool composition selects injected operations. Bounded hyper UDS board-server listener with injected handlers, peer UID/permissions; interface status/time/snapshot routes, composition-only use-case construction. Allowlisted methods/routes,1MiB body,32 headers/8KiB,10s header/body timeouts,64 connections. No host server/unit installation/start. One complete PR baseline, target300–500 hard900 including tests/dependency overhead/version. No future TCP/members/follower scope; parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b1 |
