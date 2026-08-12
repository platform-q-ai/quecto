# #1196 test/check design

Traceability:
- AC1/AC2: Chat direct mutation cap tests; SessionView projection cap; LedgerTranscript/feed retained copy cap/audit.
- AC3: exact master older history content after eviction via prepend/paged-history path; exact child older content after eviction via existing sync path; safe stale stub/page responses.
- AC4: live append/stream/finalize/tool completion after eviction pressure.
- AC5: multi-session overflow isolation.
- AC6: focused new tests plus chat/paged_history/ledger_sync/live_inflight/protocol regressions.

Planned RED tests/checks:
1. Boundary: `chat_1196_retention_does_not_evict_at_or_below_cap`; add cap entries and assert all remain, then cap+1 evicts exactly older content while keeping newest.
2. `chat_1196_retention_cap_applies_to_tail_appends`: add > cap user entries; assert retained count <= defined cap and newest exact content retained.
3. `chat_1196_retention_cap_applies_to_replace_range_batches`: replace with > cap; assert bounded newest exact window.
4. `chat_1196_recovered_prepend_is_retained_within_cap`: overflow tail, prepend older page containing exact `older-0`; assert count <= cap and `older-0` is present after prepend.
5. `chat_1196_streaming_survives_eviction_pressure`: fill to cap, stream assistant, add pressure/finalize; assert exact streamed text appears once, finalized flag/render cursor correct, order coherent.
6. `chat_1196_tool_completion_after_eviction_pressure_updates_by_id_or_safe_noop`: open tool near tail under pressure; complete; assert matching id gets result if retained, or no wrong tool/result if evicted; count bounded.
7. `session_1196_project_ledger_with_live_is_bounded_and_adjusts_indices`: project > cap ledger plus live; assert bounded, active_turn_start safe, newest/live exact content retained.
8. `ledger_1196_transcript_retention_is_bounded_but_can_recover_from_sync`: apply sync delta > cap, then existing sync/resync window with exact old ids/content; assert retained counts bounded and old content appears in projection.
9. `app_1196_child_sync_caps_feed_and_session_chat`: route large sync response to child; assert FeedState retained count and SessionView.chat count bounded with exact newest content.
10. `app_1196_multi_session_overflow_is_isolated`: overflow child A sync/chat; child B exact content, active focus, roster/feed indicators unchanged.
11. `app_1196_stale_stub_or_page_response_after_eviction_is_safe`: issue/record pending recall/page for a session, evict target, route response; assert no panic, no wrong-session mutation, bounded count.
12. Compatibility/regression checks: run focused `cargo test -p quecto-tui chat`, `paged_history`, `ledger_sync`, `live_inflight`, exact `1196` filters, and protocol/serialization filters (`protocol`, `serialization` where available). Inspect diff to confirm no server storage or wire JSON/protocol shape changes.

Use package-visible helper methods for retained counts where necessary to prove measurable memory bounds; avoid asserting private field names or exact implementation strategy beyond the defined cap/window policy.

Review findings resolved: added at/below cap boundary, exact AC3 content assertions, stale response test, protocol regression evidence, and reduced implementation-detail coupling.
