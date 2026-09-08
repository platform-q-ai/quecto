# #1679 P2 attempt transport RED handoff

## Files and command

Owned executable files:
- `quecto-agentic-harness/tests/inference_admission_attempts.rs`
- `quecto-agentic-harness/tests/common/admission_attempt_fixture.rs`

Parent-requested report: this file. No production implementation by this subagent.

```
cargo test -p quecto-agentic-harness --test inference_admission_attempts
```

Latest run: **0 passed, 56 failed, 0 ignored**, test execution 0.05s;
`/tmp/1679-attempt-final-red.log`. Earlier detailed run:
`/tmp/1679-attempt-behavior-red.log` (`-- --nocapture`, 0.04s).
Initial missing-import failure `/tmp/1679-attempt-red.log` is superseded and is
**not** behavioral evidence. Parent supplied no-op builder and inward traits.
Fixed fixture Command Debug compile errors and provided a real system message
for Codex OAuth validation. All success wire payloads now reach the intended
admission oracle, including the OAuth-specific Responses endpoint.

## Minimal proposed API (already scaffolded by parent)

`application::inference_attempt::{AttemptAdmission, AttemptPermit}`:
- `AttemptAdmission: Debug + Send + Sync`: `acquire(&self)` returns a boxed Send
  future producing `Result<Box<dyn AttemptPermit>, DomainError>`.
- `AttemptPermit: Debug + Send`: nonterminal `feedback(&mut self,
  ThrottleFeedback)` and consuming `finish(self: Box<Self>, Feedback)`.
- Three concrete leaves expose consuming builder
  `with_attempt_admission(Arc<dyn AttemptAdmission>) -> Self`.

The object passed to the builder is already bound to a trusted scope and explicit
endpoint/account alias. Acquire belongs immediately before each physical send.
Finish asserts owned local transport destruction; permit Drop without finish
retains uncertain occupancy. Never finish merely because a receiver was returned,
dropped, or a JoinHandle abort was requested. Deadline cancellation and receipt
clock extension are intentionally left to the parent, not invented in this fake.

## Cases: 14 independently named tests × 4 leaves/auth modes

OpenAI Chat Completions, Responses API-key, Codex OAuth, Anthropic:
1. chat exact grant/start count;
2. assembled-stream exact count (Codex delegates to chat);
3. incremental exact count;
4. returned receiver keeps C=1 occupancy and sibling queued;
5. 130 ordered deltas through fixed-capacity 64 channel;
6–8. queued cancellation on all three surfaces;
9. cancellation synchronously raced by gate at grant; acknowledge no-send;
10–12. receiver drop while blocked on headers/body/full channel;
13. abort chat while headers blocked;
14. abort assembled stream while body blocked (OpenAI inner-pump hazard).

Fixture records queue/grant, physical HTTP starts, acknowledgements, uncertain
permit abandonment and independently observed TCP EOF. Negative send assertions
use gate queue-registration barriers, not sleeps. Waiting for a gate event also
fails immediately if the server has observed an ungranted request. All cases are
separate Rust tests; one failure does not hide other cases. Sibling receiver
acquisition runs as an owned task, so tests do not impose receiver-before-grant
method timing. Test server is loopback-only, proxy disabled, bounded headers/body,
JoinSet cleanup with RAII abort and five-second safety bounds. No paid requests.
TCP EOF corroborates local shutdown only, not remote provider computation.

## Actual evidence versus currently masked assertions

|Case family|Observed RED with no-op builder|Later assertions NOT yet evidenced|
|---|---|---|
|1–3 (12 tests)|Expected grants=1, actual=0 after successful actual raw HTTP response|Grant/start relative order with real grant, duplicate-grant/start rejection, successful permit finish/active=0|
|4 (4)|Raw HTTP start without preceding grant|Receiver-return occupancy; exact queued sibling event trace; EOF releases; release precedes sibling grant|
|5 (4)|Full 64-slot channel active=0 instead of 1|Retained permit on blocked bounded send; all 130 ordered deltas, assembled content, no premature finish and terminal release|
|6–8 (12)|Raw HTTP start without preceding grant while fake gate is held|Queued acquire future cancelled; zero grants/starts after gate opens; no orphan waiter|
|9 (4)|Request succeeds instead of cancellation error (gate was never entered)|Granted-no-send finish exactly once; grant remains in charged history; no raw send|
|10–12 (12)|Raw HTTP start without preceding grant|Receiver Drop does not synchronously release; independent TCP EOF; owned task finish; sibling progress only after completion; no abandoned permit|
|13–14 (8)|Raw HTTP start without preceding grant|Abort intent does not release; JoinError cancelled; physical EOF; owner finish and replacement progress|

**Do not mark the masked assertions RED/mutation-proven or GREEN.** The suite is
behaviorally RED for missing leaf admission only. Gate active count alone is not
independent transport-termination evidence. TCP EOF is checked separately after
cancellation, but this does not impose a remote-event-before-local-finish ordering:
local destruction may be acknowledged before the peer is scheduled to see EOF.

## Independent falsifiability / next bounded GREEN-and-mutation sequence

Once parent connects real leaves, run each named test independently. For each
mutation below run its narrowly named test, save the failing assertion, restore,
and rerun GREEN; do not globally disable the gate, which masks lifetime oracles.

- Move acquisition after send: tests 1–3 must report missing preceding grant.
- Double admission in public stream + delegated leaf: exact grant=1 test for
  Codex assembled stream must fail (bounded timeout also detects deadlock).
- Finish on incremental receiver return: test 4 fails occupied/queued event trace.
- Drop or leak queued acquire on cancellation: tests 6–8 fail cancellation
  transition or exact zero-send history after opening the gate.
- Omit post-grant cancellation check: test 9 fails error/no-start assertion.
- Finish when receiver Drop or abort intent occurs rather than inside owned task:
  tests 10–14 fail current-thread synchronous active=1 assertion or TCP shutdown
  corroboration / retained uncertainty. Do not substitute a gate counter for EOF.
- Remove select-on-closed while blocked in send()/body read/channel send: tests
  10, 11, 12 individually fail bounded EOF/finish, not just another wait mode.
- Release while bounded channel remains full: test 5 fails active=1 before drain.
- Change 64 to 65: test 5 max_capacity assertion; drop/reorder one delta: test 5
  numbered per-delta assertion; truncate assembled output: final Done assertion.
- Lose successful finish: tests 1–3 completion bound/active=0 fail independently
  after confirming acquisition/start oracle. Leak aborted inner OpenAI pump:
  test 14's independent EOF bound fails.

Some stronger requirements are **not claimed covered by this bounded file**:
actual monotonic pacing retention after granted-no-send (fake retains grant history
but has no clock), deadline expiry, retry/auth-refresh attempt counts and sleep
occupancy, typed feedback/header cooldown, partial-output SSE errors, runtime
rebuild aliases, tools/parent waiting. Parent's other test work owns those.
No compatibility mutation evidence is claimed: existing disabled P0 fixtures are
unchanged, and their green runs/mutations must be recorded separately by parent.

## Executed preimplementation oracle perturbations (supersedes plan-only status)

Parent requested equivalent synthetic observation/result perturbations, without
implementing admission. Extracted pure `fixture::oracle` helpers and changed real
transport cases to call them. Six `oracle_counterexamples::*` tests invoke these
**same helpers**, first with valid observations and then counterexamples wrapped
in `catch_unwind`; each counterexample must panic. The liveness case catches the
same bounded future wrapper timing out on a pending future. These helpers do not
acquire permits, mutate policy, send HTTP, or fake enforcement.

Executed:
```
cargo test -p quecto-agentic-harness --test inference_admission_attempts
```
`/tmp/1679-attempt-synthetic-and-real.log`: **6 synthetic tests pass, all 56 actual
production transport tests still fail, 0 ignored**, 5.04s (five seconds is the
single deliberate shared timeout perturbation). Before the fixture-guard addition,
`/tmp/1679-attempt-oracle-checks.log` recorded five synthetic groups passing alone.

### Complete observation-assertion mapping

The prefixes below are all under `oracle_counterexamples::`. Repeated assertions
across four auth/leaf configurations and three public surfaces share the same
helper and the same perturbation; this does not claim independent production
execution of each masked observation.

|Real-test assertion / shared helper|Executed counterexample and synthetic test|
|---|---|
|`Gate::assert_exact` → `exact`: exact grant count=1/2/0|Missing/double/unexpected grant; `counts_order_identity_and_uncertain_drop_are_falsifiable`|
|`exact`: exact raw starts=1/2/0|Missing/double/unexpected raw start; same test|
|`exact`: grant and start IDs, preceding order|Wrong grant ID, wrong send ID, grant after start; same test|
|`exact`: no uncertain abandonment|Valid grant/start plus Abandoned; same test|
|`Gate::wait` → `granted_before_send`: no observed ungranted send|Raw start alone; grant appears after raw start; same test|
|All active=1 assertions: receiver return, full channel, drained deltas, receiver Drop, abort intent, sibling queued|Active=0 for each labeled release timing; `lifetime_backpressure_and_replacement_traces_are_falsifiable`|
|All active=0 completion/cancellation assertions|Leaked active=1; same test|
|Receiver-return exact lifecycle trace|Appended premature Finished; same test|
|No Finished while bounded send blocked|Finished appended to otherwise valid held trace; same test|
|No sibling Granted before owner termination|Premature Granted(1); same test|
|Finish(0) strictly before Granted(1)|Reversed order, missing finish, missing replacement; same test|
|Channel maximum=64 and full remaining capacity=0|Maximum=65, remaining capacity=1; same test|
|Queued cancellation exact trace|Missing QueueCancelled, cancelled queue later granted/sent; `cancellation_ack_and_peer_eof_are_falsifiable`|
|Granted-no-send exact charged/ack trace|Queue cancellation substituted/refund, omitted finish, extra raw send, duplicate finish; same test|
|Cancelled request returns error / aborted join has cancellation status|False result for both labeled observations; same test|
|Required event observations via `wait_event` → `present`|Missing Queued(0), Queued(1), QueueCancelled(0), Finished(0), Finished(1), PeerEof(0), independently; same test|
|Independent server-side EOF read equals zero|Read returns one byte; same test (pending read separately bounded below)|
|Successful chat/assembled/sibling content equals `0,`|Empty/wrong content; `stream_content_terminal_and_closure_are_falsifiable`|
|First delta and all 130 ordered deltas|Wrong numbered delta at **each index 0–129**, missing delta, Done substituted for delta; same test|
|Terminal Done exists and assembled full 130-delta content agrees|Missing Done, delta instead of Done, truncated Done content; same test|
|Receiver closes after Done|Extra delta or extra Done; same test|
|Sibling stream emits no Error|StreamEvent::Error; same test|
|`invoke` receives exactly one Done|Prior Done already present; same test|
|`invoke` requires terminal completion|None instead of terminal result; same test|
|`invoke` incremental and assembled content agree|Truncated assembled text; same test|
|Bounded wait/recv/join/server EOF/completion safety failure|Same `bounded` wrapper called with pending future, catches deadline panic; `bounded_liveness_oracle_rejects_nonterminating_observation`|
|Fixture actual HTTP method=POST and strict header/body safety bounds|GET; length equal to each exclusive maximum (32768 and 1048576); `fixture_protocol_safeguards_are_falsifiable`|

Ordinary fixture setup/I/O `unwrap`/`expect` paths (bind, JSON parse, socket writes,
oneshot channel send, successful task join) are infrastructure fail-fast guards,
not separate AC Then observations or an admission policy. Their nontermination
safety is shared `bounded`; this report does not invent synthetic admission
coverage for operating-system setup failure.

**Evidence classification:** all semantic assertion helpers are now demonstrated
falsifiable before implementation using explicit counterexample observations.
The later observations remain masked in the *actual production* paths, as the
previous table states. Synthetic passing tests must not be reported as provider
GREEN, actual transport lifetime proof, or sufficient post-GREEN algorithm
mutation evidence. The previously listed real implementation mutations are still
required after production GREEN; no production algorithm was changed here.

## Minimal deadline addition (parent integration required)

Added two separately executable cases:
- `queue_deadline_rejection_never_grants_or_sends`: held acquire is explicitly
  expired by the authority fixture and returns `Err(DomainError)`; it must fail the
  call, record QueueExpired distinct from caller cancellation, and never grant or
  send, even after the gate reopens.
- `active_deadline_requires_owned_transport_shutdown_ack`: authority expiry while
  OpenAI incremental transport is blocked reading its body, with receiver kept
  alive. Expiry intent retains occupancy; owned transport must close (independent
  peer EOF), emit explicit stream Error, close receiver and acknowledge finish
  exactly once. No caller cancellation flag or receiver Drop triggers it.

Necessary inward addition to `AttemptPermit`:
```
fn deadline_expired(&self)
    -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
```
The future is owned/static so obtaining it does not borrow the permit across
nonterminal feedback or consuming finish. It resolves when the authority requires
active-attempt termination; provider transport owners select it while blocked in
send/body/pump operations. Ready is cancellation intent, never completion. A
backwards-compatible default may be `Box::pin(std::future::pending())`. The test
fake overrides it with an explicit authority event/Notify signal; no wall-clock
policy implementation lives in the fixture.

Added `oracle_counterexamples::deadline_rejection_and_shutdown_observations_are_falsifiable`
using the same shared oracles: substitutes caller-cancel for queue expiry, adds
illegal grant/send, omits active expiry, releases/finishes on signal, and replaces
stream Error with EOF/Done. Existing count, closure, EOF, completion, active=0 and
bounded-liveness perturbations cover repeated deadline assertions.

Latest compile attempt `/tmp/1679-attempt-deadline-api-red.log` fails E0407 because
parent port does not yet have `deadline_expired`. This is only initial API RED;
**new deadline behavior and new perturbation test have not yet run**. Parent can
add the default pending method above and rerun to recover executable no-op-scaffold
RED. Previous executed 56-production/6-synthetic evidence remains unchanged; do
not claim these two cases executed until this additive API is integrated.

Parent integrated pending-default deadline capability and reran full target:
/tmp/p2-attempt-deadline-red.log: 58 actual behavior RED, 7 synthetic oracle tests
pass (including new deadline counterexamples). Actual deadline cases fail at
ungranted raw send, not timeouts. Suite5s includes deliberate safety-timeout oracle.
No claim GREEN; postimplementation algorithm mutations remain required.
