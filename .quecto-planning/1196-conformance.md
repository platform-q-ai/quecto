# #1196 conformance verification

Head: f8202551
PR: https://github.com/platform-q-ai/quecto/pull/1459

| Item | Evidence | Verdict |
|---|---|---|
| AC1 per-session bound | `CHAT_RETAINED_ENTRY_CAP` and retention counters in `chat.rs:99-102`; tail add/stream call retention in `chat.rs:159-186`; ledger cap in `ledger.rs:5,45,54-63`; child projection via capped chat in `view.rs:256-302`. Tests: `cargo test -p quecto-tui 1196`, `chat_1196`, `ledger_sync`. | PASS |
| AC2 eviction/demotion | Tail trim in `chat_retention.rs:3-5`; prefix bounded window in `chat_retention.rs:7-37`; cache invalidation in `chat.rs:330-364`. Tests cover tail, replace, prefix, large prefix/full suffix. | PASS |
| AC3 recovery | Prefix recovery paths retain recovered entries within bound in `chat.rs:287-319` and `chat_retention.rs:13-30`; master paged/backfill reconcile in `app_response.rs:377-398`; child sync path through `ledger.rs:31-46` and `controller_ledger_sync.rs:47-53`. Tests: `chat_1196_*recovered*`, `ledger_1196_*recover*`, `paged_history`. | PASS |
| AC4 live tail coherence | Streaming append/finalize unaffected in `chat.rs:170-196`; recovered-prefix logic preserves live suffix in `chat_retention.rs:13-30`; active turn starts reconciled in `app_events.rs:153-160`, `view.rs:306-313`; ledger projection sets committed count before live attach in `view.rs:274-300`. Tests: `chat_1196_streaming*`, `live_inflight`, `paged_history`. | PASS |
| AC5 multi-session isolation | Sync routing scoped by `agent_id` in `controller_ledger_sync.rs:47-53`; per-session projection/reconcile in same route and `view.rs:256-313`. Test: `app_1196_multi_session_overflow_is_isolated`. | PASS |
| AC6 tests | Added exact #1196 tests under chat retention, ledger sync, app ledger sync; focused suites run locally and pre-push. | PASS |
| Cache/index/cursor high-risk row | `render_cache` resized/cleared and offsets invalidated in `chat.rs:330-364`; scroll offset restored for prefix retention in `chat_retention.rs:32-36`; active-turn deltas in `chat.rs:208-212,352-358` and reconciled in controllers. | PASS |
| Non-goals | Diff touches TUI chat/agent/controllers/tests/docs/version/planning only; no server storage, protocol shape, or wire JSON changes. Protocol tests run. | PASS |
| Architecture/docs/version | Chat rendering split to keep line-count/architecture gates; feature ownership doc updated; `quecto-tui` bumped to 0.76.6 and README/Cargo.lock synced. Pre-push architecture/docs gates passed. | PASS |

CONFORMANCE: PASS
