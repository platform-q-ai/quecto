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
