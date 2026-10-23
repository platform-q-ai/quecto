<!-- quecto-task schema=1 -->
# Make pre-commit file-size checks distinguish incoming master files in merges

| Field | Value |
|---|---|
| Id | merge-aware-precommit-file-cap |
| Kind | task |
| Status | draft |
| Created | 2026-10-23T18:25:38Z |
| Updated | 2026-10-23T18:25:38Z |

## Description
PR #2650 cannot commit the required current-master merge: scripts/pre-commit.sh unconditionally diffs index against old branch HEAD and calls incoming unchanged master quecto-agentic-harness/tests/architecture/uds_canonical_modules.rs a new818-line file, rejecting750 cap. Parent must not bypass hooks or weaken limits to land moves-only work. [Primary blocked source/receipt packet](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6371830074). Independently plan and RED-test merge-aware file ownership/baseline handling while preserving rejection of genuinely introduced oversized files, existing formatting/BDD checks and non-merge behavior. Separate authorization required before implementation or merging any hook-fix PR; alternatively consume a human-landed fix on master. Scope is not part of the authorized sessions moves-only PR.
