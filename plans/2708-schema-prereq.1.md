<!-- quecto-plan schema=1 -->
# Plan 1 of 2708-schema-prereq

| Field | Value |
|---|---|
| Task | 2708-schema-prereq |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T23:03:56Z |
| Updated | 2026-10-23T23:03:56Z |

## Approach
Isolate dev-only schema validation dependency/lock infrastructure from C1 so both final PRs fit hard900 without weakening coverage. Red-test phase adds lasting dependency/configuration check over actual Cargo metadata graph, compiling on baseline and failing for absent direct dev dependency. Green adds only dev dependency/generated lock, leaves runtime unchanged, and validates metadata plus relevant architecture/contract/format gates. Exact schema behavioral regression remains unpushed immutable red in C1 fix bundle. Parent split decision records carried findings/remediation history. Fresh independent PR review then finalpatchbump/CI/serialmerge.

## Task items
| Task item | Title |
|---|---|
| dev-schema | Provide schema-validator dev dependency and lasting config contract |

## Criteria
| Task item | Criterion |
|---|---|
| dev-schema | Actual Cargo metadata graph confirms direct harness dev dependency, absent on baseline and behavioral/config red rather than unresolved import. |
| dev-schema | No production dependency/runtime/tool activation; generated lock changes reported separately. |
| dev-schema | Target300–500/max900 total, no padding or weakened tests. |
| dev-schema | Separate verified redbundle, green PR, fresh exact-head review and fullCI; no new agent/user wording, patchbump final convention. |
| dev-schema | C1 unresolved schema/inventory findings and all originalredcommits preserved for subsequent fresh fix. |
