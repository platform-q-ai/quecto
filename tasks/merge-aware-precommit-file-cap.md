<!-- quecto-task schema=1 -->
# Make pre-commit file-size checks distinguish incoming master files in merges

| Field | Value |
|---|---|
| Id | merge-aware-precommit-file-cap |
| Kind | task |
| Status | ready |
| Created | 2026-10-23T18:25:38Z |
| Updated | 2026-10-23T18:30:12Z |

## Description
User authorized standalone small PR to master and parent merge, then resume #2650 with normal hooks. In a merge commit, a candidate staged file present in MERGE_HEAD with identical content must not count as newly introduced for pre-commit750-line limit; genuinely new files retain limit. Add behavioral hook test or fixture for both cases, independently review exact published head, green required CI, and merge. Preserve normal commits, changed incoming files, fail-closed unknown states and all other hook gates; no hook bypass/spoofed index or broad suppression. Primary blocker: unchanged818-line master tests/architecture/uds_canonical_modules.rs prevented #2650 master merge. [Preserved evidence](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6371830074). Prefer separate fresh RED tests exercising actual staged-content behavior, test-only commit durable verified bundle; user permits reasoned no-red if no harness exists, parent must document. Target500/hard900 incl tests, master+1 own final harness version-only commit, foreground push timeout1500 minimum, merge-requested full CI, fresh independent review, required threads resolved, parent authorized merge only after exact-head green. Then refresh #2650 with fixed normal hook, preserve moves-only scope and remove all inherited moved-scope path attributes without changing Rust test module IDs/assertions/content.
