# Issue #1679 — P0 scope lock

Canonical requirements: https://github.com/platform-q-ai/quecto/issues/1679
Canonical execution plan: https://github.com/platform-q-ai/quecto/issues/1679#issuecomment-5575553521

This first PR implements **P0: contract and fixtures** only (AC8 planning/documentary foundation and characterization of AC1–7). The complete authorized issue requires P1–P4 afterward; this PR neither activates admission nor closes the issue. Phase-separated delivery follows the feature workflow; no unplanned cross-phase production implementation.

Deliverables: ADR selecting same-user shared authority/attempt/config/crash/protocol contracts and explicit supported private container reverse transport; provider stream/retry/readiness/cancellation characterization fixtures; prototype direct/proxy transport evidence using real separate processes and fake scripts. Current branch starts at eb3c49d86032dbb8692e9cf81ccfc8a43eb43e02.

Ownership: domain pure scheduling types/policy, application lifecycle/narrow ports, infrastructure HTTP/IPC/storage/scripts, interface composition/observation. P0 does not add runtime scheduling code or speculative public ports. Accepted ADR0001/0006/0007/0008/0011/0021/0022/0024 apply; proposed state-machine/typed-ID ADRs are guidance, not prerequisites.

Expected surfaces: new ADR and index, container transport contract docs, provider characterization unit tests, existing launch/contract tests and a test-only transport prototype. No C1/PR1674 changes.

Verification: record per-new-assertion RED via deliberate mutation, restore fixture, GREEN exact provider/application retry tests, new stream characterization, architecture/contracts and transport prototype tests. Protocol prototype is not production wiring or a claim of real Docker acceptance. Actual Docker/Podman P3/P4 evidence remains mandatory; no runtime CLI is available in this container.

Deferred: P1 policy/contracts; P2 actual-attempt integration; P3 broker and descendants; P4 observation/operational verification/rollout. Adaptive/token/discovered quota/multi-host work remains outside MVP. All issue AC boxes remain unchecked. No automatic uncertain capacity reclamation, remote computation cap promise or in-process-only shared-support claim.
