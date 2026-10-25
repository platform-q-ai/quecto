<!-- quecto-task schema=1 -->
# B13-lite reduced: Online backup and fence-command wiring

| Field | Value |
|---|---|
| Id | board-2705-b13-lite |
| Kind | task |
| Status | in_progress |
| Parent | board-service-mvp-2705 |
| Accepted plan | 5 |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-25T22:07:57Z |
| Claim expires | 2026-10-26T00:07:57Z |
| Created | 2026-10-23T16:48:32Z |
| Updated | 2026-10-25T22:07:57Z |

## Description
Operator-authorized next epic2705 deliverable NOW after completed B3; independent of B7/B8. Reduced B13-lite: (1) `quecto board-server backup --to <dir>` uses SQLite's online backup API and integrity_check, publishes completed `<board>-<ts>.sqlite` snapshots, retaining14 daily copies. Never filesystem-copy a live DB/WAL; pin usable consistent snapshot while writers operate, atomic verified publication and safe per-board retention/failure cleanup. (2) wire `quecto board-server fence` to OPERATOR'S already-merged #2718 pure fence-tree builder, publishing/pushing format4+MOVED commit to quecto/board. Idempotent same-destination fence; typed refusal if already fenced elsewhere, preserve current head/CAS semantics and existing project history. Fence testing ONLY owned scratch remotes, NEVER live board. Reuse actual merged ports/builders/allowlisted parser/composition, no duplicate format4/board_moved/builder ownership. Target fewest complete independently mergeable PRs,750target1500max PRODUCTION changed lines/tests excluded, sized once planning; split only if justified required ceiling. Fresh planning, per-PR fresh RED, fresh delivery, exact-head independent review, normalforegroundpush>=1500s/trustedhooks/ownLASTpatch/merge-requested/currentCI; aftergreen only unresolvedHMblock, lowsfilefollowups. Do not start B7/operator work. NextB8 only after B7merge, plus later PocketP1P2 NODEPLOY. No liveimport/export/fence/cutover/deploy/server/listener/unitinstallstart/live relay/state/admission/credentials/env/protectedplaybooks/globalgitconfig/complexrm. Max2buildingmembers globally incloperator. Full original owner plan/header ~/handover/board-epic/plan.md/spike.md reference only. Current completed B3 merged2731/2738 at6d3e874/version0.107.446.

## Depends on
| Task |
|---|
| board-2705-b3 |
