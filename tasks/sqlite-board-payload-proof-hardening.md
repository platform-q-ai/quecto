<!-- quecto-task schema=1 -->
# Strengthen metadata-only SQLite board tests with independent payload-read evidence

| Field | Value |
|---|---|
| Id | sqlite-board-payload-proof-hardening |
| Kind | chore |
| Status | draft |
| Created | 2026-10-24T06:23:53Z |
| Updated | 2026-10-24T06:23:53Z |

## Description
Optional test-strength follow-up, not a current production bug or an MVP blocker. Independent review of [PR #2706](https://github.com/platform-q-ai/quecto/pull/2706) at dd01f3dd165f2581d858b23284bc4961ef0291f3 established that the current admitted materializer is the sole production BLOB query and that its observer is connected. Its voluntary fetch trace does not independently detect a future eager SELECT elsewhere. Preserve existing reader/store contracts and observations; investigate an independently enforced access boundary or bounded owned-process resource check with a positive eager-read control, correctly classified memory-limit outcomes, hard SIGKILL/_exit timeouts and complete scratch cleanup. Source-token assertions or another voluntarily called helper do not establish this guarantee. Do not claim current eager loading, historical red invalidity, or universal no-materialization proof. Primary limitation/refutation: /home/shane-pocket/.quecto/artifacts/b2-sqlite-independent-rereview-r1/test-spec-strength.md. No implementation is approved by this draft; size and plan separately when selected.
