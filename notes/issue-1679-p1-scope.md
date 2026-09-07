# Issue 1679 P1 scope lock

Implement canonical P1 only: deterministic domain policy/types and application
service/port with in-memory public contracts (AC1–3, AC6 phase-local). P0 at
12508c67 and ADR-0026 are the baseline. Explicit clock inputs; no I/O or new
framework. Trusted registered root/child identity, group configuration validation,
hierarchical scheduling, reserve/3:1 arbitration, pacing, bounded queues,
cancellation/deadlines, cooldown and replay-fenced idempotency are in scope.

Expected surfaces: domain/inference_admission.rs, application/inference_admission.rs,
application/ports.rs, module exports, contracts and phase-local tests/docs.
Verification: deterministic contracts and mutation/RED evidence, architecture,
existing contracts/characterization, formatting/clippy and repository hooks.

Deferred: P2 provider/alias/reload integration and header normalization; P3 IPC,
authentication, durability/quarantine recovery and container/process integration;
P4 presentation and rollout. No production call path is enabled or changed, no
shared host bound or completed full-issue AC claim, no PR1674 changes.
