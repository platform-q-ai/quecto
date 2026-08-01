# ADR-0002 — Reload Trigger for Startup-Loaded Surfaces

**Status:** Accepted.

**Implementation status:** Partially implemented.

## Context

Startup-loaded surfaces fail the autonomy contract because an agent can modify
files but cannot use the changes until a restart. The affected surfaces are
knowledge sources, models/providers, and workflow template sets.

The open question is the **trigger**: when does reload run, and how do we avoid
paying for it every turn? The reference design (pi) refreshes its model registry
on operations that consume it, which is sufficient for human interaction but does
not guarantee unattended self-extension.

## Decision

Adopt a **hybrid trigger** behind a **cheap change-detection gate**, implemented
once as a shared mechanism:

1. **Top-of-turn reload.** At the start of each turn, rebuild any changed source
   before the turn proceeds.
2. **On-consume reload.** Operations that read a surface also trigger reload.
3. **Change-detection gate.** Use `mtime`/length and content hash checks so
   steady-state cost is effectively a `stat`.
4. **Fail-safe.** A malformed source keeps the last-good state and logs a warning.

The mechanism is one `RuntimeReload`-style component with consumers for knowledge
folder sources, models/providers, and workflow template sets.

## Current Implementation

- The shared `RuntimeReload` gate exists.
- Provider/model reload is wired for config and `models.json` and runs before UDS
  prompt/set-model consumption.
- Knowledge folder reload is not implemented.
- Workflow-template set reload is not implemented.

## Push vs Pull

Tools are deliberately not a consumer. ADR-0002 specifies a **pull** trigger:
the kernel re-reads file-backed sources on a turn boundary, gated by mtime/hash.
Tools are **push**: an external process calls UDS `register_tools` and mutates the
live `ToolRegistry` directly. the removed pre-customer compatibility no-op did not provide
the ADR-0002 trigger.

## Consequences

- *Positive:* once all consumers are wired, the autonomy contract holds for every
  file-backed surface.
- *Negative / cost:* every startup-loaded surface must route through the shared
  path, and edits made mid-turn are only guaranteed at the next turn boundary.
- *Security note:* re-reading config each turn means a process that can rewrite
  config can change endpoints/keys between turns; this has the same trust level
  as writing config at all.

## Alternatives Considered

- **A. On-consume only.** Rejected because autonomy becomes a side-effect.
- **B. Explicit `reload` signal only.** Rejected because it puts a human or
  supervisor back in the loop.
- **C. Filesystem watcher.** Rejected because it adds portability and edge-case
  complexity for little benefit over `stat` at natural checkpoints.
- **D. Unconditional rebuild every turn.** Rejected because it adds needless
  per-turn latency.

## Related

- [Kernel boundary](../architecture/kernel-boundary.md)
- [Models runtime extensible PRD](../prd/prd-models-runtime-extensible.md)
- Implementation: `src/infrastructure/reload.rs`, `src/interface/cli/provider_reload.rs`
