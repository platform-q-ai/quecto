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

## Malformed-stream contract

- Clean EOF before any frame prefix bytes is `Ok(None)`.
- EOF after a partial frame prefix, or inside a declared in-cap frame payload,
  is an `UnexpectedEof` I/O error.
- An over-cap frame declaration is rejected as oversized as soon as its prefix
  is known; a full oversized payload is discarded so a following frame can be
  read without resynchronizing.
- Legacy oversized lines recover only after a newline delimiter is consumed.
- Byte-preserving APIs (`*_into` and framed reads) leave invalid UTF-8 unchanged;
  `read_bounded_line` returns lossy `String` content for legacy convenience.
- Reusable-buffer APIs shrink previously large caller-owned buffers before the
  next read, so retained capacity stays bounded and does not scale with a prior
  large or malformed message.

Dependents should derive protocol caps from `PROTOCOL_LINE_CAP_BYTES` or
`PROTOCOL_FRAME_CAP_BYTES`, not from a separate numeric literal. Large message
recovery uses the existing ranged `get_message` protocol in consumer crates;
this crate does not introduce a new chunking format or cap value.

## Version

See `Cargo.toml` (`0.2.1` at time of writing). This crate is a workspace
library — not a shipped binary.

## See also

- [ADR-0008 — length-prefixed UDS framing](../quecto-agentic-harness/docs/architecture-design-records/adr-0008-length-prefixed-uds-framing-and-bounded-events.md)
- [UDS protocol reference](../quecto-agentic-harness/docs/uds-protocol.md)
