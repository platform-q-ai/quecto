# P2 semantic risk matrix (draft)

|Invariant|Dimensions and interactions|Observable outcome / evidence|
|---|---|---|
|AC3 identity|disabled/enabled; known/unknown alias; shared/distinct endpoint-account mapping; changed model, credentials, refreshed adapter and runtime rebuild|disabled wire unchanged; unknown fails before send; same alias keeps charges across rebuild; fake HTTP raw start count and public runtime contracts|
|AC3 pacing|first/retry/auth resend; failure vs success; queue cancellation racing grant|one charge per actual send, no refund on failure; cancelled queued request never sends; fake clock + HTTP count|
|AC4 ownership|chat/chat_stream/incremental; default delegation; EOF/error/receiver drop; slow bounded consumer; future/task cancellation during acquire/send/pump|one permit per leaf send, receiver return does not release, confirmed local transport shutdown precedes release; barriers and active-count fixture|
|AC4 retries|429/529/auth terminal/refresh resend; partial stream output; sleep and tools/parent wait|no new retry owner/replay, permit released before existing retry sleep; C=1 child/tool progress; existing retry suites + exact attempt assertions|
|AC5 feedback|seconds/ms/date; absent/invalid/negative/zero/past/excessive/overflow; wall jump; overlapping shorter/longer hints; success and repeated throttles|typed feedback at receipt; max monotonic merge; fallback exponential injected jitter; excessive unavailable, 90s not capped to 30s; unrelated group progresses; deterministic helper/service tests and sibling attempt barriers|
|compatibility|router name/as_any; borrowed requests; bounded streams and unchanged error classification/text|existing characterization and contract suites plus adapter tests|

P3 process persistence, broker disconnect and container negotiation are not P2
implementation cases; P4 UI/freshness also deferred. In-process tests make no
cross-process enforcement claim. Production activation remains disabled.

Implementation hazard discovered during seam inspection: P1 `complete(Feedback)`
merges only at completion, but P2 feedback must postpone siblings at HTTP/SSE
receipt while a response body may still be active/backpressured. Test a stalled
429 body and spare capacity: siblings must already be blocked without releasing
the original permit. Add a non-terminal feedback transition owned inward;
completion must not re-anchor a stored relative delay or double-count throttles.

Planned adapter seam: an inward-owned asynchronous attempt gate returns an owned
permit with nonterminal feedback and acknowledged-finish operations. Leaf adapters
acquire just before `send`, not at LlmProvider routing. Cancellation is scoped to
owned transport futures/tasks; drop of a permit without confirmed shutdown must
not grant replacement capacity. Bounded queued cancellation and active deadline
handling must survive future drop. A locally serialized adapter can use P1 ports
without importing the concrete service; interface/tests compose that service.
Runtime inputs retain a stable admission context and explicit alias map through
factory rebuild closures. Unknown mapping rejects composition before dispatch.

Counterexample review 901bb709 accepted corrections:
- AC3 restart-only configuration: test active occupancy AND shared cooldown while
  attempting changed C/interval/group map or enabled→disabled; reject visibly,
  retain effective mapping/context/charges/runtime. A failed reload must not hand
  existing outstanding attempts to a fresh budget. Also test no-op reload succeeds.
- AC5 terminal billing/client classification: HTTP 429 is not sufficient when a
  typed provider billing code (e.g. insufficient_quota) makes it terminal. Such
  errors never enter throttle fallback or retry; maintain external error
  text/classification. Previously accepted valid header cooldown is retained
  (see clarification below); terminal body classification adds no fallback.
- AC5 HTTP-200 SSE errors: test parsed overload/rate-limit events both before and
  after emitted text, including no-hint fallback. Shared feedback applies at the
  parsed event while transport still owns the permit; no replay after partial
  output. Test typed terminal billing/auth/client events do not throttle.

Clarification of terminal-body interaction (avoid an invented AC): canonical AC5
forbids retry of billing/auth/client errors; it does not require retracting an
already validated 429 Retry-After at headers when a later body proves billing.
A header-established cooldown is never shortened. At body classification,
terminal errors add no fallback/increment and never retry. Tests distinguish
already accepted header advice from fallback based on complete typed rejection.
For absent/invalid hints, wait for typed rejection receipt before fallback. This
keeps max-monotonic semantics and does not invent reversible provisional cooldown.
The stalled-body immediate-cooldown test supplies a valid Retry-After header.

Fresh verifier 51c26d6a: clean against canonical P2/AC5 and ADR0026 with the
clarification superseding earlier overstrong terminal cooldown wording; no
remaining high/medium semantic gap. Matrix frozen as test-design input.

RED fixture review correction: no sleep-based negative HTTP proof accepted. Group
feedback fixture now synchronizes sibling parked-at-acquire or actual raw POST;
then snapshots raw sends. Deadlines in test helpers remain bounded cleanup only.
Actual phase authority clock integration is needed to make90s boundary test GREEN;
fixture explicit reopening is only receipt distribution/group-isolation evidence.

Local review correction: fallback base is explicit group policy input, not a
hardcoded1000ms implementation default. Validate positive<=maximum, preserve it
through runtime reconstruction and reject changed base on live reload. Test
nondefault10s base120s maximum plus invalid0/greater-than-max; pair base change
with active occupancy and established cooldown. Public config migration updates
all literals; no silently defaulted operator setting.
Local semantic correction: P2-08 explicitly includes OpenAI chat-completions SSE
error envelope before/after text (no successful Done), and Responses top-level
{type:error,code} as well as response.failed nested errors. Billing/auth/client
root codes are terminal; arbitrary successful delta text/code must not throttle.
Local removed-behavior correction: cross-product vendor × surface × send failure/
truncated error body/truncated body after SSE terminal/unterminated final line/
oversized error body. Enabled errors and classifier must match disabled surface;
incremental4KiB displayed limit retained, assembled EOF validation retained while
receipt feedback remains immediate. Cancel/deadline internal outcome typed.
Observer semantic terminal boundary: HTTP bytes after provider terminal remain read
for assembled compatibility but cannot produce advice/errors ignored by canonical
parser. Cross valid terminal +trailing throttle +HTTP read failure; ignored event
name +throttle-looking JSON. Match provider event dispatch, not arbitrary JSON.
Responses unknown event type containing error object is ignored, unlike OpenAI
chat envelope. Test unknown extension→normal completed and unknown extension→real
throttle (observer must neither report early nor stop and miss real feedback).
