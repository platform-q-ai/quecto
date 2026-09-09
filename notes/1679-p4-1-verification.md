# #1679 P4 slice 1 — observation state (verification)

RED first: `tests/contracts/admission_observation.rs` failed to compile against
master (no `AdmissionPhase`, `AdmissionObservation`, `AdmissionRecorder`,
`ObservedAdmission`). GREEN: 2 contract scenarios over a real authority
(waiting → admitted → cooldown → completed with elapsed wait and freshness;
queue-deadline refusal with reason, dropped wait as cancellation, bounded live
view), 3 recorder unit tests (64-entry bound and leak-free waits, truncated
refusal reason, dropped permit = cancellation / finished = completion with
cooldown), process binding exposes `observation()`, BDD
`inference_admission_observation.feature` (tagged lane 16 scenarios / 87 steps).

Design: an `ObservedAdmission` decorator around each remote gate records
transitions; the attempt transport, permit semantics and lifecycle enums are
untouched (ADR-0024/0026). Aggregates saturate; live attempts are capped at 64;
authority cooldown deadlines are translated onto the process clock through the
permit receipt. Mutation and review records are appended below.

## Adversarial review 1 (2026-09-09) and fixes

Nine findings (4 medium, 5 low/info), all fixed test-first:

|Finding|Fix|Proof|
|---|---|---|
|M1 cooldown anchored at feedback time, drifting by the transport duration|`Until` advice is anchored at the attempt's grant instant (authority `deadline - receipt` offset added to the local admitted time)|`cooldown_is_anchored_at_the_grant_and_kept_per_group`|
|M2 cooldown/refusal process-global|per-group `GroupActivity { cooldown, last_refusal }`|same test + `refusals_and_cancellations_are_observed_per_group_with_reasons`|
|M3 no-hint / unavailable throttles invisible|`CooldownState::{Until, Unknown, Unavailable}`; `throttle_without_hint` recorded; advice beyond the permit maximum reads as unavailable; success clears an open-ended throttle|`no_hint_and_unavailable_throttles_are_visible_states`|
|M4 oldest-entry eviction miscounted|counts are exact over every live attempt; the view is a bounded sample of the oldest with a `hidden` count; `cancelled`/`abandoned` count whenever the attempt existed|`counts_stay_exact_beyond_the_bounded_sample`|
|L5 dropped permit conflated with cancelled wait|`abandoned` counter (authority keeps that occupancy uncertain)|`dropped_permit_is_abandonment_and_finish_is_completion`|
|L6 truncation unit/test|byte-bounded (200) at a char boundary, tested with multibyte input|`refusal_reason_is_recorded_per_group_and_byte_bounded`|
|L7 sleeps as oracles|contract and BDD poll until the attempt is waiting; elapsed asserted strictly increasing|contract + `inference_admission_observation_steps.rs`|
|L8 trivial/dead assertions, untested claims|removed; two-group, unavailable and clamp cases added|—|
|I9 steps file over 750 lines|P4 steps moved to `inference_admission_observation_steps.rs`|—|

## Mutations (slice 1, after review fixes)
    === MUTANT Q1 cooldown anchored at feedback time [cooldown_is_anchored]
      test infrastructure::admission::observed_gate::tests::cooldown_is_anchored_at_the_grant_and_kept_per_group ... FAILED
    === MUTANT Q2 excessive advice not unavailable [no_hint_and_unavailable]
      test infrastructure::admission::observed_gate::tests::no_hint_and_unavailable_throttles_are_visible_states ... FAILED
    === MUTANT Q3 no-hint throttle invisible [no_hint_and_unavailable]
      test infrastructure::admission::observed_gate::tests::no_hint_and_unavailable_throttles_are_visible_states ... FAILED
    === MUTANT Q5 hidden count dropped [counts_stay_exact]
      test infrastructure::admission::observed_gate::tests::counts_stay_exact_beyond_the_bounded_sample ... FAILED
    === MUTANT Q4b dropped permit counted as cancelled: killed (dropped_permit_is_abandonment_and_finish_is_completion)
    === MUTANT Q6b refusal reason unbounded: killed (refusal_reason_is_recorded_per_group_and_byte_bounded)
    === MUTANT Q7b refusal recorded on the wrong group: killed (same)

Earlier round (O1–O6): dropped wait stays live, unbounded view, elapsed not reported, process gates unobserved all killed; O2/O5 were dead-code compile errors superseded by Q1–Q7.

## Adversarial review 2 (2026-09-09) and fixes

Verified all nine first-round fixes closed; eight new items, all fixed test-first:

|Finding|Fix|Proof|
|---|---|---|
|F1 medium: receipt-based clamp could pin a group `Unavailable` locally while the authority kept granting|advice judged by the delay remaining when it arrives (offset minus time since grant), the authority's measure|`cooldown_is_anchored_at_the_grant_and_kept_per_group` ("max + 10 ms" 40 ms into the grant reads as `Until`)|
|F2 completion throttle unclamped|`finish(Throttle)` beyond the permit maximum reads as `Unavailable`|`completion_throttle_is_clamped_and_expired_cooldowns_read_as_none`|
|F3 racy anchor assertion|exact `since_ms + offset` from the observed phase, in unit and contract tests|same tests|
|F4 inner forwarding unproven|recording inner permit asserts every advice/no-hint/finish reached it; contract checks the authority's cooldown and released slot|unit fake `Forwarded`; contract `view_when_authority`|
|F5 `Unavailable` stickiness|documented: clears only on re-negotiation after operator reset, like the authority|doc comment|
|F6 lossy no-hint over a dated cooldown|documented limitation (never claims a cooldown the authority lacks)|doc comment + `no-hint keeps an unexpired dated cooldown` assertion|
|F7 untested branches|max-merge with shorter/longer advice, expired cooldown reads `None`, no-hint over `Until`/`Unavailable`, failure keeps state|unit tests|
|F8 steps file at the limit|left at 748 lines; slice 2 adds no authority steps (its steps live in the observation module)|—|

## Mutations (slice 1, after review 2)
    S1 clamp ignores elapsed transport time: killed (cooldown_is_anchored_at_the_grant_and_kept_per_group)
    S2 completion throttle unclamped: killed (completion_throttle_is_clamped_and_expired_cooldowns_read_as_none)
    S3 feedback not forwarded to the inner permit: killed (cooldown_is_anchored...)
    S4 finish not forwarded: killed (no_hint_and_unavailable_throttles_are_visible_states)
    S5 no-hint not forwarded: killed (same)
    S6 shorter advice shortens the cooldown: killed (cooldown_is_anchored...)

Process note: mutation runs restore src/ with git checkout, which also reverts uncommitted test sidecars; tests are committed before mutating.
