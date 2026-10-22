<!-- quecto-task schema=1 -->
# Refresh, review and land L11 PR #2641

| Field | Value |
|---|---|
| Id | land-layout-2641 |
| Kind | task |
| Status | in_progress |
| Parent | land-layout-2357 |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-22T12:08:33Z |
| Claim expires | 2026-10-22T14:08:33Z |
| Created | 2026-10-22T12:08:26Z |
| Updated | 2026-10-22T12:08:40Z |

## Description
Existing PR #2641, issue #2369 + Rules epic #2357. Already retargeted master before #2636 merged as d96847098fbb63a14cd40e6d8327814cbe636679 (.423). Merge current origin/master into layout/2369-uds-controllers (no rebase), preserve master's content at relocated paths, moves-only no production logic/re-export shims/leftover #[path]. Check flat-file ratchet/MIGRATED/PLACEMENTS, preserve red-first history; targeted issue gates. Final own chore version master+1 Cargo.toml/own Cargo.lock; foreground push timeout>=1500s, label merge-requested, fresh independent exact-head review/fix/re-review, required CI green. Retarget #2652 master before merging #2641 with gh pr merge --merge, user-authorized. One building member max; no admission access/human checkout modifications. Remediation history starts anew for this PR; follow session escalation rules before third remediation. Durable artifacts outside tracked diff; mechanical size separately reported.

## Depends on
| Task |
|---|
| land-layout-2636 |
