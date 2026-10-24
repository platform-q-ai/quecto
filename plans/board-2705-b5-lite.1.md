<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b5-lite

| Field | Value |
|---|---|
| Task | board-2705-b5-lite |
| Version | 1 |
| State | discarded |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:50Z |
| Updated | 2026-10-24T10:14:04Z |

## Approach
One serial PR after B7. Reuse bounded listener with Tailscale-only bind and hashed device tokens; route authorization allowlists and UDS-only administration. No members/follower scope. Red bundle, green implementation, fresh independent review and CI; hard900 all-in; never live-token/deploy changes.

## Task items
| Task item | Title |
|---|---|
| tokens | Device token lifecycle |
| tcp | Restricted TCP authorization |

## Task item dependencies
| Task item | Depends on |
|---|---|
| tcp | tokens |

## Criteria
| Task item | Criterion |
|---|---|
| tokens | Hashed tokens add/revoke/list with immediate revoke and safe output; principal distinct from asserted identity. |
| tcp | Only configured Tailscale IP bind accepted; no wildcard/public fallback. |
| tcp | Route/role authorization, malformed/missing token and bounds tests; UDS admin inaccessible over TCP. |
