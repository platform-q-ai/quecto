<!-- quecto-plan schema=1 -->
# Plan 2 of board-2705-b13-lite

| Field | Value |
|---|---|
| Task | board-2705-b13-lite |
| Version | 2 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-25T03:12:38Z |
| Updated | 2026-10-25T03:12:38Z |

## Approach
Deliver one complete reduced B13-lite PR now, independent of B7/B8. Accepted planning artifacts: /home/shane-pocket/.quecto/artifacts/b13-lite-planning/{parent-report.md,proposed-tasks.json,source-seams.md,red-handoff.md,size-forecast.md,safety-retention-fence-decisions.md}. Read them fully. parent-acceptance.md is authoritative and overrides planner deadlines, parent-alias policy and workflow wording. Forecast 1019–1447 production additions+deletions, target750/hard1500, tests excluded, sized once at planning. Do not repeatedly resize, golf, cut safety, or imply an exception. Only an actual unavoidable maximum breach triggers the parent's minimum complete backup/fence split. Planning base6d3e874/version446; refresh actual master and merged operator seams before RED and record the real pin.

Backup: exact `quecto board-server backup --to DIR` selects all sorted canonical16lowerhex.sqlite boards from existing B3 data root; optional --board-id narrows. No cwd inference, arbitrary source, source creation, token or serve graph. Recognized read-only schema, current state and bounded native/imported metadata admission; missing metadata is not native NULL. Per-board reports preserve partial successes and report all failures/unattempted targets. Use SQLite online backup API from a real pinned read transaction, not filesystem DB/WAL copy, mutating opener or immutable mode. Prove coherent WAL snapshot, independent restore and concurrent writer progress; integrity_check exactly one ok before publication. Parent copy120s/busy250ms/board300s/invocation1200s monotonic shared remaining budgets override planner5s/30s/120s; validation interrupt cancellation/join/cleanup on all paths. Inject short budgets in tests, never change CI timeouts. Bounds4096 directory entries/256 boards/256 candidates per board are explicit errors, not truncation/native-history caps.

Publish fresh board-YYYYMMDDTHHMMSSZ.sqlite each success, strict newer timestamp, no-replace collision/backward/future refusal. Private owned0700 leaf with trusted anchored ancestry and canonicalized legitimate existing parent aliases (including OS temp aliases), not blanket ancestor-symlink rejection. Canonical source/final symlinks/hardlinks, identity replacement, unsafe writable ancestry and source overlap still refuse. Cooperative per-board advisory lock; no malicious-sameUID authenticity/race guarantee. Owned0600 partial and sidecars, file fsync/atomic no-replace/directory fsync/identity-safe cleanup. Only after verified publication and prior candidate schema/meta/integrity/identity admission retain latest completed representative per UTC day for newest14 represented dates per board. Preserve other/unknown files and old healthy backups. Pruning failure keeps new completed snapshot with published=true/nonzero, publication uncertainty remains truthful.

Fence: offline explicit repo/board-id/ordered repeatable HTTP(S) URLs/optional remote, existing BoardMoved parser and exact ordered equality, sole merged2718 fence_tree owner unchanged. Thin publication in existing store_fence accesses private fetch/push/judge exact captured lease, not run-evidence GitRemoteRevisions. Already same destination creates no new fence objects/commit/push and needs no fresh commit identity lookup (fetch existing allowed). Different destination always typed Moved; malformed/unsupported/missing branch refuse, no bootstrap/repair. New one caller-authored/committed commit has captured head as sole parent, preserves history and existing builder's recognized-root/stray semantics. CAS winner/refusal/uncertain reconciliation remain distinct; no arbitrary force/rebase/refence/retry/unrelated ref movement. Ordinary reads AND writes return existing typed Moved. Every fence test uses owned scratch bare/local remotes, NEVER live board.

Proper thin inward maintenance ports/reports/capability-qualified errors/use cases, single SQLite backup resource-lifetime adapter/filesystem helper, existing Git publication owner, offline composition/allowlisted CLI. Reuse B3 schema/View/Reader/serializer/export/native and2718 ownership. No broad store-port expansion, duplicated interpreter/builder, B7/client/clock changes or listener/token access. Keep near743/744-line parent ports/store files minimal; coherent new files below750, no ratchet increases or inherited A body moves. Named real public-port contracts, sibling unit tests, constructor inventory. Preserve B3/A/B2/operator assertions; only enumerated old serve-or-token no-action strings evolve to complete action list plus positive backup/fence help; preserve existing import/export/serve/token positives.

Fresh per-PR RED, then a separate fresh delivery team. Fixed appropriate pools, member_limit25, at most one owned builder plus operator's one builder; no workflow/mutation tools or role rollover. Real core R1/R2 SQLite WAL/restore/writer and R6/R7/R8 Git publication/noop/elsewhere/CAS must compile and fail intended missing-behavior assertions, not merely unknown dispatch, uncomposed context, compiler, fixture, timeout or zero-match. Minimal named DTO/port/constructor/callable typed Unavailable shells only if core otherwise unreachable; no acceptance implementation/success fakes. Existing2718 builder/parser are passing regressions, not invented RED. R3 retention/R4 failure-publication-prune/R5 identity-path-clock-resource/R9 moved read-write/CLI preserve untouched state. Record fixture provenance, exact counts, shell inventory, failure depth and downstream assertions not executed. Seal verified UNPUSHED named-ref BASE..RED bundle/SHA256/base/redSHA/commands/controls for fresh delivery. Never push failing red. Every unsafe owned probe has its own hard SIGKILL/_exit timeout/core0/jobs1/memory cap/cleanup.

Targeted fmt/strictlint/architecture/docs/contracts/affected baseline controls locally; full suites in CI and mandatory trusted hook. Do not rerun long4102 per edit or alter its already-approved exact1200-only override. Swarms never activate workflow tools; any required bounded team adversarial source/test passes do not count as independent PR review. Foreground normal repo push hardtimeout at least1500/tool sufficient, no bypass/background, red+green together only when green; latestmaster/operator reconciliation, own LAST patch max(current,master)+1 separate manifest/lock, PR+merge-requested triggering/reset. Stop at verified published exacthead/base/environment/commands/red-first/retention/CAS/source evidence handoff. Parent owns fresh independent exact-head review and serial merge; after CIgreen only unresolved High/Medium block, Lows filed follow-ups without extra infrastructure RED cycles.

B7/T4/T5 operator-owned; B8 later after B13+B7 with end_run Git evidence OUTSIDE write lock and done_without_run PR facts pre-read as complete. Pocket later NODEPLOY. No live backup/import/export/fence/cutover/deploy/server/listener/unit install/start/relay/state/admission/credentials/env/globalconfig/protected-playbook/complicated removal operations. No human questions.

## Task items
| Task item | Title |
|---|---|
| b13-red | Fresh compiled real backup/fence RED and verified immutable handoff |
| online-backup | Consistent online backup with safe daily retention |
| fence-publication | Publish existing fence tree using exact remote CAS |
| offline-maintenance-cli | Inward composition and complete backup/fence commands |
| publish-b13 | Green PR and revision-bound durable handoff |

## Task item dependencies
| Task item | Depends on |
|---|---|
| online-backup | b13-red |
| fence-publication | b13-red |
| offline-maintenance-cli | online-backup |
| offline-maintenance-cli | fence-publication |
| publish-b13 | offline-maintenance-cli |

## Criteria
| Task item | Criterion |
|---|---|
| b13-red | Refresh actual master/2718 seams and read full authoritative parent policy; fixture/source pin and test matrix recorded. |
| b13-red | Core R1/R2 real SQLite WAL and R6/R7/R8 real Git publication/CAS compile and fail intended missing behavior, not CLI-only/setup failures; existing builder controls pass. |
| b13-red | Verified unpushed red bundle/SHA256/base/redSHA/commands/counts/controls/shell inventory/failure depth/source obligations passed to fresh delivery. |
| online-backup | Read-only recognized pinned SQLite API/coherent WAL/writer progress/independent restore/exact integrity ok; no writable opener, DB-WAL copying or missing source creation. |
| online-backup | Parent120s copy/250ms busy/300s board/1200s invocation monotonic interrupt cancellation/join/resource bounds/explicit per-board partial reports. |
| online-backup | Verified atomic no-clobber owned publication/identity+fsync/private leaf+legitimate canonical parent aliases/advisory lock; newest14 UTC daily keepers after prior admission, safe unknown files and truthful prune failure. |
| fence-publication | Reuse2718 BoardMoved/parser/builder and existing store private push/judge, explicit caller identity/captured remote head sole parent/history preserved. |
| fence-publication | Same ordered target Already without new fence objects/commit/push; other target typedMoved/malformed or missing refuses; ordinary read/write Moved preserved. |
| fence-publication | Scratch bare/local only: CAS winner/refusal/uncertain reconciliation and unrelated-ref invariants, no blind force/retry or live write. |
| offline-maintenance-cli | Thin ports/use cases/adapters/clock and owned lifetime; no duplicated B3/MOVED owners, named real port contracts/inventory/coherent files below750. |
| offline-maintenance-cli | Exact backup--to/all sorted boards or optional board-id; fence repo/id/ordered URLs/remote; allowlisted duplicate/missing options before I/O, typed JSON outcomes without token/listener graph. |
| offline-maintenance-cli | Preserve operator/B3 help and behavior except approved complete action-list additions; R1-R9 guards/legitimate aliases/clock/resources/fault paths and untouched-state assertions. |
| publish-b13 | All new tests/affected B3-A/B2 controls/applicable fmt-lint-architecture-docs-contracts green, no shells/weakened assertions; once-planned forecast1019–1447/hard1500. |
| publish-b13 | Normal trusted hook/latestmaster/operator/own LAST version/foreground at least1500 red+green push/PR/merge-requested/current CI evidence, no bypass. |
| publish-b13 | External exacthead/base/environment/commands/red-first/retention-CAS source obligations/limits accessible; stop published handoff, Parent fresh review/HM-only blocking aftergreen/Lows follow-ups, no live operations. |
