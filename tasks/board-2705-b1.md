<!-- quecto-task schema=1 -->
# B1: GitRemoteRevisions split and explicit board location

| Field | Value |
|---|---|
| Id | board-2705-b1 |
| Kind | task |
| Status | draft |
| Parent | board-service-mvp-2705 |
| Created | 2026-10-23T16:47:34Z |
| Updated | 2026-10-23T16:47:34Z |

## Description
MVP stage3 epic #2705. Start ONLY after layout PR #2648 actually merges and B3 lands. Extract GitRemoteRevisions from GitBoardStore with contracted separate wiring. Add --board-id/--url board location/config precedence and explicit no-git-spawn path while preserving git compatibility. Owner --url overrides historical --remote-url. Baseline one independently green PR, target300–500 hard900 inclusive tests/version. Test new behavior red; extraction may use reviewed no-red rationale plus unchanged characterization. No spike integration, playbook edits or live deployment/cut-over; all parent safety restrictions apply.

## Depends on
| Task |
|---|
| board-2705-b3 |
| land-layout-2648 |
