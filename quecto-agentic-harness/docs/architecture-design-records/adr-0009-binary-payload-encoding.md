# ADR-0009 — Binary Payload Encoding for the UDS Protocol

**Status:** Rejected — superseded by ADR-0011 (Stay JSON on the Wire).

**Implementation status:** Not implemented (and will not be — see ADR-0011).

## Context

With ADR-0008 (length-prefixed framing, bounded events) the socket protocol's
structural problems are addressed while payloads stay UTF-8 JSON. This ADR
covers the follow-on question of whether payloads should become binary
(MessagePack, CBOR, bincode, protobuf) for size and serialization speed.

Facts that constrain the choice:

- **Format constraint.** The protocol types are serde types using internally
  tagged enums (`#[serde(tag = "type")]`). Self-describing formats
  (MessagePack via `rmp-serde`, CBOR via `ciborium`) support this directly;
  **bincode does not** (no `deserialize_any`), and **protobuf** would require
  redefining every protocol type as a schema plus codegen. "Binary" therefore
  realistically means MessagePack or CBOR unless a full protocol-type rewrite
  is separately justified.
- **The expensive part is JSON-*shaped* code, not the codec swap.** Four
  subsystems assume JSON text on the wire path: the cap/shrink machinery
  (`infrastructure/line_cap.rs`, `serde_json::Value` trees and JSON byte
  accounting), the serialize-once `RawValue` byte-budgeting in
  `build_get_messages_line` (#994 — `RawValue` has no MessagePack/CBOR
  equivalent; splicing pre-encoded elements into an array becomes bespoke
  binary construction), the pre-parse string sniffing on the dispatch hot
  path (`is_cancel_command`/abort/steer detection), and the sub-agent
  monitor's parse-modify-forward re-stamping.
- **Sequencing with ADR-0008.** ADR-0008 removes or restructures the shrink
  machinery and the monolithic snapshot path; implemented after it, this ADR
  reduces to near the mechanical codec swap plus the hot-path sniff and
  re-stamp updates. Implemented before or alongside it, the migration pays
  the full cost of porting code ADR-0008 then deletes.
- **Boundaries.** Session files and the audit log are JSONL on disk and stay
  JSON regardless (same serde types, different sink). External extension
  processes speak the socket protocol; a payload-encoding change either
  migrates them or adds a transcoding bridge at that edge. Production code
  serializes protocol types directly to the wire encoding — there is no
  JSON-then-transcode step on the production path (test authoring is a
  separate concern, covered by ADR-0010).
- **Measurement.** Serialization CPU and frame size have not to date been
  observed as bottlenecks; the #1047 incident was framing and content growth
  (ADR-0008's territory), not encoding.

## Decision

Adopt binary payload encoding **when both preconditions hold**:

1. ADR-0008 is implemented, so the JSON-shaped subsystems that dominate the
   migration cost no longer exist; and
2. profiling of a real workload shows socket serialization/deserialization or
   frame size as a material cost.

The selected encoding is **MessagePack (`rmp-serde`) or CBOR (`ciborium`)** —
self-describing, serde-compatible with the existing tagged types. bincode and
protobuf are not candidates, for the format reasons above. The migration
reuses ADR-0008's protocol-version negotiation for the wire break, keeps disk
formats JSON, and ships a tap/decode helper to preserve socket debuggability.
Until the preconditions hold, payloads remain JSON.

## Consequences

- The decision criteria are recorded: a future proposal starts from the
  format constraint, the (post-ADR-0008: reduced) JSON-shaped subsystems, and
  a profiling bar, instead of re-deriving the analysis.
- Debuggability (`socat` + eyeballs, BDD steps asserting on JSON fields) is
  preserved while payloads remain JSON; ADR-0010 keeps the test corpus
  encoding-agnostic so a later switch does not rewrite it.
- Sequencing after ADR-0008 avoids porting code that ADR-0008 deletes.

## Alternatives considered

- **Adopt MessagePack/CBOR alongside ADR-0008.** Not chosen: it front-loads
  the migration into exactly the code ADR-0008 deletes, for a benefit not yet
  measured.
- **bincode / protobuf.** Not viable on format grounds (internally tagged
  serde enums; schema/codegen rewrite) independent of timing.
- **Rule out binary encoding permanently.** Not chosen: if paged, bounded
  events still prove hot under profiling, a self-describing binary codec is a
  contained, mechanical change post-ADR-0008.
