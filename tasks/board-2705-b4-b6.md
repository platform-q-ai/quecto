<!-- quecto-task schema=1 -->
# Operator-owned B4: UDS board server

| Field | Value |
|---|---|
| Id | board-2705-b4-b6 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:46Z |
| Updated | 2026-10-24T07:16:54Z |

## Description
Operator-owned independent B4 for epic2705; legacy task id retains b4-b6, but B6 already merged as PR2714 and B1 as PR2715. Operator delivers quecto board-server over UDS: hyper listener, bounded methods/routes/body/headers/timeouts/connections, status/time/snapshot routes, peer-credential allowlist, and composition-only construction. It uses ProjectBoardStore and works with Git now and SQLite after B2; B3 is not its dependency. This agent consumes the merged B4 seam, never duplicates listener/route ownership. B7 must wait for this operator delivery to merge. Human-approved2713 production750target1500max, tests/fixtures/snapshots/lock/generated excluded/reported separately; size once planning/rejudge at remediation checkpoint or maximum exceeded. No live server/unit installation/start/deployment/cutover or protected playbook/relay/admission changes.

## Depends on
| Task |
|---|
| board-2705-b1 |
