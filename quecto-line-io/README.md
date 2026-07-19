# quecto-line-io

Shared bounded reader/writer for Quecto's UDS JSON protocol.

Used by `quecto-agentic-harness`, `quecto-tui`, and `quecto-api` so every
emitter and consumer shares one payload cap and one framing implementation.

## Capabilities

- **Length-prefixed frames** (ADR-0008) and **legacy NDJSON lines** (dual-mode
  read during the deprecation window).
- **Hard payload cap:** `PROTOCOL_LINE_CAP_BYTES` = **8 MiB** (including the
  trailing `\n` on legacy lines). Readers refuse oversized input without
  unbounded buffering.
- Helpers: `read_bounded_line` / `read_bounded_line_into`,
  `read_frame_or_legacy_line` / `read_frame_or_legacy_line_into`,
  `write_message` with `WireMode::{Frame, LegacyLine}`.

## Version

See `Cargo.toml` (`0.2.0` at time of writing). This crate is a workspace
library — not a shipped binary.

## See also

- [ADR-0008 — length-prefixed UDS framing](../quecto-agentic-harness/docs/architecture-design-records/adr-0008-length-prefixed-uds-framing-and-bounded-events.md)
- [UDS protocol reference](../quecto-agentic-harness/docs/uds-protocol.md)
