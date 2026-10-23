<!-- quecto-plan schema=1 -->
# Plan 1 of board-2705-b3

| Field | Value |
|---|---|
| Task | board-2705-b3 |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-23T16:49:38Z |
| Updated | 2026-10-23T16:49:38Z |

## Approach
One complete serial PR after B2. Source-grounded sizing on current predecessor; reuse SQLite raw file model. New history port and import/export use cases in application, adapters infrastructure, CLI/composition proper layers. Fresh red bundle phase before delivery; independent PR review then green CI/version/merge. Hard900 all changed lines; no live board operations.

## Task items
| Task item | Title |
|---|---|
| history | Import and export full history |
| wire | Compose CLI and rollback validation |

## Task item dependencies
| Task item | Depends on |
|---|---|
| wire | history |

## Criteria
| Task item | Criterion |
|---|---|
| history | Fixture linear history including set-aside/clear imports entirely and exports identical commit ids and bytes. |
| history | Raw git metadata preserved; malformed/unsupported history fails explicitly without partial import. |
| wire | New ports contracted/use cases registered; scratch CLI import/export and rollback documented/tested. |
| wire | All targeted tests/architecture gates green and current-head independent review/CI clean. |
