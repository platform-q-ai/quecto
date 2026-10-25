<!-- quecto-task schema=1 -->
# B13 PR2: Publish board fences through the existing builder and Git CAS

| Field | Value |
|---|---|
| Id | board-2705-b13-fence |
| Kind | task |
| Status | in_progress |
| Parent | board-2705-b13-lite |
| Accepted plan | 2 |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-25T19:35:31Z |
| Claim expires | 2026-10-25T21:35:31Z |
| Created | 2026-10-25T07:51:31Z |
| Updated | 2026-10-25T19:37:24Z |

## Description
Deliver the owner's second complete B13 PR: offline `quecto board-server fence`, using #2718's existing BoardMoved/parser/fence_tree and the Git store's private captured-head CAS publication/reconciliation. Same-destination invocation is idempotent without new fence objects/commit/push or writer-identity lookup; another board/URL/order returns typed Moved. Malformed/unsupported/missing boards refuse without bootstrap or repair. Preserve caller identity, sole captured parent, recognized folders/archive/history and unrelated refs. Test only owned scratch remotes; never fence the live board.

No runtime dependency on backup was found. Branch independently from then-current origin/master; open the backup PR first. Use capability-specific ports/fence.rs and bootstrap/board_fence.rs, not a shared future-feature framework or SQLite backup fixture. Source reference: preserved delivery commit a05bfd59a0645d99fff38555133f7489ff48295e; planning master 2fc9733af1c7a82823162a9223eca42fa5b79b3f. Full approved scope, ownership, coverage and exact RED handoff: /home/shane-pocket/.quecto/artifacts/b13-lite-split-planning/{parent-report.md,proposed-tasks.json,source-ownership.md,size-once.md,coverage-partition.json,red-handoff.md,challenge.md}; owner decision and prior safeguards: /home/shane-pocket/.quecto/artifacts/b13-lite-recovery/owner-authorized-split.md.

Measured allocation:396 production additions+deletions,1104 headroom to hard1500 (target750); docs37 and future patch/lock separate, tests excluded. Preserve all16 feature-local permanent tests and their original assertions, including precise approved R7 clone0 hydration before object baseline, clone1 control and all object/type/ref equalities. Predicted baseline14 failures+2controls is not execution evidence. Together the two PRs preserve all68 original semantics in70 feature tests. Fresh RED swarm and immutable first RED commit/verified external bundle precede a different fresh delivery swarm; no failing RED push. Reuse only this feature's A05 logic after RED acceptance; retain sole builder/private Git owners, named BoardFence contract, sibling tests, USE_CASES/inventory and ordinary750-line source ratchet.

Normal trusted foreground pushes use timeout>=1500s, own-LAST patch, merge-requested/current CI and fresh independent review of the published exact head. After CI green, only validated high/medium findings block; lows become linked followups. Global maximum2 builders includes operator, maximum1 owned builder; fixed pool/member_limit25, no workflows/nested containers/mutation tools. Unsafe owned probes require hard SIGKILL/_exit timeout, core0, jobs1, memory cap and cleanup. No live backup/import/export/fence, cut-over, deployment, server/listener/unit start, relay/state/admission/credential/env access, protected-playbook/global-config changes or complicated removal. B8 later only moves end_run Git evidence outside the write lock after both B13 PRs and B7 merge; B7 already owns done_without_run PR pre-read.

## Pull requests
| Pull request |
|---|
| #2751 |
