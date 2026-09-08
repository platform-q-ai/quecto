# P2 AC5 loopback feedback transport RED / oracle sensitivity

## Scope and reproducible command

Owned test files:
- `quecto-agentic-harness/tests/inference_admission_feedback_transport.rs`
- `quecto-agentic-harness/tests/common/admission_feedback_fixture.rs`

No production changes. All HTTP requests go to ephemeral 127.0.0.1 listeners through explicitly unproxied clients. No paid requests. Fixture server tasks have RAII abort ownership.

```sh
cargo test -p quecto-agentic-harness --test inference_admission_feedback_transport -- --nocapture --test-threads=1
```

Observed: **1 passed (oracle sensitivity), 15 failed (intentional transport RED), 0 ignored**. Full local output: `/tmp/admission-feedback-red.log`. The parallel run also gave 1 passed / 15 failed in 0.33s. All transport failures are named acceptance assertions, not timeout failures.

## Oracle design / falsifiability

`header_oracle`, `group_oracle`, and `sse_oracle` are pure functions used by both the actual HTTP tests and `every_acceptance_assertion_rejects_its_synthetic_counterexample`. Each returns every labeled condition before `verify` prints PASS/RED for all IDs and asserts the complete failure set. Thus later acceptance conditions are no longer masked by an earlier panic.

The sensitivity test constructs valid observation records, asserts all their oracle conditions are true, mutates an observed fact for each ID, checks that the same oracle makes that ID false, and invokes the actual `verify` assertion on that condition under `catch_unwind`. It requires an assertion panic, printing `FALSIFIED <ID>`. Expectations and production are never perturbed. This proves rejection sensitivity, **not production correctness**. Logical implications may be vacuously true in current RED; their counterexamples establish sensitivity independently.

Observed: **63 caught, expected assertion panics**: H01–H13 once each, G01–G12 once each, S01–S10 across all pre/post-text × throttle/non-throttle variants (S06 applies only to the two throttle variants). The test itself PASSED. Panic-hook output from those caught panics is expected.

## Header assertion IDs

Actual cases: `openai_429_receipt_precedes_stalled_body`, `responses_429_receipt_precedes_stalled_body`, `oauth_429_receipt_precedes_stalled_body`, `anthropic_529_receipt_precedes_stalled_body`.

Each actual case currently reports RED H07,H08,H09,H12 and PASS for every other H ID. Every ID has independent synthetic rejection evidence:

| ID | Acceptance condition | Actual | Falsifying observed fact |
|---|---|---|---|
| H01 | Exactly one physical POST | PASS | 2 sends |
| H02 | Request not complete while body withheld | PASS | premature task completion |
| H03 | Exactly one terminal error | PASS | duplicate error |
| H04 | Exact existing HTTP error bytes | PASS | rewritten error |
| H05 | No text on HTTP failure | PASS | unexpected text |
| H06 | No false Done | PASS | Done count 1 |
| H07 | Exactly one header receipt before body | RED: 0 vs 1 | duplicate receipt |
| H08 | Finite Until hint | RED: no receipt | Unavailable instead |
| H09 | Exactly one grant | RED: 0 vs 1 | missing grant |
| H10 | No completion at receipt | PASS | premature finish |
| H11 | Billing body does not retract/reanchor receipt | PASS, currently empty==empty | changed deadline 30000→60000 |
| H12 | Exactly Failure completion, not throttle fallback | RED: empty vs Failure | Throttle completion |
| H13 | No abandoned transport permit | PASS | abandoned count 1 |

The fixture sends real candidate headers first, publishes a connection barrier, and waits on a oneshot before sending the billing body. Header receipt observation has a bounded 300ms grace window so missing implementation reaches H07/H08 rather than ending in a timeout-only panic. Header time arithmetic is not derived from this grace window.

## Group assertion IDs / no wall sleep negative oracle

Actual test: `receipt_blocks_same_group_with_spare_capacity_but_not_unrelated_group`.

The old quiet-window sleep is removed. There is **no `sleep` in the suite or fixture**. The sibling is synchronized by a positive `select!` barrier:
1. the fixture admission port records that acquire is parked behind received cooldown feedback; or
2. the real sibling server receives a physical POST, positively proving bypass.

Only then are physical-send counts observed. The gate has no occupancy cap, so the first stalled body cannot itself park the sibling. Reopen is an explicit fixture command, not elapsed wall time. `G12` independently checks the blocked-acquire barrier was reached. No deadline expiry / production policy correctness is claimed by this recording gate.

| ID | Acceptance condition | Actual | Falsifying observed fact |
|---|---|---|---|
| G01 | Initial physical sends = 1 | PASS | 2 |
| G02 | Unrelated group physical sends = 1 | PASS | 0 |
| G03 | Unrelated Done count = 1 | PASS | 0 |
| G04 | Unrelated request has no error | PASS | error |
| G05 | Same-group sends before reopen = 0 | RED: 1 vs 0 | bypass POST |
| G06 | Two shared acquire requests | RED: 0 vs 2 | only 1 queued |
| G07 | Only first shared grant before reopen | RED: 0 vs 1 | 2 grants |
| G08 | No shared finish while first body withheld | PASS | premature finish |
| G09 | Exactly one sibling send after reopen | PASS | replay count 2 |
| G10 | Exactly two total shared grants | RED: 0 vs 2 | 1 |
| G11 | Exactly one unrelated grant | RED: 0 vs 1 | 0 |
| G12 | Positive parked-acquire observation | RED: 0 | no parked acquire |

## SSE assertion IDs

Six typed-positive cases: Responses API-key, OAuth and Anthropic, each before/after visible text. Four negatives: Responses/Anthropic billing overrides and throttle-looking human prose. They use the real existing RetryingProvider(no_delay(3)), not a retry fake.

Actual typed-positive cases RED: S05,S06,S07,S08,S09. Actual negative cases RED: S07,S08,S09. All other conditions PASS. S06 is explicitly a guarded NoHint requirement, not a receipt requirement for terminal errors.

| ID | Acceptance condition | Falsifying observed fact |
|---|---|---|
| S01 | Exactly one physical POST, no replay | 2 sends |
| S02 | Exact pre/post-text transcript | additional/replayed text |
| S03 | Exact original single error string | rewritten error |
| S04 | No false Done | Done count 1 |
| S05 | One typed receipt / zero terminal receipts | extra receipt |
| S06 | Typed SSE without headers reports NoHint | Invented Until(30000) deadline |
| S07 | Exactly one grant | 0 |
| S08 | Exactly one transport completion | duplicate completion |
| S09 | Completion Failure, no second fallback | Throttle completion |
| S10 | No abandoned permit | abandoned count 1 |

S01–S05 and S07–S10 counterexamples ran four times each (all pre/post × typed/terminal variants). S06 ran twice, for both text variants with typed throttling.

## API / policy ownership boundary

Typed SSE errors without provider hints now require `ThrottleFeedback::NoHint { jitter }`, forwarded by the default `AttemptPermit::throttle_without_hint` method. Header feedback still requires `Until`. The domain group alone calculates fallback and owns escalation/consecutive-throttle counters. The earlier proposed `fallback_ms` receipt-clock field is withdrawn: leaves must not select an arbitrary fallback duration. No production method was added by this test task.

These loopback tests currently use Retry-After 30 seconds only as a finite wire hint, not as a retry cap or policy maximum. They do **not** claim 90-second authority-clock deadline coverage. Parent-owned normalizer/policy tests cover exact 90-second boundaries independently of the existing retry decorator's 30-second cap. This suite checks receipt delivery, finite header hint / typed NoHint shape, unchanged deadline across body completion, transport accounting, and isolation; exact timestamp arithmetic remains in those parent contracts.

## Non-acceptance fixture guards

Request framing size/content-length bounds, POST shape checks, channel/join results, unexpected stream events, and five-second fixture deadlines are setup/safety guards, not AC5 acceptance IDs. They were not deliberately broken as part of the 63 semantic oracle counterexamples. No transport test failed at these guards.

## GREEN integration contract correction: NoHint (later verification)

Updated S06 to require `NoHint { .. }` for the headerless typed SSE cases; H08 still requires `Until`. The valid synthetic state now contains `NoHint { jitter: u64::MAX }`; the S06 counterexample supplies an invented `Until(30000)` deadline. Both pre/post-text counterexamples print `FALSIFIED S06`. The sensitivity-only command passed (1 passed, 15 filtered), with the updated exact assertion invoked and caught before the final integrated full-suite verification. No production files were changed for this correction.

```sh
cargo test -p quecto-agentic-harness --test inference_admission_feedback_transport every_acceptance_assertion -- --nocapture
# 1 passed; 0 failed; 15 filtered; 0.00s
cargo test -p quecto-agentic-harness --test inference_admission_feedback_transport -- --nocapture --test-threads=1
# 16 passed; 0 failed; 0 ignored; 0.04s
```

Logs: `/tmp/admission-feedback-nohint-sensitivity.log`, `/tmp/admission-feedback-nohint-green.log`. Leaf GREEN implementation arrived concurrently; the old RED section above remains historical evidence, not the current suite status.

The first concurrent full run exposed a fixture-only self-notification busy loop once production began calling acquire: parked notification woke its own acquisition retry future. Fixed by separating `parked_changed` (observer barrier) from `changed` (explicit reopen). The final run confirms the explicit barrier works without wall sleeps. This change neither simulates production admission nor changes policy.

Attempted direct coordination with leaf agent label `573`, but this child session's agent namespace has no such live agent and inventory is empty. Parent must relay that these two fixture/test files remain owned by this task; no leaf agent edits are needed.
