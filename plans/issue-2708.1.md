<!-- quecto-plan schema=1 -->
# Plan 1 of issue-2708

| Field | Value |
|---|---|
| Task | issue-2708 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T21:15:30Z |
| Updated | 2026-10-23T21:15:30Z |

## Approach
Serial independently green slices C1 typed declaration, C2 scoped liveness, C3 UDS protocol, P1 Pocket outcome policy, P2 Pocket negotiation/wiring, C4 UDS guard activation, C6 CLI-parent adapter. Each uses separate red-test and delivery swarms; independent PR review begins only after PR opens, fixes use fresh swarms. No Jev or playbook changes, no deployment/live-state access. Exact proposed source plans and wording in durable artifact /home/shane-pocket/.quecto/artifacts/issue-2708/planning-428a8253/HANDOFF.md. Duplicate-reply triage is separate draft, not part of delivery.

## Task items
| Task item | Title |
|---|---|
| harness | Deliver deterministic parent outcome and liveness guard |
| pocket | Deliver Pocket needs-you projection without deployment |
| merge | Review and serially merge bounded PRs |

## Criteria
| Task item | Criterion |
|---|---|
| harness | One-line declaration waiting/blocked/asking/done; top-level only. |
| harness | Waiting/undeclared empty=>one system nudge=>stalled if still empty; blocked/asking=>needs_user(reason); done=>done. |
| harness | Actual live work includes descendants/swarms/pending wakes/timers/open question; child agents unaffected. |
| harness | UDS event contract documented and every branch tested, including original promise/no-launch. |
| pocket | stalled/needs_user display needs-you with reason and phone notification, never done. |
| pocket | Explicit done unchanged; no quiet/EOF false completion or replay notification. |
| pocket | No deployment or live relay/state/units/admission access. |
| merge | Tests first, fresh independent review current exact head, validated threads resolved and required CI green. |
| merge | Target300–500 lines/max900 each, all new wording verbatim in PR. |
| merge | Patch bump just before quecto merge; merge-requested label; gh pr merge --merge; refresh moving master. |
| merge | At most2 building members; foreground git push timeout>=1500s. |
