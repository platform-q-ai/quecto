# ADR-0003 — UDS `register_provider` for Dynamic Model/Provider Registration

**Status:** Proposed (deferred; not in v1).

**Implementation status:** Not implemented.

## Context

pi exposes a programmatic `registerProvider()` so an in-process extension can
register providers/models at runtime, including dynamic discovery from gateway
`/models` endpoints. quecto has no in-process extension surface; its live
extension surface is the **UDS tool registry** (`register_tools`).

The question is whether to add a parallel UDS `register_provider` verb so
external processes can inject providers/models into the live `ModelRegistry`,
mirroring how `register_tools` injects tools.

## Decision

**Defer.** Ship declarative `models.json` plus the ADR-0002 reload path as the v1
mechanism for adding models/providers. Do **not** add a UDS `register_provider`
verb in v1. Gate promotion on a concrete dynamic-discovery consumer.

## Current Implementation

- No UDS `register_provider` command exists.
- Dynamic/provider model additions are handled through config/`models.json` and
  provider reload where implemented.

## Consequences

- *Positive:* keeps the kernel/UDS protocol small and avoids a second registration
  path.
- *Interim bridge:* a sidecar can regenerate `models.json`; ADR-0002 reload picks
  it up next turn.
- *Negative / cost:* dynamic registration is less ergonomic than a native verb,
  and file-backed changes land on reload rather than immediate push.
- *Promotion path:* add `register_provider` only when a real consumer needs live,
  push-based, file-less registration.

## Alternatives Considered

- **A. Build `register_provider` in v1.** Rejected as speculative protocol surface.
- **B. Never add it.** Rejected because dynamic discovery is plausible.
- **C. Sidecar rewrites `models.json`.** Adopted as the deferral escape hatch.

## Related

- [Kernel boundary](../architecture/kernel-boundary.md)
- [Models runtime extensible PRD](../prd/prd-models-runtime-extensible.md)
- [UDS protocol](../uds-protocol.md)
