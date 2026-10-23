<!-- quecto-task schema=1 -->
# Pocket P1+P2: Relay UDS and deploy artifacts, no deployment

| Field | Value |
|---|---|
| Id | board-2705-pocket-p1-p2 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:45Z |
| Updated | 2026-10-23T16:48:45Z |

## Description
MVP stage9 epic #2705 in platform-q-ai/quecto-pocket, host clone ~/repos/quecto-pocket. Relay server|git backend allowlist/board_id; --url UDS with explicit committer owner via pocket and no server-backend command-scope git identity or clones. Compatible minimum harness version; service/unit/token templates plus existing backup sync every minute and operator rollback docs. Eccup backup recipient, no live follower/SSE/failover UI. DO NOT deploy or touch live relay/state/units, install/start server/units or modify admission. One complete independently green PR baseline hard900 inclusive tests; source-grounded just-in-time sizing and fresh red/green/independent review. All parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b13-lite |
