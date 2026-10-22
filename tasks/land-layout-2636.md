<!-- quecto-task schema=1 -->
# Refresh, review and land L10 PR #2636

| Field | Value |
|---|---|
| Id | land-layout-2636 |
| Kind | task |
| Status | in_progress |
| Parent | land-layout-2357 |
| Accepted plan | 1 |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-22T10:45:29Z |
| Claim expires | 2026-10-22T12:45:29Z |
| Created | 2026-10-22T09:32:28Z |
| Updated | 2026-10-22T11:09:58Z |

## Description
Existing PR #2636, issue #2368, epic #2357. Merge origin/master into layout/2368-uds-framing-wire as merge commit, maintain moves-only transport layout into interface/uds/{framing,wire}, preserve master's content at moved paths, no shims/leftover #[path]. Check flat ratchet, MIGRATED and PLACEMENTS. Preserve red-first history. Run targeted issue gates; bump harness patch to master+1 Cargo.toml/Cargo.lock final commit; push foreground >=1500s. Fresh independent exact-head review (third review owed, earlier reviews stale); separate fix/re-review as necessary with escalation before third remediation round. Retarget #2641 to master before merging #2636. Label merge-requested for full CI, wait all required checks and merge with gh pr merge --merge. User explicitly authorizes merge. Shared machine: one building member maximum; no admission access or human checkout modifications.

## Pull requests
| Pull request |
|---|
| #2636 |
