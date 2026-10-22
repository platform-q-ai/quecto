<!-- quecto-plan schema=1 -->
# Plan 1 of 2634-run-end

| Field | Value |
|---|---|
| Task | 2634-run-end |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:13:41Z |
| Updated | 2026-10-22T00:13:41Z |

## Approach
Application/subagents/use_cases coordinator owns fixed wait/select policy with outcome and progress ports. Retain termination authority and detached cleanup, CLI formatting and flush/board sequencing. Full contract/commands: /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S3. Estimate 380–500 lines. Subagents owner plus integrator, fresh small phase pools capacity25, only one builder. Resolve plan agreement and semantic red gap first; future coordinator tests alone are not valid baseline reds. Hard SIGKILL timeouts for unsafe timing probes.

## Task items
| Task item | Title |
|---|---|
| red | Establish compliant baseline red tests |
| implement | Extract and wire settlement coordinator |
| review | Independent exact-head PR verification |

## Task item dependencies
| Task item | Depends on |
|---|---|
| implement | red |
| review | implement |

## Criteria
| Task item | Criterion |
|---|---|
| red | Explicit red-gap resolution; baseline-compiling intended-failure evidence |
| red | Verified red bundle and checksum |
| implement | 60s bound and 5s cadence deterministic tests |
| implement | Settled/unsettled/interrupted/timeout and no-fleet behavior retained |
| implement | Detached cleanup survives outer timeout; no locks across await |
| implement | Output flush and leftover-board sequencing unchanged |
| review | Probe timer boundaries without wall-clock sleeps or promised tie priority |
| review | No inward registry/runtime dependencies; clean threads and CI, no merge |
