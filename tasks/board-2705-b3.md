<!-- quecto-task schema=1 -->
# B3: Full-history import and rollback export

| Field | Value |
|---|---|
| Id | board-2705-b3 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:22Z |
| Updated | 2026-10-23T16:47:22Z |

## Description
MVP stage2 epic #2705. Full-history import from git board branch into SQLite plus byte-exact rollback export preserving identical commit ids, raw object/header metadata, quarantines and clears. Proper application ports/use cases and composition; scratch fixtures only, no live import/export/cut-over. Baseline one complete independently green PR; target300–500 hard900 changed lines including tests/version. Refine sizing on actual predecessor head. Owner revised plan externally at /home/shane-pocket/handover/board-epic/mvp-planning/board-plan-revised.json. Parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b2 |
