<!-- quecto-task schema=1 -->
# Deliver SQLite board service MVP (#2705), stopping before cut-over

| Field | Value |
|---|---|
| Id | board-service-mvp-2705 |
| Kind | epic |
| Status | in_progress |
| Accepted plan | 1 |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-23T20:25:48Z |
| Claim expires | 2026-10-23T22:25:48Z |
| Created | 2026-10-23T15:48:15Z |
| Updated | 2026-10-23T20:25:48Z |

## Description
Deliver the owner-approved MVP of epic #2705. Authoritative inputs: owner header of ~/handover/board-epic/plan.md, then remaining plan and spike.md. Order: B2 SQLite store contract; B3 full-history import/export rollback; B1 GitRemoteRevisions and --board-id/--url only after #2648 merges; B4+B6 UDS server/BoardOperations; B7 transactional server operations and remote client; B5-lite Tailscale IP TCP/per-device tokens; B8 wire start/end_run; B13-lite format-4 fence/moved answer/online backup; Pocket P1+P2 relay UDS explicit committer/deploy units/tokens. Serial independently green PRs, 300–500 changed lines target, 900 maximum including tests; split where required. Test-first fresh red swarms, independent PR review and green CI before serial merges. Patch harness version before merge, merge-requested label, gh pr merge --merge. STOP before cut-over: never import live board, push live fence, install/start server or units, touch live relay/state/units or ~/.quecto/admission. No deployment. No PARENT_PLAYBOOK.md/COORDINATOR_PLAYBOOK.md changes. Spike branch reference only, never integrated. At most two building swarm members concurrently; pushes foreground with timeout >=1500s. Deliver merged MVP PRs and cut-over handoff; operator performs cut-over.
