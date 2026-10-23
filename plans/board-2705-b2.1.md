<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b2

| Field | Value |
|---|---|
| Task | board-2705-b2 |
| Version | 1 |
| State | accepted |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted at | 2026-10-23T16:46:31Z |
| Created | 2026-10-23T16:46:18Z |
| Updated | 2026-10-23T16:46:31Z |

## Approach
RED-TEST PHASE ONLY for one complete B2 PR. Fresh swarm member_limit25 with fixed small pool, at most two building members. Scope task/parent descriptions and owner-approved revised B2 requirements: SQLite unchanged ProjectBoardStore, all15 store+9 every-kind reusable contract checks, WAL/FULL, atomic CAS/rollback/writer/quarantine/clear/raw bytes and essential owned-child durability. Baseline refresh origin default and record exact SHA; planning baseline1ad426002e40bec5da0d2d65079cf521cd6b3205. Read current port/contracts and proper infrastructure persistence/sqlite/project_board layers; reuse canonical existing codecs/shared check bodies. Do not integrate spike or implement acceptance behavior. Minimal typed-error trait skeleton/test fixtures allowed solely to compile real tests; red must be intended assertion failure, not compiler/fixture error. No ignored/weakened tests. All-in B2 hard900 lines including tests/version, forecast825 approved above500 target. No private request replay/cache optimization. No server, deployment, live board import/fence, host unit/server start, relay/admission/credential reads, env printing, playbook edits, complicated rm or global git config. Red commit never pushed: create and verify git bundle from exact base to branch; record SHA256, red/base SHAs, selectors/counts/exits/failure excerpts in external handoff. Artifacts must be durable/accessibly exported before cleanup. Finish on verified red handoff; no delivery implementation or waiting to publish.

## Task items
| Task item | Title |
|---|---|
| red-contract | Compile unchanged reusable SQLite contract harness and prove red |
| red-handoff | Commit and bundle red tests for fresh delivery swarm |

## Task item dependencies
| Task item | Depends on |
|---|---|
| red-handoff | red-contract |

## Criteria
| Task item | Criterion |
|---|---|
| red-contract | Wire all15 store+9 every-kind existing shared checks using two independent tempfile DB clients and minimal skeleton/fixture seams; no copied/weakened contract assertions. |
| red-contract | Targeted commands compile and fail intended bootstrap/write/clear behavior assertions, not absent symbols; record nonzero test counts and exact failures. |
| red-contract | Supplement WAL/FULL, atomic rollback and essential durability red checks where feasible; if baseline passes or cannot run escalate parent, never invent failure. |
| red-handoff | Red commit includes no acceptance implementation and is not pushed. |
| red-handoff | Verified git bundle base..branch with SHA256, exact red/base/branch, command logs and failure evidence exported externally. |
| red-handoff | Record actual changed-line inventory and remaining B2 headroom; no mandatory scope omitted. |
