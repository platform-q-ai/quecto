# P1 implementation and limits

The domain owns explicit-time in-memory policy. Application exposes separate trusted
registry, client lifecycle and serialized dispatch roles through ports.rs; the
service delegates to policy without I/O. Immutable validated config maps non-secret
aliases to opaque groups; scope/sequence replay fences and bounded terminal cache
retain idempotency without unlimited request history. Scope issuance has an explicit
lifetime cap; retired IDs never recycle in an epoch. Queued retirement cancels;
active retirement refuses until confirmed completion.

Queued cancel/expiration is terminal. Active cancellation/deadline only requests
cancellation and retains occupancy. Only confirmed completion frees a slot and
applies typed feedback exactly once. Unknown future completion quarantines all
reachable groups because the compact input cannot safely identify a group;
historical replay is harmless. P3 may narrow quarantine with authenticated grant
metadata and durable reconciliation. Scope arguments are trusted ingress inputs,
not unforgeable credentials; P3 must authenticate and bind them.

No provider factory, transport, retry, launch or UI is wired. This is not host-wide
enforcement and does not complete full-issue AC1–8. P2 owns provider mapping ingress,
reload, throttle parsing and fallback/jitter. P3 owns authenticated capability/IPC,
durable journals, restart/outage, explicit reset and container reachability. P4
owns live status/presentation and rollout. Alias duplicate detection must occur
before construction of the typed map. Existing runtime remains unchanged.

Process deviation: initial compile RED proved missing API; full per-assertion RED
was not completed before implementation. Post-implementation individual assertion
mutations are being recorded explicitly, not mislabeled as pre-implementation RED.

Scheduling detail: reserve is accounted against active interactive occupancy first;
background is capped by C-R, and only interactive occupancy beyond R consumes a
shared-class turn. This avoids reclassifying an outstanding reserved stream as a
shared stream when a shared attempt finishes. The 3:1 trace remains bounded in
eligible opportunities; long streams holding all C still prevent progress.

Memory bounds are configuration-relative: live requests <= sum(queue_capacity +
capacity) plus terminal_capacity; registered/replay scope state and per-group
rotation state <= max_scopes (lifetime issuance). No registration or terminal
history grows beyond those explicit caps. Static config storage is immutable and
finite, constructed by trusted authority ingress. P2 parser must reject duplicate
raw aliases before converting into the typed BTreeMap.

Integration contract for follow-up: call `complete` only after transport termination,
never on receiver creation/drop alone; cancellation-required is advisory until that
confirmation. Enqueue and all dispatch/status transitions require a single monotonic
authority clock. The mutable service is serialized in process; future broker owns
serialization, epoch persistence, scoped authorization and journal-before-grant.

A separate capped reserve-pacing streak prevents short reserved interactive starts
from consuming every pacing opportunity while background is eligible. It resets
when a shared grant occurs; it does not alter the shared-slot 3:1 streak. This is
required to make reserve and group-wide pacing compose without background starvation.
