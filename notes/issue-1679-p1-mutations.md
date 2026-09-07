# #1679 P1 per-assertion mutation evidence

This is **post-implementation assertion mutation**, not pre-implementation per-assertion RED. The initial missing-API compile RED is insufficient to establish assertion falsifiability. Each source assertion was inverted alone (`assert_eq!` → `assert_ne!`; `assert!(e)` → `assert!(!(e))`), its enclosing test/scenario run, the intended assertion panic verified (exact panic line for contracts; matched Then step and assertion payload for BDD), the original bytes restored, and the same target rerun GREEN. Loop assertions are covered per source assertion, not separately per iteration. No production changes were made by this mutation task.

## Commands and scope

Shared existing Cargo cache: `CARGO_TARGET_DIR=/cargo-target`; serial runs, no broad suites.

- Contract baseline/final: `cargo test -q -p quecto-agentic-harness --test contracts inference_admission::` (19 passed).
- Each contract: same command with `inference_admission::<test> -- --exact` (one selected test).
- BDD baseline/final: `QUECTO_TAG=inference-admission cargo test -q -p quecto-agentic-harness --features test-support --test bdd` (3 scenarios passed).
- Each BDD mutation: append `-- --name '^<escaped scenario name>$'` (one scenario).
- Local runner: `/tmp/run-admission-mutations.py`; logs `/tmp/admission-mutations/<kind>-<line>-{red,green}.log`; machine results `results.json`; baseline/final logs `<kind>-baseline.log`, `<kind>-final-green.log`.

Parent applied a production reserve-starvation fix during this task. The existing reserved-grants trace and all subsequent selected tests stayed green after restoration. The independent new `contracts/admission_dispatcher.rs` regression was not mutated. Source paths/lines below describe the original files before the planned architecture-driven module split.

## Every source assertion

All rows: RED = nonzero exit with **intended assertion failure** (contract panic file:line; BDD matched Then attribute location plus assertion payload), not compile/setup failure; GREEN = exit 0, one selected test/scenario passed. Each row has both dedicated logs named above.

### `quecto-agentic-harness/tests/contracts/inference_admission.rs` — 86 assertions

| Line | Enclosing test / step (scenario for BDD) | Inversion | RED exit | Restored |
|---:|---|---|---:|---|
| 52 | `configuration_rejects_invalid_boundaries_and_incomplete_mapping` | negate | 101 | GREEN |
| 88 | `configuration_rejects_invalid_boundaries_and_incomplete_mapping` | negate | 101 | GREEN |
| 93 | `configuration_rejects_invalid_boundaries_and_incomplete_mapping` | eq → ne | 101 | GREEN |
| 120 | `hierarchical_round_robin_fifo_does_not_reward_wide_roots` | eq → ne | 101 | GREEN |
| 141 | `shared_slots_use_three_to_one_and_fallback_is_work_conserving` | eq → ne | 101 | GREEN |
| 167 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 168 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 169 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 171 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 172 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 173 | `reserve_is_inside_capacity_and_children_cannot_borrow_it` | eq → ne | 101 | GREEN |
| 182 | `idle_parent_owns_no_slot_and_group_b_is_independent` | eq → ne | 101 | GREEN |
| 184 | `idle_parent_owns_no_slot_and_group_b_is_independent` | eq → ne | 101 | GREEN |
| 192 | `idle_parent_owns_no_slot_and_group_b_is_independent` | eq → ne | 101 | GREEN |
| 193 | `idle_parent_owns_no_slot_and_group_b_is_independent` | eq → ne | 101 | GREEN |
| 203 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 204 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 208 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 209 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 213 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 218 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 222 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 223 | `queue_full_exact_deadline_and_cancel_never_dispatch` | eq → ne | 101 | GREEN |
| 233 | `active_cancel_and_attempt_deadline_require_confirmed_completion` | eq → ne | 101 | GREEN |
| 242 | `active_cancel_and_attempt_deadline_require_confirmed_completion` | eq → ne | 101 | GREEN |
| 243 | `active_cancel_and_attempt_deadline_require_confirmed_completion` | eq → ne | 101 | GREEN |
| 245 | `active_cancel_and_attempt_deadline_require_confirmed_completion` | eq → ne | 101 | GREEN |
| 253 | `active_cancel_and_attempt_deadline_require_confirmed_completion` | eq → ne | 101 | GREEN |
| 267 | `pacing_is_not_refunded_and_cooldown_only_extends` | eq → ne | 101 | GREEN |
| 268 | `pacing_is_not_refunded_and_cooldown_only_extends` | eq → ne | 101 | GREEN |
| 272 | `pacing_is_not_refunded_and_cooldown_only_extends` | eq → ne | 101 | GREEN |
| 273 | `pacing_is_not_refunded_and_cooldown_only_extends` | eq → ne | 101 | GREEN |
| 274 | `pacing_is_not_refunded_and_cooldown_only_extends` | eq → ne | 101 | GREEN |
| 285 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 287 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 291 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 293 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 305 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 306 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 307 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 309 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 310 | `duplicate_and_conflicting_acquire_terminal_eviction_and_scope_limits` | eq → ne | 101 | GREEN |
| 323 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 324 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 329 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 333 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 336 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 340 | `unknown_identity_and_completion_do_not_release_other_attempts` | negate | 101 | GREEN |
| 341 | `unknown_identity_and_completion_do_not_release_other_attempts` | eq → ne | 101 | GREEN |
| 353 | `cooldown_excess_and_arithmetic_overflow_fail_closed` | eq → ne | 101 | GREEN |
| 358 | `cooldown_excess_and_arithmetic_overflow_fail_closed` | negate | 101 | GREEN |
| 362 | `cooldown_excess_and_arithmetic_overflow_fail_closed` | eq → ne | 101 | GREEN |
| 363 | `cooldown_excess_and_arithmetic_overflow_fail_closed` | negate | 101 | GREEN |
| 379 | `completion_feedback_is_once_even_after_later_success` | eq → ne | 101 | GREEN |
| 380 | `completion_feedback_is_once_even_after_later_success` | eq → ne | 101 | GREEN |
| 385 | `completion_feedback_is_once_even_after_later_success` | eq → ne | 101 | GREEN |
| 386 | `completion_feedback_is_once_even_after_later_success` | eq → ne | 101 | GREEN |
| 397 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 405 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 413 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 414 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 416 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 418 | `active_cancellation_before_deadline_and_retirement_preserve_capacity` | eq → ne | 101 | GREEN |
| 437 | `shorter_feedback_during_cooldown_cannot_shorten_it` | eq → ne | 101 | GREEN |
| 439 | `shorter_feedback_during_cooldown_cannot_shorten_it` | eq → ne | 101 | GREEN |
| 440 | `shorter_feedback_during_cooldown_cannot_shorten_it` | eq → ne | 101 | GREEN |
| 458 | `reserved_grants_do_not_consume_shared_arbitration_turns` | eq → ne | 101 | GREEN |
| 469 | `reserved_grants_do_not_consume_shared_arbitration_turns` | eq → ne | 101 | GREEN |
| 477 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 478 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 480 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 484 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 485 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 486 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 488 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 493 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 494 | `regressions_and_unknown_completion_fail_without_unsafe_dispatch` | eq → ne | 101 | GREEN |
| 510 | `accepted_cooldown_and_dispatch_deadlines_cannot_wrap_time` | negate | 101 | GREEN |
| 511 | `accepted_cooldown_and_dispatch_deadlines_cannot_wrap_time` | eq → ne | 101 | GREEN |
| 521 | `accepted_cooldown_and_dispatch_deadlines_cannot_wrap_time` | eq → ne | 101 | GREEN |
| 522 | `accepted_cooldown_and_dispatch_deadlines_cannot_wrap_time` | eq → ne | 101 | GREEN |
| 534 | `saturated_group_queue_and_evicted_completion_do_not_affect_live_work` | eq → ne | 101 | GREEN |
| 536 | `saturated_group_queue_and_evicted_completion_do_not_affect_live_work` | negate | 101 | GREEN |
| 542 | `saturated_group_queue_and_evicted_completion_do_not_affect_live_work` | eq → ne | 101 | GREEN |
| 543 | `saturated_group_queue_and_evicted_completion_do_not_affect_live_work` | eq → ne | 101 | GREEN |
| 567 | `drained_roots_and_agents_rejoin_without_resetting_peers_turns` | eq → ne | 101 | GREEN |

### `quecto-agentic-harness/tests/bdd/inference_admission_steps.rs` — 5 assertions

| Line | Enclosing test / step (scenario for BDD) | Inversion | RED exit | Restored |
|---:|---|---|---:|---|
| 66 | `child_admitted` — An idle parent does not consume its child's inference capacity | eq → ne | 101 | GREEN |
| 89 | `no_dispatch` — Cancelling queued inference prevents later dispatch | eq → ne | 101 | GREEN |
| 93 | `no_dispatch` — Cancelling queued inference prevents later dispatch | eq → ne | 101 | GREEN |
| 97 | `no_dispatch` — Cancelling queued inference prevents later dispatch | eq → ne | 101 | GREEN |
| 112 | `pacing_boundary` — Confirming attempt completion does not refund request pacing | eq → ne | 101 | GREEN |

BDD location limitation: cucumber captures the assertion payload but reports the matched step attribute rather than the assertion panic line. BDD rows 66, 89, 93, 97, 112 map to matched attribute lines 64, 87, 87, 87, 110 respectively. The three no-dispatch assertions have distinct payloads (queued state, cancelled state, observed vector); only the listed assertion was inverted. An initial location-check rejection for line 66 was a runner validation mismatch, not a surviving mutation; the BDD mutation was rerun with step-location verification.

## Exact restoration

- `quecto-agentic-harness/tests/contracts/inference_admission.rs` SHA-256: `7337abae896b214a310e2545f7461b3a9a1b097972358d8a719ada27cf7f5bd0` (before = after).
- `quecto-agentic-harness/tests/bdd/inference_admission_steps.rs` SHA-256: `91e727f47ce7bc72a3198bd81f62947b088209464e3eb5438db438a46f654274` (before = after).

Total: **91 / 91 intended assertion failures**, each followed by its restored targeted GREEN; final 19-contract / 3-scenario GREEN.

Final naming/refactor: contracts/inference_admission.rs renamed admission_client.rs
unchanged behavior to satisfy public-port module convention. Invalid-config assertion
now includes Debug config failure context; boolean equality false uses idiomatic
negation. These preserve prior falsifiability. Additional registry three inversions
and dispatcher actual behavioral RED are documented in p1-red-evidence. Final full
contracts 98/98 and architecture 46/46 passed after refactor.
