<!-- quecto-task schema=1 -->
# B3: Full-history import and rollback export

| Field | Value |
|---|---|
| Id | board-2705-b3 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:22Z |
| Updated | 2026-10-24T07:55:07Z |

## Description
MVP stage 2 of epic #2705, following merged B2: complete bounded linear-history import and complete rollback export. Preserve every original imported raw commit/tree/blob, header and ID, quarantines and clears; append valid deterministic Git commits for all later native SQL revisions. Use proper inward ports, ImportBoardHistory/ExportBoardHistory use cases, sole shared Reader, normalized SQLite storage, and composition. Include required lineage/epochs with epoch:revision heads. Proposed planning package: /home/shane-pocket/.quecto/artifacts/b3-readonly-planning/{parent-report.md,proposed-plan.json,known-scope.md,fresh-phase-handoff.md,red-plan.md}. Source basis master71efd677 plus B2dd01f3 and declared R2 delta; actual merged B2 and operator command seams must be reconciled before fresh RED. Size policy is current accepted epic plan, superseding historical 300–500/900 all-in wording; source forecast975–1420 production additions+deletions, tests/fixtures separately reported, not a proven ceiling. Operator owns B4/shared board-server root; only additive offline import/export handlers/registration here. Scratch-only acceptance: no live import/export/fence/cutover/deploy/server/unit installation or start. Spike reference only, never integrate. Fresh RED, fresh delivery and fresh independent exact-head PR review remain separate.

## Depends on
| Task |
|---|
| board-2705-b2 |
