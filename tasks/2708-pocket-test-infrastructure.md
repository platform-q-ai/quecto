<!-- quecto-task schema=1 -->
# Repair pre-existing Pocket local unit-test infrastructure without production changes

| Field | Value |
|---|---|
| Id | 2708-pocket-test-infrastructure |
| Kind | chore |
| Status | draft |
| Parent | issue-2708 |
| Created | 2026-10-24T02:51:41Z |
| Updated | 2026-10-24T02:51:41Z |

## Description
Independent verification at PocketPR139 head8f55061501e47f574ae71b0e9aa44217961a896a/base8355c9382fc262de938d79cba854943d31715054 found283releaseCompose/Robolectric ComponentActivity resolution failures and2relaycontainmentfixture failures. Freshdifferent-member-refuted investigation identifiespre-existing source mechanisms: shared unit tests need test-hostmanifest currentlydebug-only; fixture redundantly combinessetsid withstart_new_session, orphaningchildbeforeancestryproof. CorrectONLYtest-host/configuration whilepreservingSHIPPEDrelease manifest, andfixture/sessionwiringwhilepreservingdeliberatedaemon Sweep coverage. No productioncontainment/successor-blockingpolicy changes or assertions weakened. Requiredacceptance: fullAndroiddebug+releaseunit green andfullrelaycargoatfinalhead, targetedbaselinecompiledfail evidence beforecorrection, exactcommands/resultsPR,freshindependentreview+separateverifier. NO GitHubCI/workflows/APK/deploy/live relay/state/units/admission/credentials/settings. Operator-definedlocalgate applies, hard900inclusive/no padding; sourceprimary /home/shane-pocket/.quecto/artifacts/issue-2708/{operator-pocket-local-gate.md,verify-pocket-p1-local/HANDOFF.md,investigate-pocket-local/HANDOFF.md}. BroaderwhetherunprovenSweepmustblocksuccessoradmissionnotpartofthistest-infrastructurechore; existingpolicy preserved. FoundoutsideP1typedoutcomepolicy deliveryscope; trackedasnewdraftwork.
