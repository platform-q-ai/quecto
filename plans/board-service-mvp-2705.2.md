<!-- quecto-plan schema=1 -->
# Plan 2 of board-service-mvp-2705

| Field | Value |
|---|---|
| Task | board-service-mvp-2705 |
| Version | 2 |
| State | superseded |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted by | shane-pocket (`shane@platformq.co.uk`) |
| Accepted at | 2026-10-24T00:11:14Z |
| Created | 2026-10-24T00:10:59Z |
| Updated | 2026-10-24T03:02:07Z |

## Approach
Operator coordination supersedes earlier owner-header order/sizing for this lane: agent delivers B2 then B3 then B4 ALONE then B7 then B5-lite then B8 then B13-lite then Pocket P1+P2. Operator independently owns B1(GitRemoteRevisions plus --board-id/--remote-url) and B6(BoardOperations), their branches/PRs are not agent work. Never start B1/B6. Before starting B7 establish primary evidence B6 merged; if not, report operator blocker rather than implementing it. Human-approved PR2713 sizing applies now even while documentation PR is open: target750/max1500 PRODUCTION changed lines, tests/fixtures/snapshots/lockfiles/generated excluded and separately reported; judge once at planning, rejudge only remediation checkpoint or exceedingmaximum. This is explicit human-approved exception to previous inclusive900cap, justified by avoidable split/review/CI overhead. Do not split only for target; preserve complete independently green behavior and reviewability. Fold unpushed characterization47890182409ebb23a88208f388414c4ea1d687ac into existing B2 PR2706 by merge, preserving SQL red43ed52f76340ab44bfe6e9288252e411b114609d and all new/existing assertions; no unnecessary standalone G PR. Source/refutation production462–646 conservative incltest-support fits750target, so six actual SQL fixes/all proof in ONE remaining B2 PR. Separate fresh regression-red, fix/delivery, two actual builtin adversarial workflow cycles outside swarms, fresh independent exact-head review and requiredCI before serial merge. Keep patch bump in ownLASTcommit, merge-requested label triggers/restartsCI. At most2building members, foregroundpush>=1500seconds. No agent playbook edits/deployment/cutover/live import/fence/server/unitinstall/start/relay/state/units/admission/credential/env exposure; operator performs cutover after all MVP merges. Preserve remaining original owner requirements and properlayers/affirmativeguards/test-first.

## Task items
| Task item | Title |
|---|---|
| b2 | Complete SQLite store with all preserved contracts and six fixes |
| b3 | Full-history import and rollback export |
| b4 | UDS board server only |
| b7 | Transactional server operations and remote client after operator B6 |
| b5-lite | Tailscale listener and per-device tokens |
| b8 | Run lifecycle across wire |
| b13-lite | Fence, moved answer and online backup |
| p1-p2 | Pocket relay and deployment artifacts without deployment |

## Task item dependencies
| Task item | Depends on |
|---|---|
| b3 | b2 |
| b4 | b3 |
| b7 | b4 |
| b5-lite | b7 |
| b8 | b5-lite |
| b13-lite | b8 |
| p1-p2 | b13-lite |

## Criteria
| Task item | Criterion |
|---|---|
| b2 | All15store+9kind checks unchanged, original55/eight+newcharacterization samebodies overrealGit+SQLite, exact54 preserved, newrealcompilingregressions for actualmissingbehavior. |
| b2 | WAL/FULL, metadata-first boundedloads, sharedsoleinterpreter, atomicCAS/rollback/bootstrap/clear, writer/rawbytes/quarantine/crash, no productionfixturestate/extratransactions. |
| b2 | Preserve immutableSQLred43 and characterizing4789 by merge, no rewrite; allsixcurrentPR2706threads fixed with verifiedfixSHA/testreplies and resolved before merge. |
| b2 | Sizing planningproduction462–646conservative,target750/max1500production;excludedtests/fixtures/lock/generated separatelyreported;no unnecessarysize-onlyprerequisite. |
| b3 | Scratch fullhistorybyte-exactimport/export preservescommitids,rawobjects/provenance; no live import or rollback execution. |
| b4 | Guarded boundedUDS transport/server lifecycle/composition inproperlayers; no host install/start or deployment. |
| b4 | Do not implement B6 BoardOperations port refactor; operator owns it. Use agreed interfaces/integration seams without duplicateownership. |
| b7 | Beforestarting establish operator B6MERGED primaryGitHubevidence; ifnot,notifyoperatorandwait, neverimplementB6. |
| b7 | Per-board transaction/write_id resultreplay/remoteclientcontract; integrate operator B1 --board-id/--remote-url and B6 interfaces ratherthan duplicatework. |
| b5-lite | Tailscale-IP-onlybinding/per-device tokenauth; membersdeferred, no deployment. |
| b8 | Clientlaunchfacts andserverstart/end_runparity includingrollback/failure, no host liveoperation. |
| b13-lite | Format4fence/movedanswer tests scratchonly, consistentonlinebackup; livefenceuntouched. |
| p1-p2 | Pocketrelay UDS explicitcommitter plus deploy/token templates/minutebackupsync; never live relay/state/units/admission or deploy/start. |
