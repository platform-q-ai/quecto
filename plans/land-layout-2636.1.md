<!-- quecto-plan schema=1 -->
# Plan 1 of land-layout-2636

| Field | Value |
|---|---|
| Task | land-layout-2636 |
| Version | 1 |
| State | accepted |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Accepted by | SQ (`shane@platformq.co.uk`) |
| Accepted at | 2026-10-22T09:34:47Z |
| Created | 2026-10-22T09:34:36Z |
| Updated | 2026-10-22T09:34:47Z |

## Approach
Refresh the existing reviewed layout slice rather than redesign it. Preserve original red-first commits and move-only ownership. Merge current master as a merge commit, reconcile new master content at relocated paths, and validate architecture ratchet/placements and targeted gates. Publish version master+1 as final commit. Independently review the exact refreshed PR head in a fresh checkout, remediate separately if needed, then retarget child #2641 before merging with green required CI and resolved conversations.

## Task items
| Task item | Title |
|---|---|
| refresh | Merge master and validate move-only preservation |
| publish | Publish refreshed versioned head |
| review | Independently review exact published head |
| merge | Retarget child and serially merge after CI |

## Task item dependencies
| Task item | Depends on |
|---|---|
| publish | refresh |
| review | publish |
| merge | review |

## Criteria
| Task item | Criterion |
|---|---|
| refresh | origin/master is merged without rebase |
| refresh | Master changes follow every moved path without logic changes or shims or leftover #[path] |
| refresh | Ratchet, MIGRATED, PLACEMENTS and applicable targeted gates pass |
| refresh | Mechanical moves and substantive edits are separately measured |
| publish | Final commit bumps harness Cargo.toml and its Cargo.lock package to master patch+1 |
| publish | Foreground push completes with timeout at least 1500 seconds |
| review | Fresh independent swarm validates source and focused probes |
| review | All actionable findings have explicit resolution and current-head verification |
| review | Escalation occurs before third remediation round |
| merge | #2641 base is master before parent merge |
| merge | merge-requested starts required CI and all required checks pass |
| merge | PR merges with gh pr merge --merge |
