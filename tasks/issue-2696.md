<!-- quecto-task schema=1 -->
# Fix spurious board run warning at session exit (#2696)

| Field | Value |
|---|---|
| Id | issue-2696 |
| Kind | epic |
| Status | review |
| Accepted plan | 1 |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-22T01:31:06Z |
| Claim expires | 2026-10-22T03:31:06Z |
| Created | 2026-10-21T23:57:24Z |
| Updated | 2026-10-22T01:33:15Z |

## Description
[GitHub issue #2696](https://github.com/platform-q-ai/quecto/issues/2696): one-shot sessions outside a git repository warn that board runs could not be read even when they never started runs. Sessions recording no start_run must finish without touching git for board cleanup; sessions that did start runs retain appropriate cleanup and read-failure warnings. Deliver a focused PR to master with separate planning, red-test bundle, implementation, and independent review/remediation. Human merges; do not merge. Scope excludes unrelated board changes. Tests use TDD, targeted local gates and full CI. No more than one building swarm member concurrently; never modify the human checkout, admission configuration or broker; no credential/environment disclosure. Artifacts and handoffs live externally, not in PR.
