# P1 semantic matrix

Explicit monotonic u64 millisecond time is supplied by caller; transport authentication,
header parsing, durability, UI and provider wiring are P2–P4, not simulated here.
All observations below are typed public service results/status using scope+sequence.

| Invariant | Dimensions / representative cases | Expected observation | Evidence |
|---|---|---|---|
| AC1 capacity | C=1, C>1, active finish, active cancellation/deadline, two groups | never exceed C; only confirmed finish frees active; groups independent | port contracts |
| AC2 fairness | roots with one/many agents; FIFO; absent/blocked peers; interactive/background saturated | RR roots then agents, FIFO eligible heads; adding descendants doesn't add root turns | deterministic grant trace |
| AC2 reserve/arbitration | R=0, R=C-1, C=1; mixed demand; exhausted background headroom | background active <= C-R; eligible shared slots 3 interactive:1 background, work-conserving fallback; reserve grants do not spend shared turns | trace contracts |
| AC3 validation | zero C/interval/deadline/cap; R>=C; empty/unknown/duplicate alias/group mappings; overflow | explicit error, no partial config | table tests |
| AC3 identity | trusted root registration, child/grandchild, child requested interactive, unknown scope, replay/changed payload | root lineage inherited, child cannot elevate, opaque group lookup required; invalid request rejected | port contracts |
| AC3 pacing | exact interval edge, release/error, long cooldown/shorter feedback, overflow/time regression | next start not early; no refunds; max cooldown; invalid time/overflow fail closed | fake-time contracts |
| AC6 queue | cap-1/cap/cap+1, deadline-1/deadline, cancelled queued request, independent groups | explicit full/timeout/cancelled, never dispatch expired/cancelled | contracts |
| AC6 lifecycle | enqueue duplicate queued/active/terminal, payload conflict, out-of-order sequence, tombstone eviction, unknown finish | reconcile same outcome; no second grant/release; bounded tombstones + high-water fencing | contracts |
| AC6 bounded memory | scope count, groups, queue, active, tombstone cap, retire scope with pending/active, epoch mismatch | explicit limits; no recycled scopes; old epoch cannot affect current state | contracts |

High-risk interactions: reserve exhaustion + 3:1 arbitration; wide root + per-agent FIFO;
queue timeout + duplicate acquire; cancellation + grant + duplicate finish; evicted
terminal + stale sequence; active attempt deadline + new request (capacity retained);
child/grandchild + interactive request; shorter cooldown + pacing; time overflow +
mutation atomicity. Each maps to deterministic public-port contracts. Configuration
is immutable for a service lifetime. Raw protocol auth and journal replay are deferred;
trusted registration is an application ingress port, not a client assertion.

## Accepted counterexamples (matrix extension)
| Invariant | Dimensions / representative cases | Expected observation | Evidence |
|---|---|---|---|
| AC6 unknown completion | unknown future sequence vs historical evicted terminal | all configured groups quarantined for unknown future completion (no group identity available), occupancy retained, no grant; historical duplicate harmless | port contract |
| AC3/6 ownership | lookup under B for A sequence; valid scope wrong sequence; old epoch (caller authentication deferred P3) | scoped operation rejects without changing other requests or cooldown; no separate caller-selectable grant ID | port contract |
| AC3 cooldown bounds | zero, max, max+1, deadline overflow, group B eligible | zero doesn't shorten; max accepted; excess/overflow makes A unavailable, B progresses | table/trace contracts |
| AC6 feedback replay | completion feedback duplicate after later success; conflicting terminal payload | no repeated feedback mutation; terminal outcomes stable, conflict rejected | port contract |

Test-review precision: duplicate raw mapping entries are a P2/P3 config-parser
boundary (the typed P1 map cannot represent duplicates). Cooldown fallback
base/jitter/header normalization belongs to explicit P2 checklist; P1 validates
already typed delay maxima, merges deadlines and fails closed on overflow.
Drained-root/agent rejoining rotation now has a dedicated grant trace.

Reserve allocation refinement: interactive occupancy fills reserve first; remaining
interactive and background occupancy is shared. This preserves total C and B<=C-R
while allowing background to use free shared headroom behind a long interactive.
Only interactive grants above R active interactive transports spend the shared
streak. A new `admission_dispatcher` RED→GREEN trace pins bounded grant progress.

Final counterexample: C2/R1 short interactive completion before every paced start
must not starve eligible background. A separate contested reserve-pacing streak
forces a background start after three such opportunities; actual shared grants
reset that streak and retain original 3:1 shared arbitration. Reserve grants still
do not alter shared-slot streak. New exact trace [I,I,I,B] failed before fix and
passed afterward (/tmp/p1-short-red, /tmp/p1-fix); full contracts 99 passed.
