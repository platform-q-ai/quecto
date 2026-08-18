# #1196 semantic state-space matrix (frozen candidate)

## Dimensions
Applicable: session type (master/child); retained store (Chat.entries, SessionView.live_inflight, SubagentUi feed LedgerTranscript); mutation source (tail append, streaming/finalize, tool start/end, replace, prepend, ledger reprojection); position/window mode (live-tail window vs scrollback/recovered older window); cardinality (below/at/above bound, huge batch); recovery source (master paged get_messages/get_message, child existing ledger sync/inspection evidence); identity/correlation (tool ids, stub ids, pending page ids, session ids); indices/cursors (active_turn_start, scroll offsets, oldest-loaded/paging cursors); async races (response after local eviction); isolation (active/inactive sessions, focus, roster, feed, live indicators).

Irrelevant/non-goal: server retention/compaction/deletion, wire JSON/protocol shape/version churn, global process memory budget, chat visual redesign, unrelated roster retention.

## Window semantics to test/design against
- There is one bounded retained-content window per Chat-like transcript store; default live-tail mutations retain newest entries.
- Explicit recovered/prepended older history is allowed to become the retained visible window within the same cap; it must not create a second full copy. Subsequent tail-following live mutations may return the window to newest content, and older content remains recoverable again from server-backed paths.
- Eviction never removes the current streaming assistant/tool start before its finalize/end while it is the active live tail; if a late completion targets an already-evicted item, it must no-op safely or render as existing recovery behaviour, never corrupt another entry/session.
- Front trimming must invalidate caches and adjust/reset entry-index state (including active_turn_start) and keep scroll/paging cursors safe.
- Pending stub/page responses whose target was evicted must be safe no-ops or re-anchor by id only in the correct session.

## Matrix
| AC/invariant | Dimensions/classes | Representative cases | Expected observable outcome | Observation/correlation | Planned evidence |
|---|---|---|---|---|---|
| AC1 bounded per session | master/child; Chat, live_inflight, feed LedgerTranscript | append/project/sync > cap for master and child; unfocused child feed grows | all TUI-resident retained transcript stores are capped or proven not long-lived transcript content | entry_count plus ledger/feed retained counts | unit/session tests + audit artifact |
| AC2 eviction | all mutation paths; huge batches; recovered prefix | overflow add/start_tool/replace/prepend/project/sync | older local retained content evicted/demoted; no second full transcript copy | oldest missing, newest or recovered window retained, counts <= cap | component + ledger tests |
| AC3 recoverability | master paged get_messages/get_message; child ledger sync/inspection; async after eviction | evict first page then request/prepend older; child older-prefix recovery proof; response after target eviction | older content can become visible/usefully retained within cap via existing paths; if child path cannot prove this, implementation must stop as blocker | request ids, before cursors, message ids, agent_id route | paged_history + ledger_sync/inspection tests or documented blocker |
| AC4 live tail coherence | live-tail mode; streaming/tool identity; finalization after eviction pressure | overflow while streaming/tool active, then token/finalize/end | active live entries remain ordered/coherent; late evicted completions safe; cache valid | entries/render order, flags/result by id | chat/session tests |
| AC5 isolation | multi-session; focus/roster/feed/live | overflow A feed/chat while B active/live | A bounded only; B transcript/focus/roster/feed indicators unchanged | session ids and active focus assertions | harness/agent test |
| Cache/index/cursor invariant | render_cache/combined_offsets/scroll/active_turn_start/history pending | trim after render/scroll; project ledger after trim; pending page/stub returns | no stale index corruption/panic; offsets rebuilt; scroll clamped; active_turn_start adjusted/reset | render/no panic/state asserts | component/session tests |
| No protocol/server churn | serialization/contracts | implementation diff | no wire JSON/protocol/server storage changes | git diff + existing serialization tests | inspection/regressions |
