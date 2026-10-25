<!-- quecto-task schema=1 -->
# B8: Move end_run Git evidence check outside the write lock

| Field | Value |
|---|---|
| Id | board-2705-b8 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:21Z |
| Updated | 2026-10-25T07:28:07Z |

## Description
Latest operator-authorized B8 scope ONLY: after BOTH B13-lite backup/fence PRs and operator B7#2728 have merged, move end_run Git evidence check OUTSIDE the board write lock (the current check can hold it ~4 minutes), then perform the guarded transaction using pre-read evidence. Consume actual merged B7 wire/operation/facts implementation and T4#2735/T5#2737 lifecycle rules. B7 ALREADY includes server-side done_without_run PR-state pre-read; verify and reuse it, DO NOT reimplement that work. No general start/end_run wire redesign, extra RunFacts/client/server overhaul, duplicate B7/T4/T5 work, or unrelated lifecycle changes. Preserve existing evidence semantics, stale-fact/current-head rejection, races/idempotence/rollback/error parity; obtain lasting focused red coverage of lock availability while Git evidence is stalled under a hard timeout. Fresh scoped planning/per-PR RED/delivery/PR-first exact-head independent review;750target1500max PRODUCTION changed lines/tests excluded/sized once planning, one complete small fix if feasible. Once CIgreen only unresolvedHMblock, lowslinkedfollowups. AfterB8 PocketP1P2 deploy artifacts ONLY/no deploy. No liveimports/exports/fences/cutover/server/listener/unitinstallstart/live relay/state/admission/credential/env/protectedplaybook/globalconfig/complexrm; globalmax2buildingmembersincloperator; normalforegroundrepo pushes>=1500s/trustedhooks/ownLASTpatch/merge-requested/currentCI.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b7 |
| board-2705-b13-lite |
