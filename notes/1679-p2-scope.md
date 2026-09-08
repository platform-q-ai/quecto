# #1679 P2 scope lock

Base: c3b597e0873a204b6647f11e363cbccb775610e5 (P1, PR #1683), current origin/master.
Canonical scope: issue #1679 P2, AC3–5 provider attempt boundaries only.

Integrate leaf outbound attempts below existing routing/retry/refresh, preserving all
three provider surfaces and stream ownership/backpressure. Reuse inward-owned
admission capability and shared policy; normalize HTTP throttle feedback without
changing public errors or introducing a retry owner. Explicit opaque alias mapping
must survive provider rebuilds. Disabled remains compatible.

Expected surfaces: application admission ports, infrastructure provider adapters,
provider factory/runtime composition, domain feedback types if needed, public-port
contracts and fake transport BDD/tests. ADR0026 and ADR0007 govern ownership and
per-assertion RED/mutation evidence. Architecture rules remain mandatory.

Verification: targeted attempt/fallback/refresh/receiver-lifetime and cancellation
contracts, typed numeric/ms/date/invalid/excessive throttle tests, alias/reload tests,
existing retry and agent-loop regressions, tagged inference-admission BDD,
architecture/contracts, strict clippy and repository push gates.

Not in this PR: P3 broker/IPC/durable recovery/container wiring or P4 observation,
rollout and full shared-host claims. No paid provider calls, no new retry budget,
no top-level receiver wrapper masquerading as transport ownership; no production
activation before P3 evidence. No merge without user authorization.

Resumed execution remains scoped P2. New tests include retry/refresh loopback,
leaf ownership, receipt distribution, alias runtime retention and parser/domain
contracts only. No broker IPC/process persistence/UI rollout or production
activation added. Future asynchronous phase-local adapter must expose capability
inward, not claim P3 host authority.
