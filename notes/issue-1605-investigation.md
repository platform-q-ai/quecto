# Issue #1605 — initial investigation checkpoint (historical)

**Continued investigation subsequently reproduced a direct slim-state refresh
defect and implemented a tested correction. See `issue-1605-red-evidence.md`.
Statements below describe the earlier checkpoint, not the final branch.**

## Scope and workflow

Read the complete current issue body and comments with:
`gh issue view 1605 --repo platform-q-ai/quecto --json title,body,comments,state,url`.
The issue was open with no comments. Source baseline: `4501840f`.
Branch: `fix/1605-live-subagent-roster`.

Selected `bugfix`, associated issue 1605, installed pre-commit/pre-push and
activated the git wrapper. Step 1 complete. Step 2 (RED) remains incomplete:
**the reported larger-roster direct-feed freeze has not been reproduced**.
The workflow explicitly requires stopping rather than guessing at a fix in
this case. No production behavior changed; only a test module and this record
were added. No claim that #1605 is fixed or ready to close.

## Acceptance checklist and evidence

| Requirement | Result |
| --- | --- |
| Focused direct feeds update above four entries, mixed live/terminal and nested | Characterization matrix passes on existing production code; not a reproduction of the reported failure. |
| Inspection-only automatic documented/tested refresh cadence | Not implemented; remains outstanding. |
| Refused/lost final refresh recovers without refocus or another hint | Not implemented; remains outstanding. Matrix does not force queue refusal/loss. |
| Switching away/back loses/duplicates no messages | Complete focused transcript unchanged across immediate refocus in the matrix; exactly one committed answer. Delayed/stale refocus not exercised. |
| Preserve scroll and follow-tail | Matrix checks scrolled viewport across direct committed sync/turn-end and return to tail; existing scroll tests also pass in package run. |
| Regression reproduces larger-roster failure | **Not met**. Passing characterization is not RED evidence. |

## Executed direct-feed probe

`quecto-tui/src/agents/app_direct_roster_live_tests.rs` creates real Unix child
sockets, opens production direct feed tasks through roster registration, and
pumps their actual fan-in receiver through production routing/render decisions.
It checks connection kind, initial state/sync requests, unique per-child live
token markers before completion/refocus, revision advancement and outgoing sync
cursor, committed-answer count, scrolling and refocus transcript equality.

Roster sizes: **1, 4, 5, 8, 16, 30**. For each, unique live counts from
`{1, min(4, size), size}`, both flat and parent-linked roster identities:
**30 scenarios**. Non-live rows are terminal (`dead`) with no advertised socket.
Every live child produces the same 12-chunk workload with identity-specific
markers, following initial history sync and `turn_start`.

Limits: scripted peers, not real harness/model producers; headless frames, not
an interactive terminal; pumps production routing/paint helpers rather than
`App::run()`'s select loop; nested roster metadata with direct child sockets,
not nested monitor forwarding/container transport. No saturated queues, dropped
responses, missed start events, prolonged workload, terminal rows retaining
usable sockets, above-cap churn, or quantitative event-to-paint timing. The
frame probe composes a frame explicitly, so it does not prove automatic paint
latency. These restrictions prevent extrapolating the passing matrix to the
observed production session.

## Findings and remaining hypotheses

- Direct feeds are real production paths. They open child sockets and consume
  events in `agents/controller_subagent_feed.rs`; do not describe all feeds as
  inspection-only.
- No four-entry cutoff found. Warm/retained cap is 30; the event loop has separate
  master and child receive arms with unbiased selection. Worker-count changes
  are not established root-cause fixes.
- `controller_ledger_sync.rs` loses retry intent on refused enqueue and ignores
  continuation enqueue failure. This is a conditional lost-refresh risk, not
  proof that larger rosters trigger it.
- Socketless feeds have initial inspection requests without ongoing direct
  subscription; append hints are ignored by `shell/app_events.rs`. These are
  adjacent gaps already called out by the issue, not a main-cause explanation.
- Read-only investigation identified another candidate: attachment after a
  child's start event. Sessions start `running: false`; child `get_state` does
  not initialize run-state; authoritative focused chat and live buffering gate
  idle events in `controller_subagent_stream.rs`. This was source inspection,
  not an executed reproduction, and is not roster-size-specific. It also does
  not establish refocus-only recovery.
- Nested append forwarding unconditionally restamps identity in harness
  `subagent_monitor_canonical.rs`; direct token streaming does not depend on
  those ignored append hints. No causal claim for this issue or #1608.

Next useful evidence is a capture from a failing producing session: selected
UUID, total/live roster counts, connection kind, child receive sequence,
queue refusal/lag, target/applied revision, pending sync and projection/redraw
timing, comparing before/after refocus. Repeat with connect-before-start versus
mid-turn attach and force final-refresh loss with no later hint independently.

## Test evidence

- `cargo test -p quecto-tui --lib issue_1605_direct_roster_workload_matrix`:
  **PASS**, all 30 scenarios in the one test (0.48 s after build).
- `cargo fmt --all -- --check`: **PASS** after installing missing rustfmt.
- `cargo clippy -p quecto-tui --all-targets -- -D warnings`: **PASS** after
  installing missing clippy.
- `cargo test -p quecto-tui --lib --bins`: **2100 passed, 2 failed** in lib;
  command stopped before bins. Both failures also fail individually:
  - `shell::app::app_git_tests::git_branch_refresh_task_reflects_branch_switches_promptly`
  - `shell::child_watch::tests::terminate_after_reap_kills_surviving_group_members`
- Removed only the new test-module registration temporarily and repeated the
  same package command against unchanged production/test sources: **2099 passed,
  the same 2 failed**. Registration restored afterwards. Thus these are baseline
  failures in this container; their underlying causes were not diagnosed here.

Local detailed outputs: `/tmp/1605-matrix.log`, `/tmp/1605-tui-tests.log`,
`/tmp/1605-baseline-tests.log`, `/tmp/1605-clippy.log`,
`/tmp/1605-git-retest.log`, `/tmp/1605-child-retest.log`.

No RED/ GREEN or fix-removal falsifiability claim: there is no production fix.
No PR, push, issue closure, or completed adversarial-review workflow claimed.
