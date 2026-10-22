<!-- quecto-plan schema=1 -->
# Plan 1 of issue-2634-cli-policy

| Field | Value |
|---|---|
| Task | issue-2634-cli-policy |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:13:05Z |
| Updated | 2026-10-22T00:13:05Z |

## Approach
Five independently green behavior-preserving PR slices, based on master ef17b93e0d5058e07fdd3e54f05e6c86f3e31243. Detailed requirements, source evidence, commands and proposed role allocation: /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md and propose-plan.json. Target at most 500 changed lines per PR, maximum 900; setup provisional upper 650 requires preflight sizing/splitting. Serialize listing then kill for shared controller ownership; others independent. Explicit user plan agreement and resolution of semantic red-first gaps are required before implementation. Execution uses fresh red-test, delivery and independent review swarms, fixed small pools, capacity 25, only one building member at a time. Integrator owns shared wiring; separate inventory, subagents, setup and configuration ownership. No merges; human merges. Clean independent reviews precede merge-requested CI label.

## Task items
| Task item | Title |
|---|---|
| listing | Shared listing policy |
| kill | Application forgotten termination and runtime adapter |
| run-end | Application settlement wait policy |
| setup | Setup host-path port |
| overlay | Single write eligibility owner |

## Task item dependencies
| Task item | Depends on |
|---|---|
| kill | listing |

## Criteria
| Task item | Criterion |
|---|---|
| listing | Deliver task 2634-listing with preserved CLI and agent_cmd behavior |
| kill | Deliver task 2634-kill with trusted restoration provenance and error precedence |
| run-end | Deliver task 2634-run-end with detached cleanup and bounded timing preserved |
| setup | Deliver task 2634-setup-paths with direct and indirect IO removed |
| overlay | Deliver task 2634-overlay without changing trust behavior |
