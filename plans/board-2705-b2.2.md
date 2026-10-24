<!-- quecto-plan schema=1 -->
# Plan 2 of board-2705-b2

| Field | Value |
|---|---|
| Task | board-2705-b2 |
| Version | 2 |
| State | superseded |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted at | 2026-10-23T17:03:00Z |
| Created | 2026-10-23T17:02:48Z |
| Updated | 2026-10-24T00:14:44Z |

## Approach
GREEN DELIVERY ONLY, fresh swarm member_limit25/task-appropriate fixed pool, at most two building members. Parent supplies base64 red bundle in next message; wait for it, persist outside repo, verify SHA256 ce36929ddc2c6302544e394e4e8c6f143b73c7ed1707c762343cd21db9bc1640 and git bundle verify with base1ad426002e40bec5da0d2d65079cf521cd6b3205, start branch unchanged at red43ed52f76340ab44bfe6e9288252e411b114609d. Never amend/rebase/squash red commit. Implement complete unchanged ProjectBoardStore in infrastructure persistence/sqlite/project_board, all15 store+9 every-kind reused checks unchanged. Red29 tests1 passed/28 failed intended runtime assertions, no compile failures; baseline contracts green. Red407 added lines,493 headroom incl implementation/deletions/version under hard900. Reuse codecs/folder snapshot helpers; affirmative guards/proper layers; no B7 replay or optional cache expansion. Mandatory parity: WAL/FULL each connection, atomic CAS/bootstrap/clear and midtransaction rollback, invalid/unknown head/error/bounds semantics, raw canonical BLOB byte proof, quarantines/strays/repeated names/clear, writer provenance, all6 kinds, acknowledged child-exit reopen plus uncommitted owned-child crash rollback under hard timeout. Preserve tests; request parent decision on unavoidable size or test changes. Targeted tests and applicable lint/architecture/docs/contracts local; full suites CI. Push foreground timeout at least1500 seconds once green, red+green together, open PR before independent review; do not merge or claim independent verification. Patch version final before handoff, merge-requested label triggers CI. No playbook edits/live import/fence/cutover/deploy/server or units on host/eccup, live relay/state/units/admission/credentials/env output, complicated rm/global git config. Export durable revision-bound handoff and logs outside repo before complete.

## Task items
| Task item | Title |
|---|---|
| green-store | Implement full contract-conforming transactional SQLite store |
| green-pr | Verify targeted gates and publish complete B2 PR |

## Task item dependencies
| Task item | Depends on |
|---|---|
| green-pr | green-store |

## Criteria
| Task item | Criterion |
|---|---|
| green-store | Verified red bundle/hash/base and unchanged first red commit. |
| green-store | All24 reusable contract checks plus SQLite supplementary assertions green; no skeleton/unimplemented/ignored/weakened tests. |
| green-store | WAL/FULL, atomic rollback/CAS and essential owned-child crash recovery proven; raw bytes and all error/quarantine/writer paths preserve contract. |
| green-pr | Actual all-in changed lines at most900 incl version/tests/deletions; generated/mechanical counts separately, no process artifacts in diff. |
| green-pr | Targeted contracts/architecture/applicable lint green, exact commands/logs recorded; push foreground timeout at least1500s with no backgrounding. |
| green-pr | PR opened with red-first evidence, requirements/limitations/CI label and exact head; no merge or independent review within delivery. |
