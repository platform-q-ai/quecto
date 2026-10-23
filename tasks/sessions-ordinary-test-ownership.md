<!-- quecto-task schema=1 -->
# Make sessions architecture scans honor ordinary cfg(test) module ownership

| Field | Value |
|---|---|
| Id | sessions-ordinary-test-ownership |
| Kind | task |
| Status | draft |
| Parent | land-layout-2357 |
| Created | 2026-10-23T21:37:16Z |
| Updated | 2026-10-23T21:37:16Z |

## Description
Newly discovered prerequisite requiring authorization beyond moves-only PR #2650. After normal master merge and four ordinary test-child relocations, sessions_capability legacy production-source classifier treats active_session/tests.rs as constructor owner and conversation_ledger/tests.rs as retention-port holder. Exact unpushed V441bb287ca16bf70737f712fd4c9a2ddfc02dcb0 gate21 selected/19passed/2failed; fresh AST ordinary-child regression passes unchanged. [Primary preservation and recovery](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6375156118), [validated blocked receipt](https://github.com/platform-q-ai/quecto/pull/2650#issuecomment-6375188214). Plan a bounded checker-ownership correction with fresh failing tests proving ordinary cfg(test) children excluded while real production owners remain enforced, including shared production/test-mounted cases. Preserve existing ownership expectations, assertions and all452 inherited test lines, no fake owner allowlist additions/path redirects/shims/hook bypass. Prefer existing AST ownership seam over new scanner semantics or textual exclusions. Establish independent delivery/PR scope and separate fresh RED, implementation, independent review/full required CI before resuming #2650 with current master and normal hook. Do not implement until human authorizes scope.
