<!-- quecto-task schema=1 -->
# Gate exit board composition on recorded starts (#2696)

| Field | Value |
|---|---|
| Id | issue-2696-exit-guard |
| Kind | task |
| Status | in_progress |
| Parent | issue-2696 |
| Accepted plan | 1 |
| Claimed by | SQ (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-22T00:09:31Z |
| Claim expires | 2026-10-22T02:09:31Z |
| Created | 2026-10-22T00:08:38Z |
| Updated | 2026-10-22T00:10:00Z |

## Description
Deliver a focused PR to master fixing issue #2696. Empty StartedRuns or no launcher must return empty session-end cleanup before environment capture, board composition, git/remote or store access and without warning. Preserve uncomposed-site behavior, started-run filtering/deduplication, stopped cleanup with session_end recording, read-failure warning and run_live/not_holder diagnostics. Use existing StartedRuns and site::left_runs seam; no domain/infrastructure redesign. Separate fresh red-test, delivery and independent review/remediation swarms. Red tests compile and fail for intended missing boundary guard, committed and handed off in verified SHA256 git bundle (never pushed red alone). Human merges, never agent. Target at most 500 changed lines, hard limit900, version bump in final commit, process artifacts external. Durable planning handoff: /home/swq/.quecto/artifacts/issue-2696/planning/handoff.md. All safety constraints from issue-2696 apply: one building member, no admission/broker or human-checkout changes, no credentials/environment disclosure, no global git config or complex rm -rf.
