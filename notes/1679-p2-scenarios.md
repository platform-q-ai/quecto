# P2 concrete scenario/assertion inventory

Each ID prefixes evidence entries; all rows need individual assertion RED/mutation
and restored GREEN. No row is currently claimed implemented.

|ID / AC|Given / When / Then and coverage|Decisive oracle / killing mutation|
|---|---|---|
|P2-01 AC3/4|Each OpenAI/Responses/Anthropic leaf × chat/stream/incremental; one success|exactly 1 grant preceding exactly 1 raw HTTP start; acquire after send or double both fails|
|P2-02 AC4|C=1 active stream, sibling queued; return receiver|sibling no dispatch at controlled executor barrier, active=1; release-on-return fails|
|P2-03 AC4|queued attempt; cancel before grant; separately cancel racing granted permit|no raw send, queued terminal or granted no-send acknowledgment; pacing retained if granted|
|P2-04 AC4|abort during blocked headers/body/full-channel send|transport/task independent destructor ack; sibling blocked until ack then dispatch; move release to outer guard fails|
|P2-05 AC4|slow consumer and >64 deltas, no EOF yet|complete ordered output without unbounded forwarding; occupied until owned pump termination|
|P2-06 AC3/4|429 retry, refresh401 resend, terminal billing/auth/client and Codex default delegation|configured retry count, refresh=2, terminal/fallback=1 exact starts; grant-before-send per attempt; sleep observes active=0; omit per-retry charge fails|
|P2-07 AC5|C=2 valid90s 429 headers, stalled body; sibling group G and G2 request|G blocked at 89999, admitted at90000; original active until ack; G2 progresses; apply feedback only at completion or cap30s fails|
|P2-08 AC5|HTTP200 typed SSE throttle before/after text per leaf; terminal typed code|feedback at parsed event; pretext retry existing bounded owner only, posttext no replay; terminal no fallback/retry|
|P2-09 AC5|seconds/ms/date and malformed/excessive/duplicate headers, wall clock jump after receipt|absolute deadline exact, max independent of order, excessive unavailable, no wall reread; parser tables|
|P2-10 AC5|two active siblings overlap hints, duplicate reports, success, unavailable|max retained; duplicate fallback next duration unchanged; queued/terminal reports cannot mutate; unavailable blocks even after completion; G2 progresses|
|P2-11 AC3|shared/distinct endpoint-account aliases, credential/model rebuild; active+cooldown while C/map/pacing/disable reload|same budget retained, unknown alias before send, rejected reload retains runtime; noop accepted; new-context-on-refresh fails|
|P2-12 AC4|C=1 saturated inference, tool or idle parent delegates|tool barrier/child completion independent of parent slot; gate around tools/wait fails|
|P2-13 compatibility|disabled mode and existing router introspection/borrowed slices/errors|P0 fixtures, existing provider/runtime/retry/architecture contracts unchanged|

Review 3c16e183 accepted: fake occupancy is not independent termination evidence.
Tests must tie transport-owned task drop/ack (not receiver drop) to release,
correlate each start with preceding grant, distinguish queued cancellation from
granted-no-send completion, and prove retained pacing. TCP EOF corroborates local
termination only; no claim about remote provider computation. Existing transport
helpers provide bounded cleanup; no wall sleeps used as negative behavior oracle.
