<!-- quecto-task schema=1 -->
# Extract application run-end settlement wait

| Field | Value |
|---|---|
| Id | 2634-run-end |
| Kind | task |
| Status | draft |
| Parent | issue-2634-cli-policy |
| Created | 2026-10-22T00:12:49Z |
| Updated | 2026-10-22T00:12:49Z |

## Description
Issue #2634 concern 3. Move 60-second limit, 5-second cadence and settlement select policy into application/subagents/use_cases behind ports. Preserve detached cleanup authority, completion outcomes, output flush and board sequencing. Baseline ef17b93e0d5058e07fdd3e54f05e6c86f3e31243; estimate 380–500 changed lines including tests. Requirements/evidence: /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S3. No established baseline-compiling semantic red; explicit human decision required. Unsafe timing probes require hard SIGKILL timeouts.
