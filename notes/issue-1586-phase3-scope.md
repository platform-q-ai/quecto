# Issue #1586 phase 3 scope lock

Phase: 3 Explicit snapshot/persistence request fan-out only. The issue calls this a barrier, but the repository's current TUI transport has no synchronous ack path; phase 3 therefore provides ordered enqueue + error surfacing, while phase 4 owns exit-path wiring and any stronger wait/ack policy.

In scope:
- Add an explicit command/API for a tab to persist its current session and durable roster immediately.
- Add an explicit TUI fan-out entry point that targets every currently visible/open tab before later ordinary-exit teardown wiring.
- Stamp live/restorable rows captured for ordinary TUI exit with the phase-2 `OrdinaryTuiExitStopped` restore reason.
- Preserve existing replacement semantics: each session persist replaces that session's durable roster, so empty current rosters clear stale persisted rows for that session while unrelated sessions/workspaces are preserved.
- Surface pre-dispatch enqueue failures via the fan-out return value and harness persist failures via UDS error events; phase 4 decides whether ordinary-exit teardown aborts or proceeds.

Acceptance criteria covered partially/by this phase:
- Durable storage can capture current live/restorable roster rows before ordinary-exit teardown when the explicit fan-out/command is invoked and processed.
- Captured metadata comes from phase-2 durable roster schema/classifier; no schema expansion.

Non-goals/deferred:
- Do not centralize Ctrl-D, /exit, /quit (phase 4).
- Do not invoke the barrier from actual exit paths yet (phase 4).
- Do not terminate parent/subagent processes or decide failure-to-teardown policy (phase 4).
- Do not change resume/historical sendability behavior (phase 5).
- Do not update docs/help lifecycle language (phase 6).
- Do not alter /new or /tab-close semantics.

Expected surfaces:
- TUI protocol command and shell barrier method.
- Harness UDS command handling and session persistence path.
- Existing session durable roster replacement and phase-2 restore reason/liveness classifier.

Semantic risk matrix:

| AC/invariant | Dimensions/equivalence classes | Representative single/pairwise/high-risk cases | Expected observable outcome | Observation/correlation | Planned evidence |
|---|---|---|---|---|---|
| Explicit tab snapshot command exists | id present/absent; restore reason known/omitted/unknown; ephemeral vs durable session | `persist_session` with `tab1:persist-exit` + ordinary reason; same command without reason; ephemeral session | Durable sessions save current messages/workflow/roster; ordinary reason maps only when explicitly requested; unknown/omitted preserves legacy behavior; ephemeral no-ops successfully | UDS ok/error event id/kind; saved session contents | protocol deserialization test; dispatch/session persist inspection |
| Ordinary-exit stamping uses phase-2 classifier | live/detached/dead rows; status idle/running/exited; ordinary vs legacy reason | live idle row + ordinary reason; dead/exited tombstone + ordinary reason | live/restorable rows get `OrdinaryTuiExitStopped`; pre-dead/pre-killed rows remain non-restorable/explicitly-killed per phase 2 | persisted roster entry fields; restore does not re-show killed row | existing `ordinary_exit_snapshot_marks_dead_tombstones_non_restorable` test |
| Current roster replaces stale roster for that session | old persisted roster non-empty/current empty; current non-empty; unrelated session present | prior agent A killed/removed, barrier on empty registry; same store contains session B | session A roster becomes empty/current; session B unchanged; no append duplicates on repeated persist | loaded session store by session key | existing empty-registry persist test; inspect `SessionStore::save` replacement semantics |
| TUI barrier targets every open tab without focus dependence | one/many tabs; active/inactive; duplicate display names; per-tab namespaces | two tabs both named worker; active tab not first; barrier invoked | each tab connection receives a `persist_session` command with its own `tabN:` correlation id; active selection unchanged | command channel order/ids; tab id namespace in command payload | targeted TUI API test or code inspection if harness construction is prohibitive |
| Ordering before later teardown is available at API level | FIFO command channel; barrier call before watch removal; send/enqueue failure | caller invokes barrier then phase-4 teardown; first tab enqueue fails but later tab is healthy | barrier attempts every tab, still flushes manifest, then returns first enqueue `Err`; successful commands are enqueued synchronously before subsequent teardown code can run; harness persist failure emits UDS error event; phase 4 owns abort/proceed policy | barrier return value, code order, and UDS error event | code inspection; cargo check; dispatch/session tests where practical |
| Manifest/workspace durability is flushed alongside sessions | registry path/manifest path parent exists/missing; workspace id unique; current tabs include pending attach | barrier on workspace with missing config dir; pending attach tab present in manifest | directory created; manifest snapshot saved with current tab registry/workspace id; session roster persists via per-tab command | files on default durability paths or inspected call graph | existing `persist_default_durability` behavior + barrier calls it |
| Compatibility and boundaries | older clients never send command; older persisted sessions lack roster/reason; disconnected/restored visible tab | legacy session resume then barrier; disconnected tab without sendable connection | legacy behavior unchanged; command is additive; disconnected/no-transport tabs cannot be actively re-persisted in phase 3 and are deferred to phase 4/5 handling, but manifest durability is still flushed | no deserialization regressions; no panics | cargo check/tests; explicit out-of-scope note |

Reviewer findings addressed:
- Empty snapshot clearing stale rows: in scope via existing session-save roster replacement; matrix row added.
- Failure semantics: barrier returns pre-dispatch enqueue errors and command emits harness persist errors; teardown abort/continue policy deferred to phase 4; matrix/non-goals clarified.
- Ordinary-exit restore reason: explicitly in scope; matrix row and existing test evidence.
- Merge/replace scope: per-session replacement preserving unrelated sessions clarified.
- Offline/restored tabs: no active per-tab command possible in phase 3; manifest flush included; resume UX/routing deferred to phases 4/5.

Tempting but out of scope for phase 3: input normalization/exit centralization (phase 4), actual process liveness cleanup (phase 4), teardown abort policy (phase 4), resume live-vs-historical UX/routing (phase 5), docs/help changes (phase 6).
