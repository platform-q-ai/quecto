<!-- quecto-task schema=1 -->
# Assess staged versus worktree content in pre-commit file-size enforcement

| Field | Value |
|---|---|
| Id | precommit-staged-size-accounting |
| Kind | task |
| Status | draft |
| Created | 2026-10-23T18:39:28Z |
| Updated | 2026-10-23T18:39:28Z |

## Description
Planning for authorized merge-aware new-file exemption found pre-existing size checks count worktree lines while selecting staged A files. A genuinely new staged751-line Rust file with worktree1 line currently passes; staged1/worktree751 fails. [Actual-hook probe evidence](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6372101349). The authorized merge-aware fix uses staged identity only for precise incoming-parent exemption and intentionally preserves existing line-count semantics. Independently assess and plan a separate behavior correction if desired, including actual-hook adversarial fixtures, unchanged other gate behavior and explicit scope approval. No implementation is authorized by the current bounded merge-exemption task.
