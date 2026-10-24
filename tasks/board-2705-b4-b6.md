<!-- quecto-task schema=1 -->
# B4 only: UDS board server (B6 operator-owned)

| Field | Value |
|---|---|
| Id | board-2705-b4-b6 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:46Z |
| Updated | 2026-10-24T00:12:03Z |

## Description
Agent MVP stage B4 ONLY after B3 for epic2705; legacy task id retains b4-b6 but B6 is NOT in scope. Bounded hyper UDS board-server listener with injected handlers, peer UID/permissions; interface status/time/snapshot routes and composition-only use-case construction. Allowlisted methods/routes,1MiB body,32headers/8KiB,10s header/body timeouts,64connections. Operator independently owns BoardOperations/LocalBoardOperations refactor B6; never implement or duplicate it in B4. Integrate agreed interfaces or transport seams without taking operator port ownership. No host server/unit installation/start or live deployment. Human-approved2713 target750/max1500PRODUCTION lines, excluded tests/fixtures/snapshots/lockfiles/generated separately reported, sized once planning/rejudged only remediation checkpoint or maximum exceeded. No future TCP/members/follower scope; all parent safety restrictions remain.

## Depends on
| Task |
|---|
| board-2705-b3 |
