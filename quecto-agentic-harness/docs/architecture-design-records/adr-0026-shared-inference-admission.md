# ADR-0026: Single-host shared inference admission

**Status:** Accepted
**Date:** 2026-09-07
**Scope:** #1679 P0 contract; enforcement is not implemented by this record.

## Context

Independent agent processes and container descendants currently retry provider
throttles independently. Limiting delegation conflates live processes with active
inference. A process-local semaphore cannot coordinate independent roots or
container descendants. Existing incremental providers return a receiver before
the HTTP response terminates; receiver drop is not currently an acknowledged
transport shutdown. Existing retry/refresh owners must remain unchanged.

## Decision and ownership

One explicitly started, same-host/same-user admission authority coordinates
configured quota groups. A private owner-only directory contains its endpoint,
singleton lock and durable accounting journal. Lock acquisition precedes socket
binding; losing authority ownership closes admission. No client silently starts a
replacement authority. No mandatory new crate or runtime-manager scheduler.

Domain owns pure queue/pacing/capacity transitions with injected monotonic time.
Application owns attempt lifecycle through narrow client, journal and observation
ports. Infrastructure owns IPC, durable storage, HTTP feedback/transport shutdown,
and runtime scripts. Interface composes and renders. Launch orchestration retains
socket-readiness semantics and only propagates scoped authority context. No live
semaphore or broker state lives in launch transactions or tool execution.

Admission wraps each actual leaf-provider HTTP attempt, below routing, retry and
OAuth refresh ownership, including rebuilt providers. Validation precedes
acquisition where possible. Default streaming delegation cannot acquire twice.
Release/feedback occurs before retry sleep; retry count/backoff ownership stays
where it is today. Partial output is never replayed by admission.

## Configuration and identity contract

Default is disabled and carries no shared-budget guarantee. Enabling requires:

- explicit opaque quota/account alias and provider/model-to-group mapping;
- concurrency `C > 0`, interactive reserve `0 <= R < C`;
- positive finite minimum request interval in milliseconds;
- positive finite queue capacity, queue wait deadline and attempt deadline;
- positive finite fallback cooldown base and maximum accepted cooldown durations,
  with base no greater than maximum;
- configured private authority endpoint and supported protocol capability.

Reject omitted required settings, arithmetic overflow, negative values, unknown
mappings, reserve outside bounds and unusable endpoints before HTTP dispatch.
No numeric value is advertised as a production-safe provider default. Credential
rotation/model changes preserve configured quota identity; credentials and their
hashes never become group IDs. Aliases must themselves be non-secret.

Root interactive identity is issued by the trusted entrypoint; descendant scopes
are registered at the authority under authenticated lineage and capped at
background priority. Normal child config/model/provider overrides cannot replace
or disable inherited authority, regroup quota or self-promote. A root may select
an explicitly configured deployment profile, not infer priority from model names.
Same-UID hostile processes retaining provider credentials and unmanaged account
users are outside this local coordination boundary.

Admission mappings/policy are restart-only. Reload rejects changes visibly,
retains current effective values and reports pending configuration. Operator must
acknowledge drain/quarantine/restart; reload never creates a fresh accounting
budget. Disabled rollback also requires explicit acknowledgement, never an outage
fallback.

## Queue and rate policy

Each agent is FIFO. Eligible work is round-robin across roots and then agents
inside the selected root; nested fan-out does not multiply its root share.
Background cannot consume the `R` reserved slots. Shared slots arbitrate at most
three interactive grants before an eligible background grant. Skipped ineligible
classes do not block work. Progress is bounded in grant opportunities, not wall
time: unbounded streams cannot promise bounded scheduling latency. With `C=1`,
`R=0`; a parent waiting on tools/children holds no inference permit.

Every dispatch consumes one request-start charge, including retries and refresh
resends. Failure/release does not refund pacing; only the active slot is released.
Different groups have independent capacity, queue and cooldown state. Queue-full,
wait timeout, cancellation and authority outage are distinct bounded outcomes.

Provider adapters normalize typed throttle feedback at the HTTP boundary. Policy
does not parse human-readable error prose. Delta seconds and supported millisecond
headers use the receipt monotonic timestamp. HTTP dates use the wall-clock and
monotonic timestamps captured together at receipt: add max(date minus captured
wall time, zero) to captured monotonic time. A later wall-clock change cannot
shorten the resulting deadline. Merge is maximum, never shorten an existing
cooldown. Absent/invalid/negative hints use bounded exponential fallback with
injected jitter: for consecutive throttle count n starting at zero, choose a
duration in [base, min(maximum, base * 2^n)] using saturating arithmetic. Confirmed
success resets the counter; shared cooldown still merges by maximum. Randomness
is an injected input for deterministic tests, not a domain I/O dependency;
valid zero or past dates add no delay beyond existing pacing/cooldown. A valid
hint above the configured maximum or unrepresentable arithmetic makes the group
explicitly unavailable rather than silently clamping provider advice. The maximum
is an operator-required setting, not the existing retry ceiling; 90 seconds is
accepted exactly when it does not exceed that configured maximum. Shared cooldown is
independent of the existing 30-second per-attempt retry delay ceiling.

## Attempt lifecycle and recovery

The bound is locally owned outbound inference transports, not remote computation
that a provider might continue after disconnection. Completion evidence is a
terminal provider response or acknowledged termination of the locally owned
transport/task. Merely returning/dropping a receiver, sending abort, elapsed TTL,
connection loss or process death is not sufficient evidence.

|Transition or race|Required outcome|
|---|---|
|Queued cancel/timeout|Remove queued work; never dispatch it.|
|Grant vs cancel race|Persist grant before response; client acknowledges no dispatch or terminates owned transport. Without acknowledgement retain uncertainty.|
|Dispatch|Durably record possible outstanding work before allowing HTTP start; request charge retained.|
|Streaming|Permit spans receiver creation and bounded forwarding until terminal delivery or acknowledged transport termination.|
|Deadline|Initiate local cancellation; release only after acknowledgement, otherwise uncertain.|
|Terminal rejection/finish|Record terminal result and feedback; persist release before acknowledging completion.|
|Lost grant/complete reply|Idempotent reconciliation; never create a second grant or release another request.|
|Client death/unreachable|Possibly active request becomes uncertain and consumes capacity. No expiry-based free slot.|
|Authority restart|Restore cooldown/charges/outstanding work; outstanding becomes uncertain until reconciled.|
|Missing/corrupt journal or uncertain durability|Fail closed; no empty-ledger restart.|
|Operator recovery|Inspect uncertainty; reconcile or explicitly reset to a new epoch acknowledging possible old remote work.|

Journal writes require file **and directory** durability before granting. Existing
`atomic_write` warns rather than fails on parent-directory fsync failure and is
not sufficient unchanged for this safety contract. Likewise the general UDS
binder unlinks paths and must not be used before singleton ownership is secured.
Unknown completion requires visible quarantine, not an invisible leaked counter.
An explicit new-epoch reset does not promise a cap against old remote work.

## Private protocol and replay fencing

Use versioned bounded length-prefixed JSON through `quecto-line-io`, separately
from the agent-control protocol/socket. Operations are scoped acquire, cancel,
feedback, complete, child-scope registration and status/reconciliation. Service
administration/reset is owner-only and not exposed through child capabilities.
Every operation carries authority epoch, authenticated scope, request sequence
and grant identity where applicable. Deadline bounds apply to framing, including
oversized-declaration draining; malformed input never causes unbounded buffering.
Unknown required capability/version is an explicit error before inference.

Acquire sequences are serialized per scope; durable high-water marks fence old
requests after bounded tombstone eviction. A duplicate live request reconciles its
existing grant; a conflicting payload is rejected. Replay at/below high-water
cannot recreate completed acquisition. Duplicate completion cannot decrement any
other grant, even after eviction. Retiring a scope revokes its capability for the
epoch; scope identity is never recycled. Old-epoch operations cannot complete or
recreate current work. Bound active queues/tombstones while retaining replay
fences; never solve memory pressure by discarding those fences.

## Container transport contract

Select a dedicated admission UDS visible through a private mount for official
same-host Docker/Podman environments. Create validates that the mount is present;
join verifies the same authority epoch/reachability rather than remounting or
creating a new authority. Nested launches inherit the endpoint and authority-issued
scope. A private mount exposes only admission operations, not agent-control or
provider credential stores. Mount parent directories, not a replaceable socket
inode, so authority restart reconnect remains possible after reconciliation.

For runtimes without direct path visibility, require an explicit **reverse**
stdio bridge capability: child-side admission client opens a runtime-owned bridge
that reaches the host authority. Existing parent-to-child `socket_proxy` is not
such a capability. Runtime-specific bridge invocation/mounts belong to scripts;
application receives only typed reachability/capability results. Unsupported
create/join/nested paths fail enabled launch before inference, without local
fallback. No multi-host transport is claimed.

P0's direct/proxy subprocess fixture exercises real reverse bytes through one
host listener with restricted environment and frame rejection. It is not official
Docker create/join wiring, production authentication or a working budget. P3 must
prove real supported-runtime connectivity, restart/reconnect, queue cancellation
and SIGKILL uncertainty before enabling this mode.

## Observation and rollout

Expose orthogonal waiting/running/uncertain state, group, reason and bounded wait
duration; aggregate queued/active/uncertain counts and freshness timestamps. Do
not promise exact queue positions. UDS/TUI/subagent inspection must not call a
queued agent stalled or terminal; socket readiness stays independent. Redact
capabilities, credentials and raw provider account identifiers.

P1 delivers pure policy/contracts; P2 actual-attempt and typed-feedback wiring;
P3 durable authority, trusted inheritance and real container evidence; P4 UI,
operational commands and measured synthetic-workload comparison. Only explicit
configuration activates enforcement after required cross-process/runtime proof.
Rollback preserves uncertainty until acknowledged recovery. Adaptive control,
token accounting, discovered limits and multi-host coordination remain deferred.

## Consequences and validation

Safety may reduce availability indefinitely for an uncertain group. Operators
receive explicit reconciliation/reset tools rather than automatic unsafe reclaim.
The transport cancellation work is required; an outer receiver-only decorator is
insufficient. No fresh retry owner, delegation cap or broad generic framework.

P0 fixtures: `tests/inference_admission_characterization.rs` characterizes delayed
terminal delivery, >64-event slow consumption, HTTP rejection and receiver drop;
`tests/inference_admission_transport.rs` proves real local direct/nested bridge
connectivity and negative framing/capability cases. Existing retry, refresh,
AbortOnDrop and launch contracts remain regression seams. Full runtime policy,
crash, reload, observation and real Docker/Podman tests are P1–P4 gates, not claims
made by these fixtures.

Conforms to accepted ADR-0001/0006/0007/0008/0011/0021/0022/0024. Proposed
ADR-0012/0015/0016/0017/0019 guide boundaries but are not prerequisite migrations.
Rejected alternatives: process-local budget, count live agents, lease-expiry
reclamation without acknowledgement, use full control socket as admission proxy,
credential-derived identity, adaptive/token/distributed expansion in MVP.

### P2 leaf integration status

Leaf adapters accept an explicit inward `AttemptAdmission` capability, bound before
router/refresh type erasure. The default factory remains disabled. The optional
runtime ingress rejects changed policy or aliases rather than resetting live
charges; the supplying authority is responsible for issuing the trusted scope and
same quota-group capability. `AttemptPermit` receipt/deadline/completion operations
separate feedback from transport teardown. Dropping a receiver is not release;
local transport destruction precedes the consuming completion acknowledgement.

The domain accepts idempotent, sequence-fenced nonterminal feedback. No-hint
escalation is group-owned and uses explicit `fallback_base_ms`, the configured
maximum, and authority-supplied jitter. Successful completion resets the streak,
not established cooldown. HTTP advice uses a captured wall/monotonic pair and
checked arithmetic; excessive hints make the group unavailable. HTTP-date parsing
accepts the three standard forms through `httpdate`; its legacy RFC850 year mapping
is fixed1970–2069, not a moving fifty-year interpretation.

P2 tests use in-process capabilities and loopback transports. They do not establish
P3 broker durability, container/process authority or safe production activation.
Those remain the prerequisite for enabling shared-host admission.

P2 enabled transport additionally requires an explicit `SingleAttemptClient`,
built from the caller's configured reqwest builder with automatic redirects and
protocol retries disabled. One `send` must not hide a redirected/replayed POST.
An already-built arbitrary client cannot prove these policies or recover its
proxy/TLS/timeouts; callers must supply the original configured builder recipe,
not silently substitute defaults. Disabled inference keeps its original client.
Redirect responses are rejected; manually following redirects would require a new
admitted attempt and is not implemented in this phase.

### P3 authority and descendant status

The same-user authority exists as `quecto admission-broker run`: an owner-only
directory (`client/` for the socket a container may see; journal, lock and admin
socket outside it), an exclusive `flock` taken before any socket is bound, a
ledger written with file and directory fsync before every grant and before every
completion acknowledgement, and a versioned framed JSON protocol with one bound
connection per scope. Roots register on the private socket; descendants are
registered by their parent and receive endpoint/epoch/scope/capability through a
0600 sidecar (`--admission-context`), bound before socket readiness. Abandonment
(connection loss, SIGKILL) keeps active attempts as uncertain occupancy and
quarantines the group until the same capability completes them or an operator
reset starts a successor epoch. Authority restart restores the epoch and orphans
outstanding work; a corrupt ledger fails closed. Docker/Podman adapters mount the
client directory by path and report `shared-directory-v1`; an enabled parent
refuses containers without it.

Real multi-process evidence (`tests/inference_admission_processes.rs`) covers two
independent roots plus a third bounded at C=2 against a fake HTTP provider, a
control burst without admission, descendant wait/forged-capability refusal,
SIGKILL of a client and of the authority.

### P4 observation, presentation and rollout status

Each process records its own attempts' admission transitions behind an
observation decorator (`AdmissionRecorder`/`ObservedAdmission`, read through the
`AdmissionObservation` port) as a bounded, fresh view: exact counts, a sample of
at most 64 live attempts, per-group cooldown and bounded refusal reasons. Over
the socket the view rides beside the execution phase as `get_state.admission`;
the progress verdict is `waiting` (count, group, longest wait, cause) while any
attempt is queued, so a queued agent is never idle, quiet or stalled; a changed
revision advances the single `generation` cursor once per observation and every
transition is pushed in order as `admission_state_changed`. The parent's monitor
forwards a descendant's view re-stamped with its identity. The TUI paints the
label on the footer and working spinner (master) and on the panel row
(descendants) without touching any lifecycle state. `abort` while waiting cancels
the wait at the authority. Evidence and the measured fake-workload comparison
(four roots, burst versus C=2) are retained in commit `77bd895c`
(`notes/1679-p4-3-verification.md`); the
activation/rollback/quarantine runbook is in `docs/inference-admission.md`.
Adaptive concurrency, token-aware pacing and multi-host authority stay deferred.
