<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b4-b6

| Field | Value |
|---|---|
| Task | board-2705-b4-b6 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:44Z |
| Updated | 2026-10-23T16:49:44Z |

## Approach
One serial PR after B1. Introduce contracted BoardOperations seam and injected UDS listener/routes, composition owns construction. Fresh compiling red bundle then green delivery and independent review/CI. Source-grounded all-in size before work, hard900; no host server/unit install/start.

## Task items
| Task item | Title |
|---|---|
| port | Contract local BoardOperations and rewire frontends |
| uds | Bounded guarded UDS board-server |

## Task item dependencies
| Task item | Depends on |
|---|---|
| uds | port |

## Criteria
| Task item | Criterion |
|---|---|
| port | CLI/tool use injected operations; unchanged local outcomes and registry/architecture gates. |
| uds | Injected handler routes status/time/snapshot, peer UID/permissions and allowlists enforced. |
| uds | Body/header/connection/time limits tested including valid cases; no infrastructure use-case construction. |
