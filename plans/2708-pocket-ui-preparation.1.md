<!-- quecto-plan schema=1 -->
# Plan 1 of 2708-pocket-ui-preparation

| Field | Value |
|---|---|
| Task | 2708-pocket-ui-preparation |
| Version | 1 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-25T23:10:18Z |
| Updated | 2026-10-25T23:10:18Z |

## Approach
Deliver the Android-only vertical slice described in plan-pocket141-split/pr-ui-plan.md, refutation.md, findings-test-matrix.md, future-gates.md and wording-per-pr.md. Start from pinned current master, not by restoring whole WIP files. Preserve all six new master parse-bearing sites and navigation/board lifecycle behavior. The domain-purity assertion already passes on master and is characterization, not fresh RED; establish meaningful RED through the existing bubbleBody Question fallback API and other existing behavior boundaries, with compile errors/missing APIs never counted. Preserve the 14 UI test-file ownership inventory and original display/rich/raw/cache/cancellation/notification/privacy assertions. Use separate fresh RED and delivery phases with a task-appropriate fixed pool, ceiling25, one serial builder and global maximum two building members; verify unpushed prerequisite-bound RED bundle before delivery. Keep semantic source single-owned, classify all production/debug/mechanical changes (target750/max1500), tests, docs, and generated/lock files separately. Run exact-head local unfiltered Android DEBUG tests, applicable architecture/contracts and source/tool mode/hash seals before verified foreground push. Publish all wording verbatim and the approved limited L9 observation guarantee; obtain operator independent review at actual head, then merge normally before updating relay PR141. Do not infer that the1603-line relay remainder is cap-compliant or that FINAL.1 is fixed.

## Task items
| Task item | Title |
|---|---|
| ui-red | Establish existing-API RED and preservation controls |
| ui-domain | Safe Question projection and notifications without raw mutation |
| ui-preparation | Instance-owned bounded cancellable preparation |
| ui-master-integration | Preserve all current-master render and lifecycle paths |
| ui-ownership | Verify the approved runtime-plus-compiled boundary |
| ui-deliver | Local final-head gates, wording and independent review |

## Task item dependencies
| Task item | Depends on |
|---|---|
| ui-domain | ui-red |
| ui-preparation | ui-red |
| ui-master-integration | ui-domain |
| ui-master-integration | ui-preparation |
| ui-ownership | ui-master-integration |
| ui-deliver | ui-ownership |

## Criteria
| Task item | Criterion |
|---|---|
| ui-red | A fresh baseline test compiles and fails for actual control-only Question fallback or another intended missing UI behavior, never missing new APIs. |
| ui-red | Domain purity remains a baseline-green characterization; runtime/compiled ownership checks distinguish meaningful RED from passing controls. |
| ui-red | Preserve original579 display assertions and the 14-file UI inventory; red commit, prerequisite/master SHA, commands, failure evidence, verified bundle SHA are durable and unpushed. |
| ui-domain | Control-only/bidi/invisible fallback is useful; legitimate RTL, combining and emoji remain meaningful. |
| ui-domain | Copy/share/storage retain exact raw content; rich/multiline/link/code/option Questions remain unchanged. |
| ui-domain | Alert privacy, permission and foreground suppression are preserved; domain has no ambient mutable parser callback. |
| ui-preparation | Immutable per-instance parser/cache ownership and cancellation propagation; canceled derive publishes no stale state. |
| ui-preparation | Bounded AST/source charges and current-raw readiness update atomically, including same text with a new raw reference. |
| ui-preparation | No unobserved parse in renderer fallback; all original numerical/input/cache/raw/rich/cancellation assertions stay meaningful. |
| ui-master-integration | All six TaskPlan/BoardTaskScreen master parse-bearing sites, including method references and generated lambdas, route through the same UI preparation owner. |
| ui-master-integration | Master PocketViewModel taskPage, Written/Sectioned/Evidence, board lifecycle and navigation behavior remain intact. |
| ui-master-integration | Changes are adapted semantic hunks; no wholesale WIP replacement or relay API dependence. |
| ui-ownership | Actual measured UI paths use the injected immutable owner and retain runtime count/isolation tests. |
| ui-ownership | Actual compiled production UI, nested and generated classes are scanned with the exact owner/nonparse allowlist and positive/negative bypass controls. |
| ui-ownership | Disclose that the guarantee is not arbitrary reflection or global-hook interception equivalence; no source-text-only substitute. |
| ui-deliver | Production/debug/mechanical diff is measured within1500, target750; tests/docs separately disclosed without test cuts or code-density gaming. |
| ui-deliver | Full unfiltered :app:testDebugUnitTest plus applicable quality/contracts gates pass at exact final SHA with XML/raw counts, modes/tool/source seals and owned cleanup. |
| ui-deliver | All new wording is verbatim in PR and intentional UI docs; foreground push timeout>=1500, operator independently reviews actual head, no CI/release/APK/deploy. |
| ui-deliver | Normal merge preserves history; subsequent relay integration merges updated master without rewriting original PR141/RED ancestry or duplicate ownership. |
