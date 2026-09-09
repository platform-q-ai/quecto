# #1679 P4 scope lock

Base: master after P3 (#1697, e2dd629d). Canonical: issue #1679 P4 "Presentation,
operational proof and rollout" (AC2, AC5, AC7–8), execution comment 5575553521.
Delivered as three stacked PRs so each is reviewable and gate-green on its own:

1. **Observation state** (this branch, `issue-1679-p4-observation`): a domain
   admission observation vocabulary (waiting / admitted / cooldown / uncertain
   with group, reason, elapsed wait) and a bounded per-process aggregate with
   freshness; an application read port (`AdmissionObservation`); an
   infrastructure decorator over every admission gate that records transitions
   around `acquire`/`finish`/feedback without touching the attempt transport or
   the process lifecycle enum; `ProcessAdmission::observation()`. Contracts,
   unit and BDD proof over a real in-process authority. No UDS/TUI change.
2. **UDS emission and projection**: an orthogonal `admission` field on the
   execution snapshot / `get_state` (never a lifecycle state), progress event,
   busy-path inspection and abort while waiting, no false stall/quiet verdict,
   freshness and delta bounds per ADR-0024, protocol docs. After #1698 merges
   (it reshapes the same UDS files).
3. **TUI projection, verification matrix and rollout**: quecto-tui waiting/
   cooldown rendering with render-harness and `@tui` BDD proof; the full AC1–7
   BDD matrix; a measured fake-workload burst-vs-admitted comparison; activation,
   rollback and quarantine runbook; ADR-0026 P4 status; issue AC boxes.

Not in P4: adaptive concurrency, token-aware pacing, multi-host authority, a
`cargo-mutants` sweep (tracked as follow-up). No paid provider traffic.
