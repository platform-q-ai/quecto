# P1 test design

Public admission capability ports accept explicit monotonic milliseconds and trusted
scope handles. `AdmissionService` is the in-memory implementation; policy is domain
owned and performs no I/O. Contract tests use identical entrypoints intended for
future authority composition. No runtime adapter is installed.

BDD-style Given/When/Then Rust contracts cover each matrix row, using small explicit
configurations and exact grant traces. Table-driven invalid configurations pin both
sides. A deterministic trace covers uneven fanout and FIFO; separate traces pin 3:1
and non-borrowable reserve. Lifecycle traces check queued duplicates/conflicts,
active cancellation and elapsed deadline retaining capacity, terminal eviction with
replay fencing, queue cap/expiration, independent groups, cooldown and pacing.
Every assertion receives individual RED/mutation evidence, then restored GREEN.
No sleeps, network, paid providers, ignored cases or production testing bypasses.

Proposed capability split: trusted scope registry, admission lifecycle, dispatcher.
Scope IDs are authority-issued monotonic epoch-scoped handles (bounded issued
scopes per epoch, no reuse); children inherit root and a background ceiling.
Acquire carries only scope/sequence/group, never caller-supplied priority/lineage.
Root interactive vs background is selected at trusted registration. A bounded
per-scope high-water mark rejects evicted replay; bounded tombstones reconcile
recent terminal results. Scope issuance fails explicitly at its lifetime bound.
Conflicting sequence payload is rejected even while live; completion requires the
exact request identity. Configuration maps opaque aliases to typed group IDs and
is immutable. Enqueue resolves alias via config, while scheduling consumes group
policy. All time-bearing calls reject regression before mutation.

Baseline 2026-09-07: `cargo test -p quecto-agentic-harness --test architecture
--quiet`: 46 passed. Hook installation succeeded; activation requires bash (tool
shell is /bin/sh), so git publication commands will explicitly source activation
inside bash. Both executable hooks and wrapper resolution verified.
Baseline public contracts: `cargo test -p quecto-agentic-harness --test contracts
--quiet`: 77 passed.
Baseline P0 fixtures: `cargo test -p quecto-agentic-harness --test
inference_admission_characterization --test inference_admission_transport --quiet`:
4 characterization and 2 transport tests passed.
Tagged `@inference-admission @done` BDD scenarios will additionally exercise the
public service for C=1 child progress with an idle parent, queue cancellation and
pacing. These are local policy claims, not transport/process guarantees. Existing
BDD world/module conventions apply; comprehensive combinatorics stay in contracts.
Initial test inventory: 12 contract scenarios in contracts/inference_admission.rs
and 3 tagged Gherkin scenarios. Initial RED command fails with E0432 for missing
application/domain/ports, recorded in /tmp/p1-red-compile.log; this is not claimed
as per-assertion RED evidence. Individual assertion mutations remain required once
the API compiles. BDD quality and status tags pass (existing unrelated warnings).
Final inventory includes the additional reserve recovery dispatcher and unissued/
retired registry boundary contracts. API now uses the three capability role names
AdmissionRegistry/AdmissionClient/AdmissionDispatcher rather than draft combined
InferenceAdmission. Constructor validation stays domain-owned. Contract module
naming follows the existing public-port architecture rule (no new exceptions).
