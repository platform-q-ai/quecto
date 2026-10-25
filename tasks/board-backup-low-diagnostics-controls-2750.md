<!-- quecto-task schema=1 -->
# Backup Low follow-up: missing-leaf diagnostics and permanent root-error controls (#2750)

| Field | Value |
|---|---|
| Id | board-backup-low-diagnostics-controls-2750 |
| Kind | task |
| Status | draft |
| Created | 2026-10-25T16:23:37Z |
| Updated | 2026-10-25T16:23:37Z |

## Description
Explicitly accepted/deferred non-blocking Low follow-ups from independent PR #2747 re-review at 8713a2f064f44cbf2b74c1009e2e70ac52d54407. GitHub issue [#2750](https://github.com/platform-q-ai/quecto/issues/2750) holds validated scenarios and acceptance.

\(1) Existing selected default ID directory with missing board.sqlite3 returns unsafe_path/admission instead of equivalent explicit empty-state unknown_board: narrowly correct selected-leaf NotFound diagnostics with red-first topology/inventory coverage, retaining every other admission guard. (2) Current non-NotFound regular-file/symlink/unreadable-root behavior is correctly UnsafePath; add permanent selected/bulk/privilege-aware permission controls falsifying blanket error classification and proving unchanged source/destination inventories. Preserve original 54/state-eight/four appended cases and all safety watchdogs.

Findings: [missing leaf](https://github.com/platform-q-ai/quecto/pull/2747#discussion_r4371487373) and [root-error controls](https://github.com/platform-q-ai/quecto/pull/2747#discussion_r4371487376). Independent [review](https://github.com/platform-q-ai/quecto/pull/2747#pullrequestreview-5644095996); durable primary evidence at /home/shane-pocket/.quecto/artifacts/b13-backup-round1-rereview/parent-report.md and prober reports.

Parent accepts both deferred Lows under owner policy that only unresolved High/Medium findings block after CI green. Neither fix is claimed in the MVP PR. Future separately scoped planning, test-first, applicable gates, and exact-head independent review are required. This is not an MVP dependency or authorization for immediate implementation. Related epic #2705 and board task board-2705-b13-backup. No live operations, deployment, protected-playbook, relay/admission, or credential access. Operator owns CI issue #2748; exclude it.
