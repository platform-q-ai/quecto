<!-- quecto-task schema=1 -->
# B7: Transactional /op and blocking UDS/TCP client

| Field | Value |
|---|---|
| Id | board-2705-b7 |
| Kind | task |
| Status | archived |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:59Z |
| Updated | 2026-10-25T19:24:00Z |

## Description
OPERATOR-OWNED AND EXTERNALLY DELIVERED. [PR #2728](https://github.com/platform-q-ai/quecto/pull/2728) merged2026-10-25T08:35:53Z as ea9265df365a930a20575d1529027d0889810849, verified from primary GitHub PR metadata. This stale unclaimed draft is archived as completed external prerequisite tracking, not started or implemented by the parent lane. Do not duplicate B7.

Delivered server-side /op executes in one SQLite transaction with persisted atomic write_id replay; remote blocking UDS/TCP client, authoritative clock and contention consume B4 PR2719/T2 PR2720/T3 PR2722/B5-lite PR2721. B7 already includes server-side done_without_run PR-state pre-read. B8 consumes actual merged operations/facts/lifecycle interfaces and only moves end_run Git evidence outside the write lock, preserving evidence/staleness/race/idempotence/error semantics. Snapshot follower service remains B11 deferred.

Parent lane remaining work is fence, then scoped B8; ALL Pocket P1/P2-lite is now operator-owned. No duplicate wire/client/server redesign or operator lane work. Target750/hard1500 production additions+deletions excluding dedicated tests; normal trusted foreground pushes at least1500seconds, fixed pool/member_limit25/global maximum2 building members including operator. No live board import/export/fence/cut-over/deployment/server/unit start/relay/state/admission/credential access/protected-playbook/global-config changes or complicated removal.

## Depends on
| Task |
|---|
| board-2705-b5-lite |
