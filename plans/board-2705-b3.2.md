<!-- quecto-plan schema=1 -->
# Plan 2 of board-2705-b3

| Field | Value |
|---|---|
| Task | board-2705-b3 |
| Version | 2 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-24T07:56:03Z |
| Updated | 2026-10-24T07:56:03Z |

## Approach
Source-grounded proposed complete B3 from fresh read-only planning swarm, evidence revision b3-restored-linear-bounded-M71efd677-Sdd01f3-final1. Authoritative package /home/shane-pocket/.quecto/artifacts/b3-readonly-planning/{parent-report.md,proposed-plan.json,known-scope.md,architecture.md,source-map.md,sizing.md,red-plan.md,refutation.md,fresh-phase-handoff.md}. Owner header/full plan/spike/task inputs were restored and independently read. Actual merged B2/R2 and operator B4 shared command seams require reconciliation before a fresh red phase. This is a proposed plan, not implementation evidence.

Deliver one complete independently-green PR if actual production additions+deletions remain within the accepted epic size policy. Forecast975–1420 production including reserve; tests900–1700, fixture text150–450, snapshots0–150 separately reported; no proven implementation ceiling. Do not delete rollback/epochs/CLI or compress safety to fit. Parent decides if measured unavoidable size or remediation requires decomposition.

Import only the entire selected supported LINEAR refs/heads/quecto/board ancestry and recursive actual tree/blob closure from a supplied complete local clone. Capture H0 once, configured remote is provenance only, no hidden fetch/freshness claim. Preserve original SHA1/SHA256 raw commits/headers/trees/blobs/IDs and opaque archive/quarantine/clear tokens. Follow actual blob/tree kinds on mode mismatch, sole Reader decides interpretation. Gitlinks remain foreign references. Merge/nonlinear/missing/shallow/malformed/unsupported sources refuse wholly. Proposed explicit caps:4096 commits,32768 unique blob/tree objects,128MiB total raw including commits,64MiB/blob,64KiB/commit,655360 raw-tree bytes,65536 projected entries,32MiB checked metadata,4096 raw-name bytes,depth128. No truncation. Parent adopts exact caps and checks operational suitability before RED.

Retain normalized B2 storage; schema3 adds lineage/raw imported origin/board-source provenance/epochs. Schema2 or unknown schema refuses before durable mutation, never migrate/drop/reset. Source/store/prepared-import/snapshot/sink inward ports and ImportBoardHistory/ExportBoardHistory use cases; SQL never spawns Git. Reuse sole Reader, ObjectWriter/fault seams and hardened bounded Git transport. One owned prepared IMMEDIATE transaction; application performs shared Export dry-run on its uncommitted structural view before consuming commit. Drop/error/crash rolls all imported rows/head/provenance back; distinguish initialized empty schema existence. Equal IDs require complete kind/content collision verification.

Persist native parent/sequence/message/writer/UTCtime atomically with write/clear/bootstrap. SQL heads intentionally epoch:revision, not canonical Git IDs. Original imported history through H0 exports exactly; later framed native trees/blobs translate recursively to real Git objects, followed by deterministic valid commits parented to H0 and the previous real Git commit. Export owns a read snapshot/cursor through captured frontier,4096 OID-pairs-only eviction-safe cache,one previous Git commit; no total later-native revision cap. Bound recursion/runner copies without claiming128MiB RSS. Verify all unreferenced destination objects before captured expected-head lease/CAS; report ambiguity and possible unreachable objects, never advertise partial history.

Before RED parent explicitly disposes the known-scope epoch table: current epoch inclusive current frontier Written if other guards pass; locally held current stale/old epochs Lost; plausible unseen retired-epoch token beyond local tip Lost as fence only, not asserted held; unknown/future/noncanonical/malformed/impossible pairs InvalidWrite. No broad old-epoch shortcut. Minimal storage epoch primitive only, no B12 promotion services. Required schema/head expectation-version changes need explicit approval preserving every existing case/assertion meaning/raw/crash proof.

Operator owns B4 listener/limits/status/time/snapshot/peer credentials/shared root, B1/B6/T1 and Git fence/parser/builder. Add offline handlers/registration/composition to the actual operator root only; never duplicate/replace root or remove operator verbs. B7 separately waits B4 merge. Scratch106+ linear corpus plus owner12-commit set-aside/clear replaces historical live manual gate. No live import/export/fence/cutover/deployment/server/unit start/relay/state/admission/credentials/protected-playbook changes.

Use separate fresh RED then delivery then independent exact-head PR review phases; no planning-worker rollover or workflow in swarms. Future fixed pool: coordinator plus one author and one source verifier, member_limit25, one checkout editor/lane builder and at most two building members across this lane. Parent governs actual built-in review cycles, own-diff scoped mutation, normal foreground push>=1500s, version/CI/exact-head merge. Delivery role does not absorb independent review; final execution-specific accepted plan/team is set after predecessor reconciliation.

## Task items
| Task item | Title |
|---|---|
| scope | Reconcile merged predecessor and adopt precise B3 decisions |
| red | Fresh red-first history and rollback acceptance handoff |
| ports-schema-source | Layered history contracts, schema3 and finite source capture |
| atomic-import | Prepared atomic import and application self-export proof |
| lineage-epochs | Native lineage and precise minimum epoch fencing |
| rollback-cli | Complete pinned rollback export and additive offline CLI |
| delivery-handoff | Verified complete PR and revision-bound independent acceptance |

## Task item dependencies
| Task item | Depends on |
|---|---|
| red | scope |
| ports-schema-source | red |
| atomic-import | ports-schema-source |
| lineage-epochs | atomic-import |
| rollback-cli | lineage-epochs |
| delivery-handoff | rollback-cli |

## Criteria
| Task item | Criterion |
|---|---|
| scope | Read actual merged B2/R2 and actual operator command integration seams; preserve external ownership. |
| scope | Parent adopts exact bounds, complete forecast and known-scope epoch table. |
| scope | Explicitly approve necessary schema3/epoch-head expectation versions without weakening any assertion meaning. |
| red | Preserve original store/every-kind,43/G4789/R1/R2 cases and original raw/crash/fetch proofs with a complete manifest. |
| red | Compiling intended baseline failures cover owner12-commit set-aside/clear,106+scratch SHA1/SHA256 history, atomic refusal, native continuation, epoch boundaries and CLI. |
| red | Independently verify red evidence and durable verified bundle; do not push failing tests. |
| ports-schema-source | Inward source/store/prepared/snapshot/sink ports and two application use cases, composition-only construction and USE_CASES/contracts. |
| ports-schema-source | Full linear actual-kind closure and exact raw original IDs, metadata and provenance with checked bounds/hash verification. |
| ports-schema-source | Exact DDL recognition and old-schema refusal; full collision equality; reuse sole interpreter and bounded transport. |
| atomic-import | One owned IMMEDIATE transaction over initialized empty target; consuming commit and Drop rollback. |
| atomic-import | Shared Export dry-run verifies every original ID/count/H0 from uncommitted view before publication. |
| atomic-import | Failures/crash leave all rows/head/provenance unchanged; distinguish empty initialized schema; no staging protocol. |
| lineage-epochs | Persist parent/sequence/message/writer/UTCtime with existing bootstrap/write/clear; unchanged writes remain distinct children. |
| lineage-epochs | Implement parent-disposed inclusive frontier/stale/divergent/impossible epoch classification, including held-at-promotion regression. |
| lineage-epochs | Preserve existing admission, bounds, typed refusal and owned-child crash semantics; fingerprints cover all new columns/tables/unreachable rows. |
| rollback-cli | Replay original raw history exactly and translate every later native revision into deterministic valid Git commits. |
| rollback-cli | Pinned cursor through captured frontier; cache eviction preserves results and no later-native total truncation. |
| rollback-cli | Verify before expected-head publication; dry-run never writes refs; source unchanged and ambiguity explicit. |
| rollback-cli | Inject use-case handles through composition and register offline import/export additively in actual operator root; preserve all operator verbs. |
| delivery-handoff | Targeted gates, preservation manifest, actual production/exclusion sizing and complete scratch equivalence evidence at exact head. |
| delivery-handoff | Normal trusted foreground pre-push, PR publication and durable handoff; no full local suites/mutation by delivery workers. |
| delivery-handoff | Parent-managed separate actual built-in cycles, scoped mutation, fresh independent current-head review, resolved threads and green required CI before merge. |
| delivery-handoff | No live cutover/deployment or protected ownership changes. |
