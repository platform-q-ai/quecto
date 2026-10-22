<!-- quecto-plan schema=1 -->
# Plan 1 of issue-2696-exit-guard

| Field | Value |
|---|---|
| Task | issue-2696-exit-guard |
| Version | 1 |
| State | accepted |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Accepted by | SQ (`shane@platformq.co.uk`) |
| Accepted at | 2026-10-22T00:09:29Z |
| Created | 2026-10-22T00:09:02Z |
| Updated | 2026-10-22T00:09:29Z |

## Approach
At site::left_runs, affirmatively admit composed cleanup only when the existing launcher has nonempty StartedRuns; otherwise return empty before environment_of/build. Keep existing application selection and deduplication intact. TDD at tool_left_runs_tests boundary with isolated trap/counting builder; no production assertion on valid absent launcher. Preserve stopped outcome and session_end assertions. Separate red-test bundle phase (no behavior implementation), delivery phase, then fresh PR-first independent review and separate remediation if needed. One fixed small pool per phase, member_limit25; only one member builds. Baseline ef17b93e0d5058e07fdd3e54f05e6c86f3e31243; full handoff /home/swq/.quecto/artifacts/issue-2696/planning/handoff.md. Estimate109–255 lines including version; target500, max900. Targeted cargo lib tests for tool::left_runs_tests, application list_left_runs::tests, project_board; cargo fmt check and strict targeted clippy. Full suite in CI. Required crate patch bump in final commit, explicit paths staged, hooks intact; no process artifacts committed.

## Task items
| Task item | Title |
|---|---|
| red | Establish boundary regressions |
| green | Implement affirmative preflight |
| preserve | Verify started-session behavior |
| review | Publish focused PR and independently review |

## Task item dependencies
| Task item | Depends on |
|---|---|
| green | red |
| preserve | green |
| review | preserve |

## Criteria
| Task item | Criterion |
|---|---|
| red | Empty ledger and no launcher on composed failing/trap builder return no lines and invoke builder zero times. |
| red | Each new test compiles and fails on baseline for intended missing guard. |
| red | Preserve red commit unchanged in verified SHA256 bundle with base/head/commands/failure evidence; never push red alone. |
| green | Guard precedes environment capture and board composition. |
| green | New tests pass without git, live models, credentials or real stores. |
| green | Uncomposed site still returns empty. |
| preserve | Recorded starts retain unreadable warning, stopped cleanup and session_end record. |
| preserve | Filtering/deduplication and live/holder refusal tests preserved. |
| preserve | Targeted tests, fmt, strict clippy and applicable project gates pass sequentially. |
| review | Final commit includes required version bump; diff meets budget and no process artifacts. |
| review | PR first commit matches red handoff unchanged. |
| review | Fresh independent reviews cover exact head; findings explicitly resolved after separate fix passes. |
| review | No merge; label merge-requested only after clean independent review. |
