<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-pocket-p1-p2

| Field | Value |
|---|---|
| Task | board-2705-pocket-p1-p2 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:50:00Z |
| Updated | 2026-10-23T16:50:00Z |

## Approach
One serial quecto-pocket PR after harness MVP stages. Inspect actual Pocket source and minimize combined relay/deploy diff; red-first fresh swarms and independent exact-head review. Backend allowlist supports git compatibility; server path UDS explicit committer/no clones. Deploy templates and minute backup sync only, DO NOT deploy. Hard900 all-in.

## Task items
| Task item | Title |
|---|---|
| relay | Relay board service adapter |
| templates | Deployment and rollback artifacts |

## Task item dependencies
| Task item | Depends on |
|---|---|
| templates | relay |

## Criteria
| Task item | Criterion |
|---|---|
| relay | Server backend UDS command has board ID/explicit owner via pocket committer and no git identity scope/clone. |
| relay | Git compatibility backend and answer parsing unchanged; minimum harness version correct. |
| templates | Service/token templates and existing consistent-backup sync cadence every minute tested/documented. |
| templates | No install/start/live relay/state/unit changes; eccup backup recipient not follower. |
