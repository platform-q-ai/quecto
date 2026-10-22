<!-- quecto-plan schema=1 -->
# Plan 1 of 2634-listing

| Field | Value |
|---|---|
| Task | 2634-listing |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:13:36Z |
| Updated | 2026-10-22T00:13:36Z |

## Approach
Use distinct domain NonStopped scope, existing application listing query and presentation-only CLI with unlimited results; preserve Actionable scope. Baseline and full acceptance/commands in handoff.md I1 at /home/swq/.quecto/phase-artifacts/issue-2634-planning/. Estimate 220–350 lines. Fixed small red-test/delivery/review pools, member_limit 25; one building member; inventory owner serialized before kill, integrator owns wiring. Explicit plan approval and red-gap decision required: current characterization passes, new API tests do not compile baseline, spelling-only checks rejected.

## Task items
| Task item | Title |
|---|---|
| red | Establish compliant red evidence |
| implement | Delegate selection and preserve contracts |
| review | Independent PR review and CI readiness |

## Task item dependencies
| Task item | Depends on |
|---|---|
| implement | red |
| review | implement |

## Criteria
| Task item | Criterion |
|---|---|
| red | Resolve human red-strategy checkpoint |
| red | Baseline-compiling test fails for intended missing architecture behavior; bundle verified and checksummed |
| implement | Default/all table and JSON matrices retain non-stopped foreign Killing/CleanupFailed |
| implement | Numeric order, empty inventory, unlimited results, counts and diagnostics preserved |
| implement | Existing agent_cmd tests unchanged |
| review | Fresh exact-head review; no Actionable conflation or interface selection policy |
| review | Resolve findings and pass required CI; never merge |
