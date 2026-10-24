<!-- quecto-task schema=1 -->
# Make sessions architecture scans honor ordinary cfg(test) module ownership

| Field | Value |
|---|---|
| Id | sessions-ordinary-test-ownership |
| Kind | task |
| Status | in_progress |
| Parent | land-layout-2357 |
| Accepted plan | 2 |
| Claimed by | shane-pocket (`shane@platformq.co.uk`) |
| Claimed since | 2026-10-24T08:26:13Z |
| Claim expires | 2026-10-24T10:26:13Z |
| Created | 2026-10-23T21:37:16Z |
| Updated | 2026-10-24T08:26:13Z |

## Description
Human authorized standalone small architecture-scanner ownership fix PR to master, parent merge after independent exact-head review and green required CI, then finish moves-only #2650 normally. Test-child modules, *_tests.rs and test_support must not count as production constructor or retention owners; production classification remains exactly as strict. Fresh RED proves test files constructing/retaining are not flagged and genuine production files still are; include shared production/test-mounted controls and reuse established AST ownership where possible. No fake production-owner allowlists, assertion weakening, test-blob changes, runtime changes, deferred #2702 hardening, or hook bypass. Target500/hard900 changed lines incl tests; fresh planning, tests-only verified RED bundle, separate delivery final own master+1 version-only commit, foreground push >=1500, merge-requested full CI, fresh independent PR review and resolved threads, parent authorized merge. [Primary #2650 blocker/source](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6375156118), [verified recovery](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6375188214). Candidate V441bb287ca16bf70737f712fd4c9a2ddfc02dcb0 has349 changed lines, all452 inherited test lines conserved; legacy sessions scanner falsely classifies ordinary active_session/tests.rs constructor and conversation_ledger/tests.rs retention owner. Existing RED/layout assertions remain unchanged. After fix merge, refresh #2650 via actual master merge/own version bump/targeted gates/fresh independent review/green full CI/parent merge without hooks bypass.
