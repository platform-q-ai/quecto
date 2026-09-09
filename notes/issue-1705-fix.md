# Issue 1705 bugfix evidence

## Acceptance checklist
- Real connect-time slim projections accepted, including control receipts, suspension/failure counts, typed admission and workflow variants.
- Shared explicit contract, strict affirmative field/shape allowlists; malformed/unknown data rejected.
- Preserve cursor finalization, correlation and nested-target isolation.
- Busy real accept loop plus actual consumer returns without serialized dispatch; idle correlated response remains supported.
- Public get_state has short whole-operation deadline across connect/write/read; long commands retain their semantics.

## Reproduction plan
Actual accept loop test holds the command receiver unconsumed and calls the real registry transport with a 250ms bound. The existing producer always emits two fields excluded by the consumer, so the correct assertion should fail with a timeout before the schema fix. Independent producer/consumer matrix tests and inspection stalled-phase tests accompany this.

Historical incident causation remains unverified; these tests establish the source contract defect and deadline behavior, not the cause of past hangs.

## RED
`cargo test -p quecto-agentic-harness --lib busy_inspection_accepts_production_projection_without_dispatch -- --nocapture`: FAILED (0 passed, 1 failed), `real busy snapshot must answer without dispatch: Tool("subagent response timed out (0s)")`, 0.25s. Raw output: `/tmp/1705-busy-red.log`. Test uses actual accept loop and registry transport; removing schema compatibility restores this failure without changing fixtures.

## GREEN and same-class sweep
Real busy accept-loop regression now passes (1/1, immediate). Snapshot validator suite 6/6; registry suite 66/66; projection suite 12/12; accept-loop suite 6/6. BDD agent_cmd feature: 74 scenarios / 330 steps pass using real production projections in the busy mock instead of obsolete hand-built slim JSON.

Sweep: searched `automaticTurnsSuspended`, `allowed_top_level`, `INSPECTOR_RESPONSE_TIMEOUT`, and all `send_subagent_uds_command_with_timeout` callers. Single slim-state acceptance predicate now consumes the same typed contract as production projection. Registry and BDD busy fixtures updated; older minimal fixtures retained to characterize backwards-compatible slim senders. Explicit bounded sender callers (forwarding, spawn, swarm control) gain the same connect/write/read bound; default long sender remains independently 300-second response-budgeted. No additional in-scope duplicate validator identified.

Additional RED evidence: actual projection validator test failed on the ordinary emitted suspension/failure fields (`/tmp/1705-schema-red.log`). Public get_state and stalled-write tests both exceeded their external watchdogs before the deadline fix: 0 passed / 2 failed in 7 seconds (`/tmp/1705-timeout-red.log`). After fix, unavailable connection, invalid/silent responses, blocked write, Linux saturated accept queue, and injected pending connection cancellation are covered. The injected connector traverses the same production transport/deadline wrapper, with drop assertions proving cancellation; kernel backlog behavior may instead fail immediately depending on platform.

Full harness library suite: 4306 passed. Strict all-targets clippy with test-support passed. Hooks verified installed and active. Shared finite serde structs with unknown-field rejection are affirmative schemas, not key denylists. Schema defaults preserve legacy optional omissions while rejecting malformed present values.

Residual scope: the historical aborted calls are not reproduced or causally attributed. Tokio deadlines bound asynchronous waits, not arbitrary synchronous executor starvation. Optional malformed workflow source metadata is omitted when it cannot form the typed slim workflow; valid workflow behavior is unchanged. No automatic merge.

## Refactor and review
Extracted registry transport into `subagent_transport.rs` to retain repository's 750-line source limit while keeping public reexports stable (registry now 556 lines). Final full library run: 4306/4306; final BDD run: 74/74 scenarios, 330/330 steps. Independent read-only adversarial review found no actionable bugs after checking issue acceptance, schema shapes, routing, deadlines and test boundaries. Repository quality, BDD quality/tag checks pass (existing nonblocking BDD warnings remain).

First push gate caught a source-location architecture assertion still pointing at the registry after extraction. Updated its expected consumer path to `subagent_transport.rs`, retaining the shared-frame-reader assertion (not weakening it). All other first-push gates passed.
