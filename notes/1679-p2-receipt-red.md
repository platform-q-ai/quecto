# #1679 P2 step 6: bounded receipt-contract RED evidence

## Scope and status

Owned files only:
- `quecto-agentic-harness/tests/contracts/admission_feedback.rs`
- `notes/1679-p2-receipt-red.md`

Reviewed `1679-p2-matrix.md` and `1679-p2-test-design.md`, including the fresh-review
requirements for sibling overlap, independent success behavior, queued/unknown/
terminal rejection or idempotence, and unavailable isolation before/after finish.
No production implementation was changed. The application `report_feedback` port
still returns `Err(AdmissionError::Unavailable)` without applying feedback.
**This is RED evidence, not GREEN, mutation testing of a finished implementation,
or completion of the overall P2 workflow.** Parent owns integration/workflow.

The original seven tests had multiple assertions, most hidden by receipt setup
`unwrap()`. There are now **29 independently runnable, single-oracle tests**.
Acceptance/rejection assertions are separate from state-effect assertions. Effect
tests intentionally discard receipt results and proceed to their own observable
oracle. The `receipt` helper merely forwards to the actual application port; it
does not model policy or pretend acceptance. Existing fixture setup still unwraps
configuration/enqueue/grant operations, which succeed in every recorded run.
Completion in effect scenarios is likewise observed via the final state rather
than allowed to mask its oracle with an intermediate result panic.

The three invalid-lifecycle accounting tests compare an entire `GroupSnapshot` as
one unchanged-accounting oracle, independently of their respective rejection
result tests. The mutation evidence below demonstrates changed accounting; it
is not a separate mutation claim for every field of that snapshot.

## Commands and natural behavioral failures

```sh
cargo test -p quecto-agentic-harness --test contracts admission_feedback
```

Initial revised run: **exit 101, 20 failed, 9 passed, 107 filtered out**.
Log: `/tmp/1679-p2-receipt-baseline.log`.
Every failure is at its test's final behavioral assertion, not at setup unwrap.
The nine naturally passing tests protect behaviors that a no-op stub happens to
preserve; those received the reverted negative-control demonstrations below.

Natural failure mapping (all names have module prefix `admission_feedback::`):

| Test | Actual versus expected |
|---|---|
| `active_receipt_is_accepted` | `Err(Unavailable)` vs `Ok(())` |
| `duplicate_receipt_is_accepted` | `Err(Unavailable)` vs `Ok(())` |
| `unavailable_receipt_is_accepted` | `Err(Unavailable)` vs `Ok(())` |
| `terminal_duplicate_receipt_is_accepted` | `Err(Unavailable)` vs `Ok(())` |
| `conflicting_receipt_is_rejected` | `Err(Unavailable)` vs `Err(Conflict)` |
| `queued_receipt_is_rejected` | `Err(Unavailable)` vs `Err(Conflict)` |
| `new_terminal_receipt_is_rejected` | `Err(Unavailable)` vs `Err(Conflict)` |
| `unknown_receipt_is_rejected` | `Err(Unavailable)` vs `Err(UnknownRequest)` |
| `receipt_blocks_spare_capacity_before_deadline` | `Ok(Some(request 2))` vs `Ok(None)` at 89 |
| `late_completion_does_not_reanchor_receipt_deadline` | cooldown `0` vs `90` |
| `duplicate_receipt_does_not_reanchor_deadline` | cooldown `0` vs `90` |
| `conflicting_receipt_cannot_extend_deadline` | cooldown `0` vs `90` |
| `shorter_receipt_does_not_shorten_deadline` | cooldown `0` vs `90` |
| `overlapping_siblings_merge_longer_deadline` | cooldown `0` vs `90` |
| `overlapping_siblings_keep_longer_deadline_in_reverse_order` | cooldown `0` vs `90` |
| `sibling_success_does_not_shorten_deadline` | cooldown `0` vs `90` |
| `terminal_duplicate_receipt_does_not_reapply_deadline` | cooldown `0` vs `90` |
| `unavailable_receipt_marks_group_unavailable` | unavailable `false` vs `true` |
| `unavailable_denies_dispatch_before_completion` | `Ok(Some(request 2))` vs `Err(Unavailable)` |
| `unavailable_denies_dispatch_after_completion` | `Ok(Some(request 2))` vs `Err(Unavailable)` |

## Reverted equivalent fault demonstrations for the other nine oracles

No production files were mutated. Temporary changes in the owned contract file
injected faulty side effects through existing public P1 operations, or omitted
one completion action. Expected values/oracles were **not changed**. These are
negative controls for assertion reachability/sensitivity, not implementations of
receipt policy. Each mutation started from the same original test source, and a
`finally` restored that source. All nine executions reached the target assertion
and returned **exit 101, 0 passed, 1 failed**.

Executed driver:

```sh
python3 /tmp/1679-p2-receipt-mutations.py \
  > /tmp/1679-p2-receipt-mutations.log 2>&1
```

The driver executed this command for each test named below:

```sh
cargo test -p quecto-agentic-harness --test contracts \
  admission_feedback::<test_name> -- --exact
```

Detailed mutation recipes (apply only while demonstrating, then revert):

| ID | Temporary fault in test-side action | Test(s) and actual assertion failure |
|---|---|---|
| M1 early release | In forwarding `receipt`, call real `report_feedback`, then `service.complete(scope, sequence, Feedback::Failure, now)`, discarding that completion result and returning the real receipt result | `receipt_retains_transport_occupancy`, `unavailable_receipt_retains_transport_occupancy`, `sibling_success_releases_only_completed_transport`: active `0` vs `1` in each independent run |
| M2 overlong cooldown | After the real receipt call, complete that request with `Feedback::Throttle { delay_ms: 91 }` at receipt time 1 (P1 establishes deadline 92) | `receipt_allows_spare_capacity_at_exact_deadline`: `Ok(None)` vs `Ok(Some(request 2))` at 90 |
| M3 lost completion | Remove the effect scenario's `let _ = service.complete(scope, 1, Feedback::Failure, 50);` | `completion_releases_receipt_bearing_transport`: active `1` vs `0` |
| M4 queued misattribution | After real receipt call, complete active request 1 with `Feedback::Throttle { delay_ms: 88 }` at time 2, rather than leaving the queued request's rejection side-effect free | `queued_receipt_cannot_change_accounting`: snapshot active `0`, cooldown `90` vs active `1`, cooldown `0` (queued remains `1`, unavailable false) |
| M5 unknown treated as completion | After real receipt call, call existing `complete` for that unknown sequence; P1 unknown-completion semantics mark groups unavailable | `unknown_receipt_cannot_change_accounting`: snapshot unavailable `true` vs `false`, other fields equal |
| M6 terminal advice reapplied | After real receipt call, enqueue request 100 to `account` at time 2, grant it via `next(shared, 2)`, then complete it with `Throttle { delay_ms: 88 }`, establishing a real observable cooldown | `new_terminal_receipt_cannot_change_accounting`: snapshot cooldown `90` vs `0`, other fields equal |
| M7 globally poisoned availability | After real receipt call, call existing `complete(scope, 999, Feedback::Failure, now)` to demonstrate unrelated-group poisoning | `unavailable_does_not_block_unrelated_group`: `Err(Unavailable)` vs `Ok(Some(request 2))` in other group |

Per-run logs are `/tmp/1679-p2-receipt-<ID-label>-<test_name>.log`, with ID-labels
`M1-early-release`, `M2-overlong-cooldown`, `M3-lost-completion`,
`M4-queued-misattributed`, `M5-unknown-as-completion`, `M6-terminal-reapplied`,
`M7-global-unavailable`. The summary log records actual/expected assertion text.
The restored pre-format source SHA256 was
`b65926a01662917b7746cc1036dd7928b910d8e0ccafffa560b3db8d9c501f8d`.
These `/tmp` artifacts are local supporting logs; this note records the commands,
recipes, and outcomes without requiring those artifacts to survive integration.

## Restoration and verification

Initially `rustfmt --edition 2024 ...` failed because the component was missing;
no test command ran in that `&&` chain. Installed it with
`rustup component add rustfmt`, then formatted only the owned Rust file:

```sh
rustfmt --edition 2024 quecto-agentic-harness/tests/contracts/admission_feedback.rs
cargo test -p quecto-agentic-harness --test contracts admission_feedback \
  > /tmp/1679-p2-receipt-restored.log 2>&1
```

Restored run: **exit 101; same 20 assertion failures, 9 passes, 107 filtered out**.
No temporary fault code remains. No full-suite or GREEN claim.

## What remains

- Implement receipt policy inward in a later parent-owned GREEN step; this
  subtask intentionally leaves all twenty missing-behavior failures intact.
- Re-run these contracts against the implementation. Nine no-op-compatible
  assertions passing today do not imply accepted receipt behavior; acceptance
  tests are still RED, and the temporary faults are only equivalent demonstrations.
- After GREEN, use localized **production** mutations for precise algorithmic
  faults (overwrite-vs-max, reanchor, accept-conflicting-report, release-on-receipt,
  global unavailable, replay after terminal). Natural cooldown `0` failures prove
  the assertions execute, not that those subtler faulty algorithms were exercised.
- Bounded report-history/older-replay policy, shared fallback count and next-delay
  duplicate observation, injected jitter, header parsing/overflow/wall-clock cases,
  transport receipt barriers, runtime rebuilding, and all remaining P2 matrix rows
  remain outside this bounded receipt split. `ThrottleFeedback::Unavailable` here
  is already-normalized advice, not proof of excessive-header normalization.
- Parent must reconcile the earlier seven-test summary in the general evidence
  note and assess overall step 6 sufficiency. This subtask does not advance it.
