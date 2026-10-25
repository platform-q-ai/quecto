<!-- quecto-task schema=1 -->
# B8: Move end_run Git evidence check outside the write lock

| Field | Value |
|---|---|
| Id | board-2705-b8 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:21Z |
| Updated | 2026-10-25T19:22:36Z |

## Description
Latest operator-authorized B8 scope ONLY: after BOTH B13-lite backup/fence PRs and operator B7 #2728 have merged, move end_run Git evidence check OUTSIDE the board write lock (the current check can hold it about4minutes), then perform the guarded transaction using pre-read evidence. Consume actual merged B7 wire/operation/facts implementation and T4 #2735/T5 #2737 lifecycle rules. B7 ALREADY includes server-side done_without_run PR-state pre-read; verify and reuse it, DO NOT reimplement that work. No general start/end_run wire redesign, extra RunFacts/client/server overhaul, duplicate B7/T4/T5 work, or unrelated lifecycle changes. Preserve existing evidence semantics, stale-fact/current-head rejection, races/idempotence/rollback/error parity; obtain lasting focused red coverage of lock availability while Git evidence is stalled under a hard timeout.

Fresh scoped planning, per-PR RED, delivery and PR-first exact-head independent review. Target750/hard1500 PRODUCTION additions+deletions, dedicated tests excluded, size once planning; one complete small fix if feasible. Once CI green only unresolved High/Medium findings block, Lows become explicitly accepted linked follow-ups. Normal foreground repository push timeout at least1500seconds, installed/activated/verified actual hook dispatchers before first network push, trusted hooks, own-LAST patch, merge-requested and current CI. Compilation allowances are separate from unchanged hard unsafe-probe/runtime watchdogs.

Latest ownership: operator owns ALL Pocket P1 and P2-lite; do not plan or implement any Pocket code, deployment units, tokens, egress, standby backup-copy sync or failover documentation in this lane. Pocket P1 [#146](https://github.com/platform-q-ai/quecto-pocket/pull/146) is merged; P2-lite remains operator-owned, no follower until B11. This lane ends with fence and this B8 fix. No live imports/exports/fences/cutover/deployment/server/listener/unit installation or start/live relay/state/admission/credential/env access/protected-playbook/global-config changes/complicated removal. Global maximum2 building members including operator, maximum1 owned builder; member_limit25 ceiling, task-appropriate fixed pool, no workflow/nested containers/mutation tools.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b7 |
| board-2705-b13-lite |
