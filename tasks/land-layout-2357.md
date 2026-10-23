<!-- quecto-task schema=1 -->
# Land five parked layout PRs for epic #2357

| Field | Value |
|---|---|
| Id | land-layout-2357 |
| Kind | epic |
| Status | in_progress |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-23T12:40:51Z |
| Claim expires | 2026-10-23T14:40:51Z |
| Created | 2026-10-22T09:31:41Z |
| Updated | 2026-10-23T12:40:51Z |

## Description
Serially refresh, independently review, and merge existing PRs #2636, #2641, #2652, #2648, #2650 in that order. Scope and specifications: each issue body and Rules in #2357; moves only, no logic changes or re-export shims or leftover #[path]. Merge origin/master (no rebase), preserve master's contents under relocated paths, verify layout ratchets/MIGRATED/PLACEMENTS, version master+1, push foreground timeout >=1500 seconds, label merge-requested, wait required CI, gh pr merge --merge. Retarget stacked child before parent merge. Human authorizes these five merges only. At most one building swarm member, never touch admission/broker or human checkout, no credential/environment reads. Fresh independent exact-head reviews and separate remediation swarms; escalate before third remediation round.
