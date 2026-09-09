# #1679 P3 scope lock

Base: 6a383f30 (origin/master, after #1688). Branch `issue-1679-p3`, worktree
`quecto-wt-1679`. Canonical scope: issue #1679 P3 "Authority and process/container
scope" (AC1, AC4, AC6–7 phase-local), execution comment 5575553521.

Deliver the shared same-user authority and its descendants:

- Domain: uncertainty/quarantine transitions (client abandonment, verified
  termination, orphaned restart occupancy, operator epoch reset), grant withdrawal
  on failed durability, dispatch wake computation and a durable ledger value.
- Application: authenticated authority service over the P1 policy (capability-bound
  root/child lineage, journal-before-grant, disconnect handling, reconnect
  idempotency, operator status/reset) behind narrow ports; typed commands/replies.
- Infrastructure: private owner-only directory, singleton lock preceding socket
  bind, strict file+directory durable journal, framed UDS server (`quecto-line-io`),
  remote `AttemptAdmission` client, launch-argument/container-script capability
  propagation (`--admission-context` sidecar, `QUECTO_ADMISSION_DIR` mount).
- Interface: `admission-broker` subcommand (run/status/reset), agent-side
  negotiation before socket readiness, admission-enabled provider composition.
- Verification: contracts for every new port, real multi-process fake-HTTP proof
  (two roots + local grandchild share C), SIGKILL/disconnect/restart/journal-failure
  tests, real Docker/Podman container child evidence, tagged BDD.

Not in this PR: P4 observation/TUI projection, fairness end-to-end matrices, burst
comparison, rollout/runbook docs beyond what P3 activation requires; adaptive or
token-aware policy; multi-host authority. No paid provider traffic. Normal runtime
stays disabled unless `admission` is explicitly configured. No merge without user
authorization.
