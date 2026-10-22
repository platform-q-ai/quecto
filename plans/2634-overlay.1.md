<!-- quecto-plan schema=1 -->
# Plan 1 of 2634-overlay

| Field | Value |
|---|---|
| Task | 2634-overlay |
| Version | 1 |
| State | draft |
| Proposed by | SQ (`shane@platformq.co.uk`) |
| Created | 2026-10-22T00:13:46Z |
| Updated | 2026-10-22T00:13:46Z |

## Approach
Pure write flag-to-layer mapping; PatchConfiguration sole eligibility owner; translate typed refusal for CLI wording only. Retain trust overlay-target helper. Handoff /home/swq/.quecto/phase-artifacts/issue-2634-planning/handoff.md S5 contains full contract/evidence. Estimate100–180 lines. Configuration owner plus integrator, fresh small phase pools capacity25; one builder. Plan approval and semantic red-gap resolution needed: duplication is same-layer and existing behavior tests pass; no spelling tests.

## Task items
| Task item | Title |
|---|---|
| red | Resolve red-first checkpoint |
| implement | Remove duplicated write preflight |
| review | Independent exact-head PR review |

## Task item dependencies
| Task item | Depends on |
|---|---|
| implement | red |
| review | implement |

## Criteria
| Task item | Criterion |
|---|---|
| red | Baseline-compiling semantic intended-failure evidence, not passing characterization or future API compile error |
| red | Verified bundle/checksum/commands |
| implement | Set/unset explicit-config and absent-cwd refusal matrices, no writes |
| implement | Global explicit-config success and SharedOverlay protections |
| implement | Trust explicit/implicit selection unchanged |
| implement | Characterize composition-error precedence; presentation not second policy |
| review | No error masking or path reread; retain trust helper |
| review | Resolve threads, clean review then CI; human merges only |
