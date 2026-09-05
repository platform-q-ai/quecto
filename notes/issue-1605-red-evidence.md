# #1605 continued RED investigation

Supersedes the initial checkpoint's statement that no direct stale-until-refocus
sequence was reproduced. The historical production session's roster-size
correlation remains unconfirmed.

## New reproduction

The earlier matrix advertised `capabilities.sync=1`, unlike the current harness
`slim_state_projection` (`interface/cli/uds_state_projection.rs`). Idle query and
busy connect/get_state use that projection and omit `sync`; the underlying
session snapshot still sets sync=1. Both idle and busy Sync request handlers
serve valid deltas independent of this advertisement.

With the actual slim state shape, the real-socket TUI tests fail:

```
cargo test -p quecto-tui --lib issue_1605_direct_slim_state
3 failed
issue_1605_direct_slim_state_committed_checkpoint_visible_without_refocus:
  direct checkpoint stayed stale until refocus: roster=8 live=1;
  initial sync succeeded, ledger hint received, but no automatic sync followed
issue_1605_direct_slim_state_final_refresh_large_mixed_roster:
  child-00: expected sync on direct socket before deadline
issue_1605_direct_slim_state_final_refresh_single_control:
  child-00: expected sync on direct socket before deadline
```

Full output: `/tmp/1605-slim-red.log`.

The checkpoint test asserts the correct visible automatic refresh contract,
then independently confirms stale-focus recovery does display the committed
checkpoint before asserting that automatic refresh was missing. The larger
mixed case uses eight rows/four live direct sockets, with parent links; all
stream tokens display before the final committed refresh stalls. No inspection
feed, queue refusal, dropped token, or forced transport loss is involved.

The single-row control fails too: **this proves a real direct refresh defect,
not that roster growth causes it or that it is the only production cause**.
Refocus bypasses capability gating; `note_ledger_advanced` does not. Initial
successful sync currently does not establish capability, so the feed remains
stuck behind the missing advertisement.

## Falsifiability and proposed correction

Successful typed sync is stronger evidence of sync support than an optional
state field. Latch that evidence and do not let later slim state responses
clear it. Test remains red on unchanged production implementation; removing
this correction must restore failure. Avoid inventing a four-feed threshold.

Acceptance checklist remains in `issue-1605-investigation.md`. Also cover
periodic inspection refresh, final lost/refused work recovery, retained
scroll/follow-tail, and live direct cases without conflating their causes.
No production changes preceded this RED run.

## Additional RED/GREEN and same-class sweep

- Four automatic-recovery tests failed before the timer implementation: refused
  final sync, accepted request/lost response, refused pagination continuation,
  and idle inspection-only cadence. They run `App::run()` without another hint.
  Output `/tmp/1605-cadence-red.log`; all now pass.
- Searched production `supports_sync`, `pending_rev`, Sync enqueue, and sync
  response dispatch sites. There is one shared child capability latch, used by
  both direct and routed feeds. Fixed there; routed responses inherit success
  validation. Focus and pagination enqueues are covered by autonomous retry.
- Same-class delayed-response test showed cursor rollback 9 -> 1. Added a
  same-epoch applied-cursor guard before transcript mutation; test now passes.
  Output `/tmp/1605-stale-red.log`.
- Slim state after a successful sync cannot disable hints; failed sync cannot
  prove support or project data. Both have targeted tests.
- Current #1605 tests: 11 passed (`/tmp/1605-green.log`). No modification to
  scroll/follow-tail implementation. README documents cadence and limits.

The source-inspected missed-start latch and nested append identity issues are
not the same capability/refresh defect. They remain separate hypotheses, not
silently claimed fixed by this change. No roster-size causal threshold claimed.

## Fix-removal proof and validation

Temporarily restored all three changed production files to `4344899e`, leaving
new tests intact: `cargo test -p quecto-tui --lib issue_1605` gave **1 passed,
10 failed**. Restored the fix immediately. `/tmp/1605-fix-removal.log`.
After restoration and additional cross-tab coverage: **12/12 #1605 tests pass**.
Package lib suite: **2110 passed, 2 baseline failures** before the last cross-tab
test; same two failures previously reproduced on unchanged baseline. Bin target
passes (0 tests). Strict pre-push Clippy flags pass after extracting the fixture
handshake (no lint suppression). Formatting passes.

The first local review overlapped the intentional fix-removal window and
reported the expected red suite as a blocker. That report is invalid as a
review of the restored fix; a stable-worktree re-review was requested. Its
coverage observations prompted explicit periodic per-tab namespace/cursor
coverage. Existing App::run tests already cover lost/refused work independently
of the manual-pump live matrix.

## Stable review finding and correction

Stable adversarial review found that a ledger hint overwrote `feed.epoch` while
leaving the old applied revision, defeating the periodic cursor and lower-rev
rollover. Added `issue_1605_epoch_hint_keeps_applied_cursor_for_refused_retry`:
RED output `/tmp/1605-epoch-red.log`. Hints now leave the applied epoch/revision
pair untouched, and requests send that pair so the producer detects an epoch
mismatch and supplies a full resync. Epoch mismatch requests also run for a
zero-revision target. Updated two existing initial-request assertions to expect
the applied epoch (0) rather than the un-applied hinted epoch (1).

After correction: package lib **2112 passed, same 2 baseline failures**;
all 13 #1605 tests pass within that run. `/tmp/1605-epoch-suite.log`.

## Authoritative CI first run

Run `33961396569` on `39f55203`: workspace tests, coverage, static quality,
dependency policy, review threads, mock LLM E2E and non-real BDD passed. TUI BDD
failed because its sibling initial-request assertion still expected hinted
epoch 1. Updated it to applied epoch 0, matching the two corrected unit tests;
no assertion removed. Focused BDD: 1 scenario/5 steps passed. This CI failure
was introduced by the intentional epoch contract correction, not a baseline
failure. Logs `/tmp/1605-ci-bdd-failure.log`, `/tmp/1605-bdd-retry.log`.
Stable local re-review reported no remaining verified blockers and 13/13
issue tests passed after the epoch correction.
