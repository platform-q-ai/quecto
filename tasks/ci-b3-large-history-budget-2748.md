<!-- quecto-task schema=1 -->
# Fix the CI budget mismatch for the 4,102-revision B3 acceptance test (#2748)

| Field | Value |
|---|---|
| Id | ci-b3-large-history-budget-2748 |
| Kind | chore |
| Status | archived |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-25T14:30:53Z |
| Updated | 2026-10-25T14:46:08Z |

## Description
Owner-authorized follow-up in this delivery lane, preferably folded into the next planned MVP PR to avoid an extra PR: issue platform-q-ai/quecto#2748. The existing B3 4,102-revision native export/cache-eviction acceptance test has nextest20minute termination allowance while Workspace Tests has25minute overall job timeout and about6minutes other work, causing cancellation without a failing test on slower runners. Preserve complete greater-than4096 export and actual cache-eviction coverage, assertions and a hard bound; never silently skip/drop coverage or broadly relax test timeouts. Choose the smallest effective fix after inspecting workflow: dedicate an enforced job/nightly coverage, make the fixture faster without losing both behaviors, or align the job envelope with its unchanged bounded test per issue's options. Add a lasting configuration/coverage contract that fails baseline for the mismatch, and characterize preserved coverage. For current backupPR2747, owner explicitly says rerun Workspace Tests if cancelled with no failing test, rather than treating it as a backup regression; do not interrupt an in-progress job. Track exacthead CI outcome and ensure all required checks green before merge. Do not change already-reviewed backup head merely to address this opportunistic follow-up unless repeated CI cancellation actually blocks delivery or another validated fix pass already changes that head. No live operations, new infrastructure deployment, protected-playbook/relay/admission/credential access. Keep source/config size and test-first requirements; independent review required. Full issue read2026-10-25T14:27:40Z; example cancelled runs40200088818(PR2746),40173615363(B7).
