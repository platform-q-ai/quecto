<!-- quecto-plan schema=1 -->
# Plan 1 of issue-2708

| Field | Value |
|---|---|
| Task | issue-2708 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Edited by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T21:15:30Z |
| Updated | 2026-10-23T23:11:15Z |

## Approach
Serial independently green slices: schema-validation dev prerequisite split from C1 first remediation, C1 typed declaration, C2 scoped liveness, C3 UDS protocol, P1 Pocket outcome policy, P2 Pocket negotiation/wiring, C4 UDS guard activation, C6 CLI-parent adapter. Preserve immutable red history and unresolved findings across splits; hard900 no exception. Each uses fresh red-test/delivery/independent-review/fix phases, PR-first review at exact head. Inactive C1 staging versus construction/catalogue CI invariants is under fresh source-backed planning; no blanket exemptions or production activation to satisfy tests. Pocket P1 source review clean but merge blocked absent established CI; draft2708-pocket-ci requires operator scope decision. Operator approves complete new runtime wording verbatim in PR bodies; no playbook text changes. No Jev/deployment/live-state access. Detailed planning/evidence in external /home/shane-pocket/.quecto/artifacts/issue-2708/; current-live.md and active-phases.md describe revision-bound handoffs. Duplicate-reply triage remains separate out-of-scope draft.

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
