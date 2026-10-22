<!-- quecto-plan schema=1 -->
# Plan 1 of 2634-setup-paths

| Field | Value |
|---|---|
| Task | 2634-setup-paths |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:13:44Z |
| Updated | 2026-10-22T00:13:44Z |

## Approach
Environment preparation request/use case and host-path port; infrastructure fs/env adapter; composition bridges configuration selection and init/status without concrete cross-capability leakage. Remove direct and helper-reachable setup IO. Full contract /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S4. Target450–500, provisional650. Preflight over target splits consumed preparation/bridge250–350 then final integration250–350. Setup owner plus integrator; fresh small phase pools capacity25; one builder. Explicit plan approval/red-gap resolution required.

## Task items
| Task item | Title |
|---|---|
| red | Resolve red gap and preflight sizing |
| implement | Prepare host paths via port and wire setup |
| review | Independent exact-head PR verification |

## Task item dependencies
| Task item | Depends on |
|---|---|
| implement | red |
| review | implement |

## Criteria
| Task item | Criterion |
|---|---|
| red | Compliant baseline-compiling intended-failure evidence; verified red bundle |
| red | Choose complete production-consumed slices if estimate exceeds500 |
| implement | Explicit-config refusal before IO; project overrides cwd |
| implement | Canonical symlink identity and project layer selection preserved |
| implement | Unknown cwd/project and current_dir errors preserved |
| implement | Absolute nonexistent base valid; root/absolute defenses and no-write behavior intact |
| implement | No direct or indirectly reachable setup fs/env IO |
| review | Probe symlinks, fallback and no-write refusals |
| review | No global CliContext churn, relative prepared base or trust identity drift |
| review | Threads resolved and CI clean; no merge |
