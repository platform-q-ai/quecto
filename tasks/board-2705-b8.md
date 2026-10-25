<!-- quecto-task schema=1 -->
# B8: Remote start and end_run choreography

| Field | Value |
|---|---|
| Id | board-2705-b8 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:48:21Z |
| Updated | 2026-10-25T02:49:59Z |

## Description
Parent-owned next deliverable after NOW-authorized reduced B13-lite AND operator B7 have merged (operator B7 PR2728 reconciling B3 schema; T4 PR2735/T5 PR2737 lifecycle also merging). Start/end_run choreography across wire, consume actual merged B7 one-transaction /op/blocking UDS+TCP client/clock gate/contention, do not duplicate or start B7. RunFacts DTO/fact-adapter contracts; BeginBoardRun server use case, client launch/undo/end_run/foldback/evidence; client owns launch/evidence, server owns transaction/time, failure/lost-run parity. REQUIRED B7-review correction: move end_run Git evidence check OUTSIDE the write lock (current check can hold lock ~4min); pre-read PR states for done_without_run the way complete does, then perform guarded transaction using facts. Preserve stale facts/current-head rejection, races/idempotence/rollback semantics. Reconcile actual merged T4/T5 lifecycle and task-completion rules; no deferred task redesign. Fresh planning/per-PR RED/delivery/exact-head independent review;750target1500max PRODUCTION changed lines/tests excluded/sized once planning, fewest independently complete PRs. Once CIgreen only unresolvedHMblock, lowslinkedfollowups. AfterB8 PocketP1P2 deploy artifacts ONLY/no deploy. No liveimports/exports/fences/cutover/server/listener/unitinstallstart/live relay/state/admission/credential/env/protectedplaybook/globalconfig/complexrm; globalmax2buildingmembersincloperator; normalforegroundrepo pushes>=1500s/trustedhooks/ownLASTpatch/merge-requested/currentCI.

## Depends on
| Task |
|---|
| board-2705-b3 |
| board-2705-b7 |
| board-2705-b13-lite |
