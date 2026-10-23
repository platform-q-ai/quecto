<!-- quecto-plan schema=1 -->
# Plan 1 of board-service-mvp-2705

| Field | Value |
|---|---|
| Task | board-service-mvp-2705 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:45:10Z |
| Updated | 2026-10-23T16:45:10Z |

## Approach
Deliver nine owner-approved serial MVP stages in minimum complete independently green PRs: B2, B3, B1 after #2648 merged, B4+B6, B7, B5-lite, B8, B13-lite, Pocket P1+P2. Revised handoffs stored externally at /home/shane-pocket/handover/board-epic/mvp-planning/. Earlier32-PR plan rejected. One PR per stage baseline; measured splitting only if unavoidable above900. B2 forecast825 approved above500 target, hard900 inclusive of tests/version. Separate fresh red, delivery and independent review swarms; compiling failing red commit handed off as verified bundle, never pushed alone. Version bump final, merge-requested CI, independent current-head review and resolved threads before serial merge. At most two building members. Stop before deployment/cut-over and preserve live systems unchanged.

## Task items
| Task item | Title |
|---|---|
| b2 | SQLite unchanged store contract |
| b3 | Full-history import and rollback export |
| b1 | Remote revision split and board location |
| b4-b6 | UDS board server and operations port |
| b7 | Transactional server operations and remote client |
| b5-lite | Tailscale listener and device tokens |
| b8 | Run lifecycle across wire |
| b13-lite | Fence, moved answer and backup |
| p1-p2 | Pocket relay and deploy artifacts |

## Task item dependencies
| Task item | Depends on |
|---|---|
| b3 | b2 |
| b1 | b3 |
| b4-b6 | b1 |
| b7 | b4-b6 |
| b5-lite | b7 |
| b8 | b5-lite |
| b13-lite | b8 |
| p1-p2 | b13-lite |

## Criteria
| Task item | Criterion |
|---|---|
| b2 | All15 store and9 every-kind reusable checks pass unchanged; WAL/FULL, atomic CAS, rollback, writer/quarantine/clear and crash durability tested. |
| b2 | Complete PR at most900; no B7 replay or optional cache expansion. |
| b3 | Scratch full history byte-exact import/export preserves commit ids; no live import. |
| b1 | #2648 merged before starting; --board-id/--url and GitRemoteRevisions in proper layers. |
| b4-b6 | Guarded bounded UDS listener; BoardOperations contracted and composed; no host install/start. |
| b7 | Per-board transaction and write_id result replay; remote client contract. |
| b5-lite | Tailscale-IP-only binding, per-device token authorization; members deferred. |
| b8 | Client launch facts and server start/end_run parity including rollback/failure. |
| b13-lite | Format4 fence/moved answer on scratch only; consistent online backup; live fence untouched. |
| p1-p2 | Relay UDS explicit committer; deploy/token templates and minute backup sync; no deployment. |
