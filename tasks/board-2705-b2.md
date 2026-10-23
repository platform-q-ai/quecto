<!-- quecto-task schema=1 -->
# B2: SQLite board store passing the unchanged contract

| Field | Value |
|---|---|
| Id | board-2705-b2 |
| Kind | task |
| Status | ready |
| Parent | board-service-mvp-2705 |
| Accepted plan | 1 |
| Created | 2026-10-23T16:45:47Z |
| Updated | 2026-10-23T16:46:31Z |

## Description
First MVP deliverable for epic #2705: complete SQLite ProjectBoardStore with unchanged15 store and9 every-kind reusable contract checks; WAL and synchronous FULL, atomic CAS/rollback, bootstrap/writer/quarantine/clear/raw-byte parity, essential crash durability. Infrastructure persistence/sqlite/project_board owns SQL; reuse existing codecs/snapshot helpers; composition-only use-case construction. No public port changes, server/remote behavior, history implementation, B7 request replay or optional cache expansion. Source planning base1ad426002e40bec5da0d2d65079cf521cd6b3205; revised scoped handoffs at /home/shane-pocket/handover/board-epic/mvp-planning/. Owner permits one complete PR forecast825 above500 target, absolute900 changed lines including red tests/version; split only on concrete unavoidable evidence. Spike4143fbafab754156d9c6777a46729dd74ea843ec reference only, never integrated. Fresh red bundle then green delivery, independent current-head review and CI before merge. No live board import/fence/deployment/server start or units, playbook edits, relay/admission/credential access. At most two building members; pushes foreground timeout at least1500s.
