# Issue #1196 scope lock

Phase: all approved phases for feature workflow.

ACs: AC1-AC6 from issue #1196.

Non-goals: no server retention/compaction/storage changes; no protocol/wire JSON changes; no replacement of ledger sync/get_messages/tail/message/paging; no chat UX redesign beyond bounding; no global memory budget; no unrelated roster limits; no second full transcript/rendered copy.

Expected surfaces: quecto-tui Chat component/tests; SessionView/ledger sync/history recovery tests; multi-session agent/harness tests as needed. HistoryPaging remains pure policy. Controllers continue owning runtime I/O/retrieval.

Verification planned: focused cargo tests for chat, paged_history, ledger_sync, live_inflight and exact #1196 tests; hooks; PR review/CI.

Deferred work: none beyond approved non-goals; stop if child older-prefix recovery cannot be proven through existing sync/inspection paths without protocol changes.
