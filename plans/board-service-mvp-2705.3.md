<!-- quecto-plan schema=1 -->
# Plan 3 of board-service-mvp-2705

| Field | Value |
|---|---|
| Task | board-service-mvp-2705 |
| Version | 3 |
| State | draft |
| Proposed by | shane-pocket (`shane@platformq.co.uk`) |
| Created | 2026-10-24T02:59:27Z |
| Updated | 2026-10-24T02:59:27Z |

## Approach
Operator-updated lane is B2, B3, B4 ALONE, B7, B5-lite, B8, REDUCED B13-lite, Pocket P1+P2, serial complete independently green PRs. Operator owns B1(GitRemoteRevisions,--board-id/--remote-url), B6(BoardOperations), T1(NBSP normalization/command-scope identity over16entries,2703b), and git-store fence implementation(format4/MOVED typed board_moved reads+writes,pure idempotent fence-tree builder,allowlisted MOVED parser). Never start or duplicate those scopes. PrimaryGitHub confirms B6#2714MERGED02:30:59Zcommit5e2eca19285f040750d91ace51d6cfbb70b8b5cd; B7 externalgate satisfied, verify integrated interface whenreached. Operator reports B1#2715merging; latest primarywasOPEN, do notclaimmergeduntilverified; integrate latestmasterwithoutbackout. OurB13ONLYonlinebackup, quecto board-server fence command wiringtooperatorbuilder(push/idempotenceonSCRATCH),board_moved rehearsal. NEVERlivefence/cutover/import/install/start/deploy/relay/state/units/admission. Pocket has NO GitHubCI: mergegateis exact-headLOCAL Android DEBUG-variantunit tests plusrelaycargo tests, resultsrecordedonPR; pre-existingrelease-variantunitfailureonmaster is NOT gate. Fresh independentreview stillrequired, no inventedPocketGitHubCI/merge-requestedwait. Human-approved2713 target750/max1500PRODUCTION, tests/fixtures/snapshots/lock/generatedexcludedandreported, sizeonceplanning/rejudgeonlyremediationcheckpointormaxexceeded, no target-onlysplit. B2 remains existingPR2706 preserving originalred43+G4789+regressionR1e8414bymerge, allproof/sixfindings, currentlylocalimplementationtargetedgreenbutnormalhookpublicationfailed andfreshminimalgatingfixinprogress; nevercallpartial/timeoutpassed. Separate fresh red/fix/review, twoactualbuiltinworkflowcyclesoutside-swarm, exactheadindependentreview, appropriatehead-boundmergegates. HarnessPRownLASTversionpatch+normalforegroundpush>=1500seconds andmerge-requestedCI; Pocketusesitsdocumentedlocalgate. Atmost2builders. Protectedplaybooksourceeditsnotours. Preserve remaining ownerrequirements/CleanArchitecture/affirmativeguards/test-first; human/operatorcutoverafterMVPmerges.

## Task items
| Task item | Title |
|---|---|
| b2 | SQLite store contract and verified six-finding remediation |
| b3 | Full-history import and rollback export |
| b4 | UDS server only, operator ports consumed not rebuilt |
| b7 | Transactional server operations and remote client |
| b5-lite | Tailscale listener and device tokens |
| b8 | Run lifecycle across wire |
| b13-lite | Online backup, existing fence-builder wiring and moved rehearsal only |
| p1-p2 | Pocket relay and deployment artifacts with local merge gates only |

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
| b2 | Original15store+9kindbodies unchanged;exact54/original55/eight+newcharacterizationssamebodies overrealGit+SQLite; allactualSQLregressions/rawBLOB/crash/rollbackproofpreserved. |
| b2 | WAL/FULL,metadata-firstboundedreads,soleinterpreter,transactionalCAS/bootstrap/write/clear/provenance,no ordinaryfixturestate/extratransaction; unknownschemafailclosedbeforemutation. |
| b2 | Preserve43/G4789/R1e8414unchanged; actualnormalprepushandfreshindependentexactheadreview/requiredCI/threadsresolvedbeforemerge, no timeout/partialpassclaims. |
| b2 | Production750target1500max/excludedtotalsreported, no extra size-onlyprerequisite. |
| b3 | Scratchfullhistorybyte-exactimport/export preservescommitids/rawobjects/provenance; no liveimport/rollback. |
| b4 | GuardedboundedUDStransport/serverlifecycle/compositioninproperlayers, no livehostserver/unitstart/install. |
| b4 | B6operatorownedandmerged2714; integrate itsport, do notimplementB6/B1/T1/gitfence scopes. |
| b7 | B6externalmergegate primarysatisfiedby2714commit5e2eca1; verifyactualinterfaceintegratedbeforestarting,reportoperatorifmissing/revertedratherthanrebuild. |
| b7 | Per-boardtransaction/write_idresultreplay/remoteclientcontract; useoperatorB1--board-id/--remote-url/B6interfaces. |
| b5-lite | Tailscale-IP-onlybinding/per-deviceauthorization;membersdeferred,nodeployment. |
| b8 | Clientlaunchfacts/serverstart-end_runparityincludingrollback/failure, no liveoperation. |
| b13-lite | Consistentcompletedonlinebackup, neverliveDB/WALcopyasbackup. |
| b13-lite | quecto board-server fencecommandWIREStooperatorpureidempotenttreebuilder; push+idempotenceverifiedSCRATCHonly. |
| b13-lite | board_movedrehearsalconsumesoperatorformat4/MOVEDtypedread/writerefusal+allowlistedparser; no newgit-storefence/parser/builderimplementation, neverpushlivefence/cutover. |
| p1-p2 | RelayUDSexplicitcommitter/service+unit+tokentemplates/minutebackupsync,nolive relay/state/units/admission/deploy/start. |
| p1-p2 | NO GitHubCIexists: exactreviewedHEADlocalAndroidDEBUGvariantunittests+relaycargotestsresultsrecordedonPR required;releasevariantmasterpre-existingunitfailureNOTgate. |
| p1-p2 | Freshindependentexactheadreviewandnormalapplicablelocalchecks/threadsresolved, no inventedGitHubCIwait. |
