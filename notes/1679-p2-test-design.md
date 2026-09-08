# P2 test-design working draft

Baseline at c3b597e0: architecture 46, contracts 99; P0 characterization 4,
transport fixtures 2 pass. Commands: cargo test -p quecto-agentic-harness --test
contracts --test architecture; cargo test -p quecto-agentic-harness --test
inference_admission_characterization --test inference_admission_transport.
Logs: /tmp/p2-baseline.log, /tmp/p2-fixtures.log.

Tests will exercise real leaf HTTP adapters through all public provider surfaces
with loopback servers and deterministic barriers, no paid requests. Table rows
cover OpenAI Chat Completions, Responses/Codex and Anthropic; default method
fallback on Codex must charge once. Existing disabled fixtures remain unchanged.
A contract fake admission gate records acquire/feedback/ack order and keeps
uncertain attempts active; a composed local adapter shares P1 service ports.

Key transport tests: receiver created before grant; queued receiver dropped before
grant has zero sends; raw sends equal acquisitions for retries/refresh; stream
receiver return, bounded consumer stall, and abort-with-transport-still-owned do
not release early; confirmed transport destruction eventually releases; queue and
active deadlines yield explicit failure without replay. Barrier server proves
nonterminal 429 headers apply cooldown before delayed error body completion.

Deterministic policy contracts add feedback-at-receipt, exact max deadline merge,
duplicate report idempotency/conflict, active occupancy retained until completion,
unavailable sentinel, and terminal duplicate not reapplying feedback. Existing
completion feedback remains compatible. Shared fallback count is group-owned,
resets on confirmed success only, exponential ceiling and injected jitter tested
at zero/max/overflow. No independent per-leaf fallback counter.

Parser table: seconds/ms/date, case/repeated headers and maximum across hints,
invalid/absent/negative, zero/past, huge integer and checked deadline overflow,
90s vs 30s retry ceiling; only typed throttle statuses/events enter fallback.
Preserve old public error strings and classification. SSE overloaded/rate-limit
errors apply feedback from parsed event, never error-prose matching.

Runtime contracts: explicit endpoint/account aliases not display models, same
context and mapping retained across credential refresh/model change; unknown alias
fails before send; attempted mapping/policy changes fail retaining current runtime.
Router introspection and borrowed request forwarding unchanged.

Every new assertion/Then receives recorded RED or deliberate localized mutation
with restored GREEN per ADR0007. Missing-API compilation is initial RED only,
not sufficient behavioral evidence. Production disabled behavior retains all P0
fixtures. BDD uses existing @inference-admission conventions and test-support.

Fresh-review corrections are in the semantic matrix. Terminal billing429 coverage
must assert no retry and no persisted throttle fallback; HTTP headers alone are
insufficient for body-dependent terminal classification. Reload cases include
active + cooldown, changed C/interval/map, disable attempt, and accepted no-op.

Constraint: Codex leaf source is already 750 lines, so move cohesive transport
implementation to a sibling module rather than exceed gate or suppress lints.
OpenAI assembled stream currently spawns an inner pump; enabled path must place
permit inside that pump or explicitly await its shutdown, not release from the
outer AbortOnDrop guard. Incremental pump must select receiver closure/cancel
while blocked on HTTP/body reads as well as bounded channel sends. Disabled
P0 detached-pump characterization must remain unchanged.

Concrete receipt contract draft: tests/contracts/admission_feedback.rs (not yet
registered until review): spare-capacity blocking without release, completion not
reanchoring, idempotent report/conflict, shorter-max merge and unavailable retaining
occupancy. Proposed report sequence is attempt-local, monotonic and bounded (retain
latest report; reject older replay), not an unbounded feedback history.

Planned named transport cases (BDD-style Rust plus tagged scenarios):
- `all_leaf_surfaces_charge_one_raw_attempt` (AC3/4; 3 leaves × 3 surfaces).
- `receiver_return_does_not_release` and `cancel_queued_never_sends` (AC4).
- `dropped_receiver_acknowledges_owned_transport_shutdown` (AC4).
- `slow_consumer_retains_transport_permit` (AC4; >64 deltas).
- `retry_and_refresh_reacquire_after_completion` (AC3/4; exact sends + sleep hook).
- `header_feedback_blocks_sibling_before_error_body_ends` (AC5; valid 90s).
- `sse_throttle_after_output_never_replays` (AC5; typed event).
- `provider_rebuild_retains_alias_budget_and_rejects_policy_reload` (AC3).

Negative absence assertions use executor-controlled gate/clock barriers, not wall
sleep as oracle. HTTP server start/EOF oneshots record physical ownership; external
TCP EOF is corroboration only, never a claim of remote computation cancellation.
All spawned test tasks use RAII cleanup and bounded safety timeouts. Enabled test
adapter is explicitly phase-local; it cannot be selected as shared-host activation.

Test review ef6c5a8b accepted: exact expected grant/raw-send counts (success and
fallback=1, auth resend=2, retry exhaustion=configured attempt count, terminal=1),
not merely equality. Enqueue is not a grant. No receiver-before-grant requirement:
observe cancellation/lifetime without imposing method timing. Parser tests add
reversed/repeated/mixed-case hints, invalid+valid orders, excessive last, exact
ms/date boundaries and arithmetic overflow independent of policy rejection.
Receipt tests add queued/unknown/terminal feedback rejection or idempotence,
separate sibling overlap and success not shortening, unavailable dispatch denial
before/after completion with unrelated group progress. Duplicate fallback tests
must observe the NEXT fallback duration so double count cannot hide behind max.
Port contract registration must reuse real leaf transport contracts for
AttemptAdmission/AttemptPermit. A fake-only recording permit test would merely
prove its own Vec push implementation and is not acceptable contract evidence.

RED seam findings to carry into GREEN design:
- Receipt feedback needs shared authority receipt clock and policy bound, not
  per-adapter epoch or SystemTime::now minus private start. Wall time is captured
  only to normalize dates; authority monotonic time drives all dispatch.
- Active permit requires cancellable deadline signal; bare future drop must
  destroy owned request/response before acknowledged finish. Scope cancellation
  cannot be represented solely by polling the user CancelFlag every read.
- Fallback arithmetic lives in domain; one group counter/jitter source accessed
  at typed confirmation, not each leaf deriving independent fallback_ms.
- Alternate runtime ingress keeps optional production activation out of existing
  AgentRuntimeInputs; every refresh closure retains explicit bound capability.

Contract discovery: AttemptAdmission registration includes real leaf integration
suite and AttemptPermit includes real HTTP/SSE feedback suite; lib mirrors both.
Avoid adding allowlist escape or fake-only contract to satisfy architecture scan.

GREEN integration dependency ordering (not implementation yet):
1. Domain nonterminal report/idempotency/max-unavailable, shared fallback arithmetic.
2. Inward attempt context/clock/deadline; phase-local public-port adapter owns
   queued request cancellation and group fallback. No production shared-host switch.
3. Shared transport primitive returns owned response+permit; ensures feedback at
   receipt and destruction before acknowledged finish; all leaves use per-send.
4. Preserve disabled paths, propagate guard into actual SSE pump including nested
   assembled stream task and all selected cancellation/backpressure points.
5. Alternate runtime composition stable explicit binding, reject changed snapshot;
   existing routes/refresh/retry untouched owners.
6. All RED suites GREEN, then real localized algorithm mutations for ownership,
   feedback timing, reload reset, retry charging, exact clock boundaries.

Receipt report sequence fencing retains latest report fingerprint bounded per
attempt; rejects older report with Replay, conflicting same sequence Conflict,
accepts exact duplicate even retained terminal without reapplying. Historical
report receipt cannot reanchor cooldown on duplicate or final completion.
Local authority sketch preserved as text only: needs asynchronous scheduling,
clock/jitter ports and targeted cancellation/drop tests before activation. Do not
register sketch as production: polling fairness and fallback hardcoded randomness
are unverified. P2 phase-local public-port fixtures already compose actual service;
production remains disabled pending P3 shared authority.
GREEN boundary integration correction: infrastructure references capabilities only
through application::ports reexports (not direct application module imports), as
existing architecture test requires. No exceptions added.
GREEN helper parser supports strict IMF-fixdate (RFC preferred wire form), validates
weekday/calendar and rounds fractional milliseconds upward. Obsolete HTTP dates
currently treated invalid/fallback; assess during adversarial review against
canonical validated HTTP-date requirement before accepting this restriction.
GREEN leaf audit counterexample: opaque/plain529 or429 noheader still typed HTTP
status throttle candidate. Requiring structured body rate_limit type would drop
normal HTTP throttle fallback. Exclude known terminal billing/auth/client codes,
otherwise status candidate confirms nohint at body receipt. SSE200 requires typed
event classification. Add direct wire test before accepting implementation.
GREEN audit: assembled Responses/Codex success path must not buffer body then
inspect SSE advice at EOF. It is a live stream even for chat(); typed error event
must postpone siblings before stalled body EOF. Reuse owned streaming pump and
collector. Terminal error delivery after run releases permit can await downstream
capacity without holding transport; do not lose compatibility with try_send full.
Review fix23 parity tests added disabled characterization and fixture checks; those
passing assertions need own oracle sensitivity/explicit shared coverage mapping,
not merely parityRED. Tests owner requested full inventory before review closure.
