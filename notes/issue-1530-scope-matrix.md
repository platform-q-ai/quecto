# Issue #1530 scope lock and semantic matrix

Phase: implement all planned phases 1-4 for behavior-preserving extraction of default unread agent_cmd get_messages planning/delivery.

ACs covered: all issue ACs. Non-goals: no user-facing semantic changes, no explicit count/before changes, no new persistence store, no full UDS DTO rewrite.

Expected surfaces: agent_cmd.rs orchestration, agent_cmd_report.rs focused component, session/subagent_registry persistence, focused agent_cmd report/recovery/persistence tests.

Semantic risk matrix:

| AC/invariant | Dimensions/classes | Representative cases | Expected observable/correlation | Evidence |
|---|---|---|---|---|
| default planning parity | delivered=0 vs >0; assistant substantive vs tool/empty; unread empty/present; missing ordinals | first contact returns latest substantive assistant only; delta returns all unread; stale observed<delivered unchanged | JSON data messages/unchanged same as baseline; cursor pending only for deliverable report | focused planner tests + existing report tests |
| bounding/recovery | fits/over budget; final assistant priority; recoverable vs unrecoverable truncation; stripped payload fields | huge content with id gets contentRecovery; no id unrecoverable yields incomplete/no ack | bounded response flags hasMoreMessages/messageContentTruncated/reportIncomplete | component tests, recovery tests |
| backfill | first page contiguous/gap; before present/missing; older empty/malformed/timeout; page cap | need backfill until contiguous or assistant first contact; stop incomplete on failures/cap | parent fetches pages only; planner decides continue/incomplete | component backfill policy tests; AgentCmd loop inspection |
| ack transition | success/error; incomplete; explicit count/before; post append timing; receipt present/missing/malformed; two byte-identical reports with different ordinals/tokens; normalized JSON order/content changed | only a matching stable token/receipt advances the corresponding new pending report; error/incomplete/count/before do not; equal response with different token cannot ack wrong pending; malformed/missing unknown new receipt is ignored | delivered ordinal changes only after Tool::result_delivered; user correlation is opaque receipt embedded in result envelope | transition tests for success, error, incomplete, explicit paging neutrality, identical response collision, malformed/unknown receipt |
| clear_history | success true vs false/malformed/error; pending queue nonempty; delivered set/unset | success resets delivered and all pending state; others no advance/reset | subsequent default report acts first-contact only after success | transition tests |
| persistence compatibility | old {response,ordinal}; new receipt shape; mixed old/new queue after reload; two old byte-identical pending responses; roundtrip | deserialize old safely preserving ordinal/cursor data; legacy old entries may use compatibility response matching only for old records or be safely non-ackable without data loss, but must not fail load or corrupt newer receipt matching | roster state loads; new entries serialize with receipt; delivered/pending state retained in roster store | serde compatibility and mixed-queue tests |
| routing/recovery | routed descendant target; get_message contentRecovery; explicit pages | routed backfill and recovery keep agent_id/target; count/before neutral | same command/JSON compatibility | existing recovery/routing tests |
| irrelevant dimensions | permissions, model/effort, container lifecycle not materially changed except registry liveness lookup | inspect only | no test beyond unchanged suites | inspection |
