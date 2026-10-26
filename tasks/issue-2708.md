<!-- quecto-task schema=1 -->
# Prevent silent parent stalls and surface actionable outcomes in Pocket

| Field | Value |
|---|---|
| Id | issue-2708 |
| Kind | epic |
| Status | in_progress |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-26T00:48:12Z |
| Claim expires | 2026-10-26T02:48:12Z |
| Created | 2026-10-23T20:48:14Z |
| Updated | 2026-10-26T00:48:12Z |

## Description
Deliver platform-q-ai/quecto#2708 per owner decision 2026-10-23: deterministic parent turn-outcome declaration and one-nudge liveness guard, UDS stalled/needs_user/done events with documentation, and companion quecto-pocket relay mapping stalled/needs_user to needs-you with reason and notification (never done). No Jev, playbook edits, deployment, live relay/state/units or admission changes. Tests first in separate red-test swarms; fresh independent PR review and fixes. PRs target 300–500 lines, max900, merged serially after CI; quecto patch bumped immediately before merge; refresh against moving master. List new agent-facing and user-facing wording verbatim in each PR. At most two building members simultaneously. Source: [issue and comments](https://github.com/platform-q-ai/quecto/issues/2708).
