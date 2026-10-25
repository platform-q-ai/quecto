<!-- quecto-plan schema=1 -->
# Plan 5 of board-2705-b13-lite

| Field | Value |
|---|---|
| Task | board-2705-b13-lite |
| Version | 5 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-25T07:52:39Z |
| Updated | 2026-10-25T07:52:39Z |

## Approach
Deliver the owner's explicitly authorised B13 split as TWO complete, independently testable PRs. PR1 is online SQLite backup; PR2 is wiring to the existing fence builder. No runtime dependency exists: both branch from then-current origin/master, with backup opened first. Capability-specific ports and composition avoid an invented shared DTO/helper dependency. Each merge leaves master buildable and green.

The fresh planning run completed at 2fc9733af1c7a82823162a9223eca42fa5b79b3f. Full accepted proposal, ownership, exact assertion ledger, RED surfaces and resolved different-member challenge are in /home/shane-pocket/.quecto/artifacts/b13-lite-split-planning/{parent-report.md,proposed-tasks.json,source-ownership.md,size-once.md,coverage-partition.json,red-handoff.md,challenge.md}. Owner authority is /home/shane-pocket/.quecto/artifacts/b13-lite-recovery/owner-authorized-split.md. These supersede prior one-PR instructions; all unaffected safety and failure semantics remain.

Preserved implementation a05bfd59a0645d99fff38555133f7489ff48295e is reference-only until each new PR's separate fresh RED phase. Its verified bundle is /home/shane-pocket/.quecto/artifacts/b13-lite-recovery/b13-lite-implementation-a05.bundle, SHA256 fe9c90e4a80efe3ad56ffc4b62476c97bfcf4be3aa7d8b569c84278235bb2102, prerequisites fa364c9043fbdd1b83e5407fa5b5f064ffc0d4dc and6d3e874fd6b8df55f0373d69107b7da499c8c6e7. Never merge the combined feature branch or disposable spike wholesale. Original f6821a26c47cf4b28f85ed588d6708e4e22ea96d remains immutable forensic evidence. Every new PR gets its own first RED commit and verified external bundle, then a different fresh delivery swarm; no failing RED push or phase rollover.

One measured allocation is backup1275 production additions+deletions (225 headroom), fence396 (1104 headroom), including conservative test declarations. Hard1500 per PR, target750, tests excluded; docs41/37, required backup Cargo feature2, and each future own-LAST patch/lock reported separately. No repeated optimisation, golf, safety cuts or waiver. All56 source paths assigned; all68 original semantics and precise R7 clone0 prebaseline hydration preserved in70 feature-local tests (54backup/16fence). Only mechanical imports/modules, feature-specific help/badargv partition and already-approved R7 are authorised. No future sibling imports or temporary duplicate owners.

Each phase refreshes actual master and preserves T4/T5/operator B7. Use fixed task-appropriate pools/member_limit25; global maximum2 building members includes operator, maximum1 owned builder. No workflow, nested containers or mutation tools. Coordinators judge/integrate rather than redo work. Normal foreground trusted pushes require timeout>=1500s and sufficient tool time, own-LAST actual max(current,master)+1 patch, merge-requested/current CI, and fresh independent review of the published exact head. After CI green only validated high/medium findings block; lows receive linked followups. Both PRs must merge before B13 completes. B8 later ONLY moves end_run Git evidence outside the write lock after B13+B7; B7 already includes done_without_run PR pre-read.

No live backup/import/export/fence, cut-over/deploy, server/listener/unit operations, live relay/state/admission/credential/env access, protected-playbook/global-config changes or complicated removal. Unsafe owned probes use hard SIGKILL/_exit timeouts, core0, jobs1, memory caps and cleanup.

## Task items
| Task item | Title |
|---|---|
| backup | Merge complete backup PR1 |
| fence | Merge complete independent fence PR2 |
| aggregate | Verify complete B13 coverage and merge state |

## Task item dependencies
| Task item | Depends on |
|---|---|
| aggregate | backup |
| aggregate | fence |

## Criteria
| Task item | Criterion |
|---|---|
| backup | Child board-2705-b13-backup completes separate fresh RED/delivery phases on current master. |
| backup | Real online backup, exact integrity,14 daily retention and all accepted safety/failure contracts are green, under1500 production lines. |
| backup | Published exact-head independent review has no unresolved high/medium findings, required current CI passes, PR merged. |
| fence | Child board-2705-b13-fence branches from then-current master without a fabricated backup dependency; backup PR opened first. |
| fence | Existing builder/private Git CAS reused, same-destination idempotence and elsewhere refusal proved on scratch remotes only. |
| fence | Separate fresh RED/delivery, under-cap source, clean exact-head independent review and required current CI precede merge. |
| aggregate | All68 original semantics preserved across70 feature tests; no safety/assertion weakening beyond authorised mechanical partition and precise R7 precondition. |
| aggregate | Both child PRs merged and linked; only then complete the B13 umbrella. |
| aggregate | B8 remains narrowed to end_run Git evidence outside write lock after B7; no duplicate server PR pre-read or live operations. |
