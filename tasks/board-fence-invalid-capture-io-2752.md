<!-- quecto-task schema=1 -->
# Fence Low follow-up: prove invalid-capture refusal precedes remote I/O (#2752)

| Field | Value |
|---|---|
| Id | board-fence-invalid-capture-io-2752 |
| Kind | task |
| Status | draft |
| Created | 2026-10-25T19:37:02Z |
| Updated | 2026-10-25T19:37:02Z |

## Description
Explicitly accepted deferred Low test-strength follow-up from independent PR2751 review at422ee6bd262d2d0011eac9ea35d7f9bcae9d216e, current authoritative CI all8success. [Issue #2752](https://github.com/platform-q-ai/quecto/issues/2752) and [finding](https://github.com/platform-q-ai/quecto/pull/2751#discussion_r4372115203) contain validated scenario/acceptance. Current capture object_id allowlist is correct. The nonrepo fixture test would still returnUnavailable if the allowlist were removed and remote I/O attempted; guard-deletion was source-proven, NOT mutation-executed. Add permanent zero-Git-call trap/recorder for invalid capture preserving all old assertions and CAS/identity/no-op/refusal behavior. Document actual falsifier execution honestly. No production bug or coverage fix claimed in MVP. Future separately scoped planning/tests/gates/current-head independent review required; nonMVPdependency, no immediateimplementation. Only owned scratch Git/HOME, hard timeouts/cleanup; no liveops/deploy/relayadmission/credentials/protected/globalconfig. Related epic2705/taskboard-2705-b13-fence; operator owns ALLPocket and CI2748; exclude those lanes.
