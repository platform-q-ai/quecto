<!-- quecto-task schema=1 -->
# Pin sessions exact-file and guard-role exception boundaries (#2730)

| Field | Value |
|---|---|
| Id | sessions-file-role-regression-matrix |
| Kind | task |
| Status | draft |
| Created | 2026-10-24T20:59:43Z |
| Updated | 2026-10-24T20:59:43Z |

## Description
Deferred LOW test-strength [follow-up2730](https://github.com/platform-q-ai/quecto/issues/2730) from independent PR2650 review5639987046 at head63613c197608d6f51fbabab215182955625c315f; inline4368487402 confirmed by a different finder. Current implementation correct/private34 child checks reject wrong-child/same-basename/extra-role cases, but permanent sibling fixtures do not pin whole boundary. Future test-only compact matrix using existing Fixture and real guard APIs: approved-child positives; wrong child/same basename/typo prefix suffix/opposite child/extra-role negatives; exact2files3roles active_session/tests constructor+RawKey and conversation_ledger/tests retention under services. Ordinary production siblings/unrelated paths rejected, legitimate production sets/assertions/bounds/hard30 preserved. No scanner/runtime implementation or weakened test/moved blob edits/ignore. Characterization honestly passes current correct baseline; no invented RED. Future independently planned/tested/reviewed delivery, not authorized by recording this draft. Parent explicitly defers LOW after current63613 fullCI green and resolves PR thread as accepted deferral, NOT fixed. Separate from stopped general model#2729/deliberate bypass#2702; does not authorize resuming those.
